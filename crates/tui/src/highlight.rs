//! Syntax highlighting for fenced code blocks in assistant text, via
//! `syntect`'s bundled syntax/theme dumps (see `ui::split_code_fences` for
//! where the fence is actually found and stripped). Runs entirely offline —
//! no network, no user-supplied grammar/theme files.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, Style as SynStyle, Theme as SynTheme, ThemeSet};

use crate::palette::Theme;
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

fn syntax_set() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

/// The `base16-ocean` pair, both bundled with syntect — matched siblings,
/// so a code block's token colors keep the same relationships in either app
/// theme instead of two unrelated schemes trading places. This used to be
/// pinned to the dark half regardless, "in the absence of any way to detect
/// the terminal's actual background"; `Theme` is that way — the developer
/// states it outright in `tui.yaml` (see `palette::Theme::from_config`) —
/// so the block no longer has to sit on a fixed-dark surface in a light
/// session to keep dark-tuned syntax colors legible.
fn theme(theme: Theme) -> &'static SynTheme {
    static DARK: OnceLock<SynTheme> = OnceLock::new();
    static LIGHT: OnceLock<SynTheme> = OnceLock::new();
    let load = |name: &str| ThemeSet::load_defaults().themes[name].clone();
    match theme {
        Theme::Dark => DARK.get_or_init(|| load("base16-ocean.dark")),
        Theme::Light => LIGHT.get_or_init(|| load("base16-ocean.light")),
    }
}

/// Memo for [`highlight_lines`], keyed on exactly its three arguments.
///
/// Highlighting is a pure function of `(lang, body, theme)`, and the
/// transcript asks for the same answer over and over: while a reply streams
/// in, the entry it is arriving into is re-rendered once per frame (see
/// `ui::Transcript`), so every code fence that reply has *already closed*
/// gets re-highlighted on every frame for the rest of the turn — measured at
/// 0.28ms per fence per frame, which was half of what a streaming frame
/// still cost after the transcript itself stopped being rebuilt.
///
/// A closed fence never changes, so the second highlight of it is pure
/// waste. This is a memo rather than a cache threaded through `Ctx` because
/// the thing being memoised is a free function with no other state: keying
/// on its own arguments cannot disagree with what it would have computed.
///
/// Bounded by clearing rather than by eviction order. The steady state is
/// tiny — only the entry currently being re-rendered asks — and the one
/// case that touches every fence in the session at once (a resize, which
/// rebuilds everything) is rare enough that falling back to a cold memo
/// costs a frame, not a session.
type Highlighted = Vec<Vec<Span<'static>>>;
static MEMO: OnceLock<Mutex<HashMap<(String, String, Theme), Highlighted>>> = OnceLock::new();

/// How many distinct fences the memo holds before it is emptied. Comfortably
/// more than any one reply has, which is the working set that matters.
const MEMO_CAP: usize = 64;

/// Highlights `body` (a fenced code block's content, `lang` from the
/// opening fence, e.g. `bash` in ` ```bash `) into one `Vec<Span>` per
/// line. Falls back to the plain-text syntax (still passes through the
/// theme's default foreground, just no per-token color) when `lang`
/// doesn't match a known syntax — better than refusing to render the code
/// at all over an unrecognized or missing language tag.
///
/// Memoised — see [`MEMO`]. A poisoned lock falls through to computing the
/// answer rather than panicking: this is a cache, and losing it must not
/// take the session down.
pub fn highlight_lines(lang: &str, body: &str, app_theme: Theme) -> Vec<Vec<Span<'static>>> {
    let memo = MEMO.get_or_init(|| Mutex::new(HashMap::new()));
    let key = (lang.to_string(), body.to_string(), app_theme);
    if let Ok(map) = memo.lock() {
        if let Some(hit) = map.get(&key) {
            return hit.clone();
        }
    }
    let out = highlight_uncached(lang, body, app_theme);
    if let Ok(mut map) = memo.lock() {
        if map.len() >= MEMO_CAP {
            map.clear();
        }
        map.insert(key, out.clone());
    }
    out
}

fn highlight_uncached(lang: &str, body: &str, app_theme: Theme) -> Vec<Vec<Span<'static>>> {
    let set = syntax_set();
    let syntax = set.find_syntax_by_token(lang).unwrap_or_else(|| set.find_syntax_plain_text());
    let mut highlighter = HighlightLines::new(syntax, theme(app_theme));

    LinesWithEndings::from(body)
        .map(|line| {
            let ranges = highlighter.highlight_line(line, set).unwrap_or_default();
            ranges.into_iter().map(|(style, text)| Span::styled(text.trim_end_matches('\n').to_string(), convert_style(style))).collect()
        })
        .collect()
}

fn convert_style(style: SynStyle) -> Style {
    let fg = style.foreground;
    let mut out = Style::default().fg(Color::Rgb(fg.r, fg.g, fg.b));
    if style.font_style.contains(FontStyle::BOLD) {
        out = out.add_modifier(Modifier::BOLD);
    }
    if style.font_style.contains(FontStyle::ITALIC) {
        out = out.add_modifier(Modifier::ITALIC);
    }
    if style.font_style.contains(FontStyle::UNDERLINE) {
        out = out.add_modifier(Modifier::UNDERLINED);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The memo must be invisible: a second call has to return exactly what
    /// the first did, and what an uncached call would.
    #[test]
    fn the_memo_returns_what_the_uncached_highlighter_would() {
        let code = "fn main() {\n    let x = 1;\n}\n";
        let want = highlight_uncached("rust", code, Theme::Dark);
        assert_eq!(highlight_lines("rust", code, Theme::Dark), want, "a cold memo must agree with the uncached path");
        assert_eq!(highlight_lines("rust", code, Theme::Dark), want, "and so must a warm one");
    }

    /// The three arguments are the whole key — none of them may be dropped
    /// from it, or a fence would be served another fence's colours.
    #[test]
    fn the_memo_keys_on_language_body_and_theme_together() {
        let code = "let x = 1;\n";
        let dark = highlight_lines("rust", code, Theme::Dark);
        let light = highlight_lines("rust", code, Theme::Light);
        assert_ne!(dark, light, "the same code in the two themes must not share a memo entry");
        assert_ne!(highlight_lines("rust", "let y = 2;\n", Theme::Dark), dark, "different bodies must not share one either");
    }

    /// A session with more distinct fences than the memo holds must keep
    /// answering correctly — the bound is a cache policy, not a limit on
    /// what can be rendered.
    #[test]
    fn highlighting_stays_correct_past_the_memos_capacity() {
        let sample = |i: usize| format!("let x{i} = {i};\n");
        for i in 0..MEMO_CAP * 3 {
            assert_eq!(highlight_lines("rust", &sample(i), Theme::Dark), highlight_uncached("rust", &sample(i), Theme::Dark), "entry {i} past the cap");
        }
        // And the very first one, long since evicted, is still right.
        assert_eq!(highlight_lines("rust", &sample(0), Theme::Dark), highlight_uncached("rust", &sample(0), Theme::Dark));
    }

    #[test]
    fn known_language_produces_more_than_one_color() {
        let lines = highlight_lines("rust", "fn main() {\n    let x = 1;\n}\n", Theme::Dark);
        let colors: std::collections::HashSet<Color> = lines.iter().flatten().map(|span| span.style.fg.unwrap()).collect();
        assert!(colors.len() > 1, "expected keyword/identifier/etc. to use different colors, got {colors:?}");
    }

    #[test]
    fn unknown_language_falls_back_to_plain_text_without_panicking() {
        let lines = highlight_lines("not-a-real-language", "hello\nworld\n", Theme::Dark);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].iter().map(|s| s.content.as_ref()).collect::<String>(), "hello");
    }

    #[test]
    fn output_preserves_line_count_and_strips_trailing_newlines_from_spans() {
        let lines = highlight_lines("python", "a = 1\nb = 2\n", Theme::Dark);
        assert_eq!(lines.len(), 2);
        for line in &lines {
            for span in line {
                assert!(!span.content.ends_with('\n'), "span content should not carry the line terminator");
            }
        }
    }
}

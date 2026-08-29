//! Syntax highlighting for fenced code blocks in assistant text, via
//! `syntect`'s bundled syntax/theme dumps (see `ui::split_code_fences` for
//! where the fence is actually found and stripped). Runs entirely offline —
//! no network, no user-supplied grammar/theme files.

use std::sync::OnceLock;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, Style as SynStyle, Theme, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

fn syntax_set() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

/// `base16-ocean.dark` is one of syntect's bundled themes — picked as a
/// reasonable default in the absence of any way to detect the terminal's
/// actual background, same open question as mjolnir-tui.md's deferred
/// accent color. Revisit together if/when that's settled.
fn theme() -> &'static Theme {
    static THEME: OnceLock<Theme> = OnceLock::new();
    THEME.get_or_init(|| ThemeSet::load_defaults().themes["base16-ocean.dark"].clone())
}

/// Highlights `body` (a fenced code block's content, `lang` from the
/// opening fence, e.g. `bash` in ` ```bash `) into one `Vec<Span>` per
/// line. Falls back to the plain-text syntax (still passes through the
/// theme's default foreground, just no per-token color) when `lang`
/// doesn't match a known syntax — better than refusing to render the code
/// at all over an unrecognized or missing language tag.
pub fn highlight_lines(lang: &str, body: &str) -> Vec<Vec<Span<'static>>> {
    let set = syntax_set();
    let syntax = set.find_syntax_by_token(lang).unwrap_or_else(|| set.find_syntax_plain_text());
    let mut highlighter = HighlightLines::new(syntax, theme());

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

    #[test]
    fn known_language_produces_more_than_one_color() {
        let lines = highlight_lines("rust", "fn main() {\n    let x = 1;\n}\n");
        let colors: std::collections::HashSet<Color> = lines.iter().flatten().map(|span| span.style.fg.unwrap()).collect();
        assert!(colors.len() > 1, "expected keyword/identifier/etc. to use different colors, got {colors:?}");
    }

    #[test]
    fn unknown_language_falls_back_to_plain_text_without_panicking() {
        let lines = highlight_lines("not-a-real-language", "hello\nworld\n");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].iter().map(|s| s.content.as_ref()).collect::<String>(), "hello");
    }

    #[test]
    fn output_preserves_line_count_and_strips_trailing_newlines_from_spans() {
        let lines = highlight_lines("python", "a = 1\nb = 2\n");
        assert_eq!(lines.len(), 2);
        for line in &lines {
            for span in line {
                assert!(!span.content.ends_with('\n'), "span content should not carry the line terminator");
            }
        }
    }
}

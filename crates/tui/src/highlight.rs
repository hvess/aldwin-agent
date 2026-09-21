//! Syntax highlighting for fenced code blocks in assistant text, via
//! `syntect`'s bundled syntax dumps (see `ui::split_code_fences` for where
//! the fence is actually found and stripped). Runs entirely offline — no
//! network, no user-supplied grammar/theme files.
//!
//! # The colours are the design system's, not syntect's
//!
//! syntect ships themes as well as grammars, and this module used to load
//! one: the `base16-ocean` pair, dark and light. That made a fenced block
//! the single region of the frame carrying hues the design system never
//! chose — every other cell reads a `--tui-*` role, and a code block read
//! base16.
//!
//! The design system closed that at its Turn 15 by defining five syntax
//! roles ([`Palette::syn_keyword`] and its four siblings) and two rules
//! about them: no syntax role may outrank the accent mark, and there are
//! exactly five — everything else in a block stays [`Palette::code`] and a
//! comment drops to [`Palette::dim`].
//!
//! So syntect is kept for what it is good at, parsing, and the theme is
//! *built* from the palette rather than loaded ([`theme`]). A `Theme` is
//! only a default style plus a list of scope-selector → style rules, which
//! is exactly the mapping the five roles need; building it means the scope
//! matcher, the caching highlighter and the specificity scoring all still
//! come from syntect, while no colour can enter a frame that
//! `palette.rs` did not put there. Pinned by
//! `every_highlighted_colour_is_one_of_the_seven_roles`.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use syntect::easy::HighlightLines;
use syntect::highlighting::{
    Color as SynColor, FontStyle, Style as SynStyle, StyleModifier, Theme as SynTheme, ThemeItem, ThemeSettings,
};

use crate::palette::{Palette, Theme};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

fn syntax_set() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

/// Scope selectors for the five syntax roles, plus the two deliberate
/// *demotions*. Read as a table of "which token kinds are a category".
///
/// Resolution is syntect's, not ours: when several selectors match a scope
/// stack the most specific one wins, so a two-atom selector overrides a
/// one-atom selector without the order of this list mattering. That is what
/// makes the demotions work — `keyword.operator` beats `keyword`, so `=`
/// and `&` stay code-coloured while `let` and `use` do not.
///
/// Three mappings that look wrong until you read the scopes a grammar
/// actually emits (dumped from `SyntaxSet` while writing this):
///
/// * **`storage.type` is a keyword, not a type.** Rust's `let` and its
///   `u32` are *both* `storage.type.rust`; the grammar does not distinguish
///   them, so no selector can. Colouring the pair as keywords is what every
///   editor does and what the alternative — colouring `let` as a type —
///   plainly is not. Named types still land in [`Palette::syn_type`],
///   because they come through `entity.name.*` and `support.type`.
/// * **A macro name is a call.** `format!` is `support.macro`, which is a
///   name being invoked; `syn_call` is "the name in a call or definition",
///   so it belongs there rather than in the fallthrough.
/// * **Nothing needs to say `punctuation`.** Punctuation carries no scope
///   the five selectors match, so it falls through to [`Palette::code`] on
///   its own. The only punctuation that needed naming is the operator kind
///   the grammars file under `keyword`.
const SYNTAX_SCOPES: [(&str, Role); 7] = [
    ("keyword, storage, constant.language, variable.language", Role::Keyword),
    // Demotion: an operator is punctuation the grammars happen to file
    // under `keyword`. The design system's rule is that punctuation is not
    // a category.
    ("keyword.operator", Role::Code),
    (
        "entity.name.function, entity.name.macro, support.function, support.macro, variable.function",
        Role::Call,
    ),
    (
        "entity.name.type, entity.name.class, entity.name.struct, entity.name.enum, entity.name.trait, \
         entity.name.union, entity.name.namespace, support.type, support.class",
        Role::Type,
    ),
    ("string", Role::String),
    ("constant.numeric", Role::Number),
    // Demotion: a comment is not one of the five, and drops below body text
    // rather than taking a colour of its own.
    ("comment", Role::Comment),
];

/// Which palette field a matched scope resolves to. An indirection only so
/// [`SYNTAX_SCOPES`] can be a `const` table naming roles rather than a
/// runtime list of colours; [`Role::of`] is the whole of it.
#[derive(Debug, Clone, Copy)]
enum Role {
    Keyword,
    Call,
    Type,
    String,
    Number,
    Code,
    Comment,
}

impl Role {
    fn of(self, pal: &Palette) -> Color {
        match self {
            Role::Keyword => pal.syn_keyword,
            Role::Call => pal.syn_call,
            Role::Type => pal.syn_type,
            Role::String => pal.syn_string,
            Role::Number => pal.syn_number,
            Role::Code => pal.code,
            Role::Comment => pal.dim,
        }
    }
}

/// The syntect theme for one app theme, built from that theme's palette —
/// see this module's doc comment for why it is built rather than loaded.
///
/// Everything a fenced block can be is here: a default foreground of
/// [`Palette::code`] for the majority of a block that is not one of the
/// five categories, and one rule per row of [`SYNTAX_SCOPES`]. No
/// background (`ui.rs` paints the block's surface) and no font style, since
/// the design system carries hierarchy in colour and position and asks for
/// bold nowhere.
fn theme(theme: Theme) -> &'static SynTheme {
    static DARK: OnceLock<SynTheme> = OnceLock::new();
    static LIGHT: OnceLock<SynTheme> = OnceLock::new();
    let build = |which: Theme| {
        let pal = which.palette();
        SynTheme {
            name: Some(format!("aldwin-{which:?}")),
            author: None,
            settings: ThemeSettings { foreground: Some(syn_color(pal.code)), ..ThemeSettings::default() },
            scopes: SYNTAX_SCOPES
                .iter()
                .map(|(selectors, role)| ThemeItem {
                    // Every selector here is a literal in this file, so a
                    // parse failure is a typo in the table above and not a
                    // condition a session can be in.
                    scope: selectors.parse().expect("SYNTAX_SCOPES selector parses"),
                    style: StyleModifier {
                        foreground: Some(syn_color(role.of(pal))),
                        background: None,
                        font_style: None,
                    },
                })
                .collect(),
        }
    };
    match theme {
        Theme::Dark => DARK.get_or_init(|| build(Theme::Dark)),
        Theme::Light => LIGHT.get_or_init(|| build(Theme::Light)),
    }
}

/// A palette colour as syntect sees it, opaque.
///
/// Every field of a [`Palette`] is a `Color::Rgb` by construction, so the
/// arm below it is unreachable. It is a mid grey rather than a panic
/// because a theme colour is not worth taking a session down over, and
/// rather than a palette value because this function has no theme to pick
/// one from — a caller reaching it is already outside the design system,
/// which is what `Color::Reset`, the only other thing that could arrive
/// here, means. The render snapshot's
/// `every_painted_cell_uses_a_palette_colour_never_the_terminals_own` is
/// what keeps `Reset` out of a frame in the first place.
fn syn_color(c: Color) -> SynColor {
    match c {
        Color::Rgb(r, g, b) => SynColor { r, g, b, a: 0xff },
        _ => SynColor { r: 0x80, g: 0x80, b: 0x80, a: 0xff },
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
/// What a memo entry is keyed by: the fence's language, its body, and the
/// theme — a re-render under a different theme is a different answer.
type MemoKey = (String, String, Theme);
static MEMO: OnceLock<Mutex<HashMap<MemoKey, Highlighted>>> = OnceLock::new();

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

    /// The gap this module was rewritten to close: a fenced block used to
    /// be the one region of the frame painted from syntect's own
    /// `base16-ocean` themes, so it carried hues the design system never
    /// chose. Nothing else caught it —
    /// `render_snapshot.rs`'s `every_painted_cell_uses_a_palette_colour_never_the_terminals_own`
    /// only rules out `Color::Reset`, and an arbitrary `Rgb` passes it.
    ///
    /// Seven colours are reachable and no eighth is: the five syntax roles,
    /// `code` for everything that is not a category, and `dim` for a
    /// comment. Asserted across several languages so a grammar emitting an
    /// unexpected scope shows up here rather than on screen.
    #[test]
    fn every_highlighted_colour_is_one_of_the_seven_roles() {
        let samples = [
            (
                "rust",
                "// c\nuse std::fmt;\npub struct Cfg { pub n: u32 }\nfn go() -> Result<(), String> {\n    \
                 let s: String = format!(\"x{}\", 1.5);\n    Cfg::new(&s).run();\n    Ok(())\n}\n",
            ),
            (
                "python",
                "# c\nimport os\nclass A(dict):\n    def f(self, x: int) -> str:\n        \
                 return f\"{x}\" + str(os.getcwd())\n",
            ),
            ("bash", "# c\nset -e\nfor f in *.rs; do\n  echo \"$f\" | grep -c 3\ndone\n"),
            ("json", "{\"a\": [1, 2.5, true, null], \"b\": \"s\"}\n"),
            ("yaml", "# c\nkey: value\nlist:\n  - 1\n  - \"two\"\n"),
        ];
        for theme in [Theme::Dark, Theme::Light] {
            let pal = theme.palette();
            let allowed =
                [pal.syn_keyword, pal.syn_call, pal.syn_type, pal.syn_string, pal.syn_number, pal.code, pal.dim];
            for (lang, body) in samples {
                for span in highlight_lines(lang, body, theme).iter().flatten() {
                    let fg = span.style.fg.expect("every highlighted span carries a foreground");
                    assert!(allowed.contains(&fg), "{lang} in {theme:?}: {:?} painted {fg:?}, not a palette role", span.content);
                }
            }
        }
    }

    /// The five roles are a mapping, not a decoration: the tokens a reader
    /// picks a block apart by have to actually land on them. Pinned per
    /// role, because a scope-selector edit that silently stops matching
    /// leaves the block still rendering — just flat, in `code`, which
    /// `every_highlighted_colour_is_one_of_the_seven_roles` would happily
    /// accept.
    #[test]
    fn each_syntax_role_claims_the_tokens_it_names() {
        let code = "// c\nfn go() {\n    let s: String = fmt(\"x\", 12);\n}\n";
        let pal = Theme::Dark.palette();
        let lines = highlight_lines("rust", code, Theme::Dark);
        let colour_of = |needle: &str| {
            lines
                .iter()
                .flatten()
                .find(|s| s.content.trim() == needle)
                .unwrap_or_else(|| panic!("no span for {needle:?}"))
                .style
                .fg
                .unwrap()
        };
        assert_eq!(colour_of("fn"), pal.syn_keyword, "a keyword");
        assert_eq!(colour_of("let"), pal.syn_keyword, "storage.type is a keyword — see SYNTAX_SCOPES");
        assert_eq!(colour_of("go"), pal.syn_call, "a definition's name");
        assert_eq!(colour_of("String"), pal.syn_type, "a named type");
        assert_eq!(colour_of("12"), pal.syn_number, "a numeric literal");
        assert_eq!(colour_of("="), pal.code, "an operator is punctuation, not a category");
        assert_eq!(colour_of("s"), pal.code, "an identifier is not a category");
        assert!(
            lines[0].iter().all(|s| s.style.fg == Some(pal.dim)),
            "a comment drops below body text, not into a colour of its own"
        );
        assert!(
            lines.iter().flatten().any(|s| s.content.contains('x') && s.style.fg == Some(pal.syn_string)),
            "a string literal"
        );
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

//! Stage 4 — the design tokens, generated rather than transcribed.
//!
//! `crates/tui/src/tokens.rs` is emitted from `.claude/design/` and
//! committed. The stage passes when regenerating produces no diff, which
//! makes drift between the app's palette and the design system impossible by
//! construction rather than detectable after the fact.
//!
//! Four sources, each read for the one thing only it states:
//!
//! * `tokens/colors.css` — every colour role, as an `oklch()` literal (the
//!   light theme mixes in six hexes). There is no ramp indirection to resolve
//!   any more; the generator converts OKLCH to sRGB itself.
//! * `tokens/layout.css` — the grid, in `ch` and `px`.
//! * `guidelines/glyphs.html` — the closed glyph table. The README carries
//!   the same fourteen marks as a prose sentence; the card is the one
//!   machine-readable copy.
//! * `frames/Aldwin Agent TUI.dc.html` — the brand mark and the context
//!   bar's ramp. Both are `color-mix()` expressions that exist nowhere else,
//!   and the mark's 108 cells are the shape of the letter itself (redrawn
//!   for a terminal's cell before they are emitted).
//!
//! **Ten roles are deliberately not carried.** `--chrome` and `--dot` paint
//! the mock's macOS title bar, which a terminal does not draw. `--syn` and
//! `--call` are "reserved, not applied in any current frame" — carrying them
//! would put a hue within reach of code the design says must not use it yet.
//! The six `--canvas-*` roles are the documentation page around the frames.
//! A role that is neither carried nor on that list is an error, not a silent
//! omission — see [`generate`].

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::{Baseline, Error, Result};

/// Roles the terminal does not draw, with the reason.
const UNCARRIED: [(&str, &str); 10] = [
    (
        "chrome",
        "the mock's macOS title bar; a terminal's title bar is the terminal's",
    ),
    ("dot", "the title bar's traffic lights; same"),
    (
        "syn",
        "reserved by colors.css: \"not applied in current frames\"",
    ),
    (
        "call",
        "reserved by colors.css: \"not applied in current frames\"",
    ),
    ("canvas", "the documentation page around the frames"),
    ("canvas-ink", "same"),
    ("canvas-body", "same"),
    ("canvas-caption", "same"),
    ("canvas-badge", "same"),
    ("canvas-link-hover", "same"),
];

/// The grid tokens the app consumes, and the constant each becomes. Only
/// these are emitted: `layout.css` also declares the mock's window measures
/// (`--fw`, `--chrome-h`, the body heights), and a constant nothing reads
/// would be this file asserting a layout rule rather than carrying a value.
const GRID: [(&str, &str); 12] = [
    ("margin-x", "MARGIN_X"),
    ("body-x", "BODY_X"),
    ("mark-col", "MARK_COL"),
    ("group-gap", "GROUP_GAP"),
    ("fact-col", "FACT_COL"),
    ("detail-col", "DETAIL_COL"),
    ("command-col", "COMMAND_COL"),
    ("number-col", "NUMBER_COL"),
    ("tree-w", "TREE_W"),
    ("pane-gap", "PANE_GAP"),
    ("gutter-ln", "GUTTER_LN"),
    ("sign-col", "SIGN_COL"),
];

/// Where the generated file lands, relative to the workspace root.
pub const OUTPUT: &str = "crates/tui/src/tokens.rs";

/// The frame, relative to the design directory.
pub const FRAME: &str = "frames/Aldwin Agent TUI.dc.html";

/// The imported design system. Everything the loop knows about the design is
/// read from here and from nowhere else.
pub fn design_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.claude/design")
}

/// Where the generated file lives in the workspace at `root`.
pub fn output_path(root: &Path) -> std::path::PathBuf {
    root.join(OUTPUT)
}

/// One theme's resolved colour roles, by token name.
type Roles = BTreeMap<String, Rgb>;

/// Emit the file. Returns its full text.
///
/// `baseline` is an input like the design files: its contradictions license
/// the glyphs in `MARKS_BY_EXCEPTION`.
///
/// # Errors
///
/// [`Error::Design`] when a design file this reads is missing something the
/// app needs — a role, a grid token, a glyph list, the launch mark — or
/// states it in a shape the generator does not parse, or when rustfmt rejects
/// the output; [`Error::Io`] when a design file cannot be read or rustfmt
/// cannot be run.
pub fn generate(design_dir: &Path, baseline: &Baseline) -> Result<String> {
    let colors = strip_comments(&std::fs::read_to_string(
        design_dir.join("tokens/colors.css"),
    )?);
    let layout = strip_comments(&std::fs::read_to_string(
        design_dir.join("tokens/layout.css"),
    )?);
    let glyph_card = std::fs::read_to_string(design_dir.join("guidelines/glyphs.html"))?;
    let frame = std::fs::read_to_string(design_dir.join(FRAME))?;

    let dark_raw = scope_declarations(&colors, ":root")?;
    let light_raw = scope_declarations(&colors, ".tui-light")?;

    let uncarried = |role: &str| UNCARRIED.iter().any(|(name, _)| *name == role);
    let carried: Vec<&String> = dark_raw.keys().filter(|role| !uncarried(role)).collect();

    // Every carried role must parse to a colour in the dark scope; the light
    // scope overrides most and inherits the rest, which is the design's own
    // arrangement rather than a fallback. Only a role the light scope does not
    // declare is inherited: one it declares and this generator cannot read is
    // an error, never a silent dark value in the light theme.
    let resolve_scope = |raw: &BTreeMap<String, String>,
                         inherited: Option<&Roles>|
     -> Result<Roles> {
        let mut out = Roles::new();
        for role in &carried {
            let rgb = match raw.get(*role) {
                Some(value) => parse_color(value).ok_or_else(|| {
                    Error::Design(format!("--{role}: {value} is not a colour this generator can read (oklch() or #hex)"))
                })??,
                None => inherited
                    .and_then(|roles| roles.get(*role).copied())
                    .ok_or_else(|| {
                        Error::Design(format!("--{role} is not declared"))
                    })?,
            };
            out.insert((*role).clone(), rgb);
        }
        Ok(out)
    };
    let dark = resolve_scope(&dark_raw, None)?;
    let light = resolve_scope(&light_raw, Some(&dark))?;

    // The mark and the gauge mix two roles in OKLCH; the mix has to happen on
    // the unrounded values, so the source roles are re-read as OKLCH here.
    let oklch_of = |raw: &BTreeMap<String, String>,
                    fallback: &BTreeMap<String, String>,
                    role: &str|
     -> Result<Oklch> {
        raw.get(role)
            .or_else(|| fallback.get(role))
            .and_then(|v| parse_oklch_or_hex(v))
            .ok_or_else(|| {
                Error::Design(format!(
                    "--{role} is needed for a color-mix and does not parse"
                ))
            })
    };

    let mark = fit_to_terminal(&mark_cells(&frame)?, MARK_TERMINAL_ROWS)?;
    check_gauge_against_frame(&frame)?;

    let mut out = header();

    for (name, theme, roles, raw) in [
        ("DARK", "Dark", &dark, &dark_raw),
        ("LIGHT", "Light", &light, &light_raw),
    ] {
        let fill = oklch_of(raw, &dark_raw, "fill")?;
        let win = oklch_of(raw, &dark_raw, "win")?;
        let track = oklch_of(raw, &dark_raw, "track")?;

        out.push_str(&format!(
            "pub(crate) const {name}: Palette = Palette {{\n    theme: Theme::{theme},\n"
        ));
        for (role, rgb) in roles {
            out.push_str(&format!("    {}: {},\n", field(role), rgb.literal()));
        }
        out.push_str("};\n\n");

        // The mark: each cell's top and bottom half, as `--fill` mixed over
        // `--win`. An empty cell is the ground twice, so a renderer can paint
        // every cell the same way.
        let mut mark_colors: Vec<Rgb> = Vec::new();
        out.push_str(&format!(
            "/// The brand mark for this theme: `[row][col]` of (upper half, lower half).\n\
             pub(crate) const MARK_{name}: [[(Color, Color); MARK_COLS]; MARK_ROWS] = [\n"
        ));
        for row in &mark {
            out.push_str("    [");
            for cell in row {
                let top = cell.top.map_or(win, |p| mix_oklch(fill, win, p));
                let bottom = cell.bottom.map_or(win, |p| mix_oklch(fill, win, p));
                let (top, bottom) = (top.to_rgb()?, bottom.to_rgb()?);
                mark_colors.push(top);
                mark_colors.push(bottom);
                out.push_str(&format!("({}, {}), ", top.literal(), bottom.literal()));
            }
            out.push_str("],\n");
        }
        out.push_str("];\n\n");

        // The gauge: one row per filled count, each row the ten segments left
        // to right — the filled run ramping to `--fill` at the leading edge,
        // then `--track`. `ContextBar.jsx`'s own arithmetic, resolved.
        let mut ramp_colors: Vec<Rgb> = Vec::new();
        out.push_str(&format!(
            "/// The context bar, `[filled][segment]`: for `n` filled segments the run ramps\n\
             /// `--fill` over `--track` in steps of `60/n` percent to full at the leading\n\
             /// edge; the rest are `--track`. Index with `gauge_filled(percent)`.\n\
             pub(crate) const GAUGE_{name}: [[Color; GAUGE_SEGMENTS]; GAUGE_SEGMENTS + 1] = [\n"
        ));
        for n in 0..=GAUGE_SEGMENTS {
            out.push_str("    [");
            for i in 0..GAUGE_SEGMENTS {
                let rgb = match gauge_mix(n, i) {
                    Some(p) => mix_oklch_f(fill, track, p).to_rgb()?,
                    None => track.to_rgb()?,
                };
                ramp_colors.push(rgb);
                out.push_str(&format!("{}, ", rgb.literal()));
            }
            out.push_str("],\n");
        }
        out.push_str("];\n\n");

        // Every colour of that theme as a flat list, so a conformance test
        // can ask "is this colour in the design system?" without naming the
        // fields — and without going stale when the design gains one.
        let mut values: Vec<(String, Rgb)> = roles
            .iter()
            .map(|(role, rgb)| (format!("--{role}"), *rgb))
            .collect();
        let mut seen: BTreeSet<(u8, u8, u8)> =
            values.iter().map(|(_, c)| (c.0, c.1, c.2)).collect();
        for rgb in &ramp_colors {
            if seen.insert((rgb.0, rgb.1, rgb.2)) {
                values.push(("the context bar, --fill mixed over --track".into(), *rgb));
            }
        }
        for rgb in &mark_colors {
            if seen.insert((rgb.0, rgb.1, rgb.2)) {
                values.push(("the mark, --fill mixed over --win".into(), *rgb));
            }
        }
        out.push_str(&format!(
            "pub(crate) const {name}_VALUES: [Color; {}] = [\n",
            values.len()
        ));
        for (what, rgb) in &values {
            out.push_str(&format!("    {}, // {what}\n", rgb.literal()));
        }
        out.push_str("];\n\n");
    }

    out.push_str(&format!(
        "// ---- The brand mark's shape, from the frame ------------------------\n\
         //\n\
         // An open A in half-block cells: the frame draws each cell as a\n\
         // `linear-gradient(top 50%, bottom 50%)`, which in a terminal is `▀` with\n\
         // an independent foreground and background. Redrawn from the frame's\n\
         // 6 rows to a terminal's cell shape; baseline.json records why.\n\n\
         pub(crate) const MARK_COLS: usize = {};\n\
         pub(crate) const MARK_ROWS: usize = {};\n\
         /// The glyph every mark cell is drawn with: upper half foreground, lower half background.\n\
         pub(crate) const MARK_CELL: char = '▀';\n\n",
        mark[0].len(),
        mark.len()
    ));

    out.push_str(&grid(&layout)?);
    out.push_str(&glyphs(&glyph_card, &frame, baseline)?);
    rustfmt(&out)
}

/// The generated source as `cargo fmt` leaves it.
///
/// The file is committed inside a workspace that stage 2 holds to `cargo fmt
/// --check`, so it is emitted formatted — the way bindgen and prost emit
/// theirs — rather than written one way by this generator and rewritten
/// another by the formatter, which would fail stage 2 or stage 4 whichever
/// ran last. The edition is the workspace's, which is what `cargo fmt` passes.
fn rustfmt(source: &str) -> Result<String> {
    let mut child = Command::new("rustfmt")
        .args(["--edition", "2021"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    // rustfmt reads all of stdin before it writes, so this cannot deadlock
    // against a full stdout pipe.
    child
        .stdin
        .take()
        .expect("stdin was piped above")
        .write_all(source.as_bytes())?;
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err(Error::Design(format!(
            "rustfmt rejected the generated source: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    String::from_utf8(output.stdout).map_err(|e| Error::Design(e.to_string()))
}

fn header() -> String {
    format!(
        "//! The design system, in Rust. **Generated — do not edit.**\n\
         //!\n\
         //! Emitted by `aldwin-review tokens --write` from `.claude/design/`:\n\
         //! `tokens/colors.css`, `tokens/layout.css`, `guidelines/glyphs.html` and\n\
         //! the frame. The review loop's stage 4 regenerates this file and fails if\n\
         //! the result differs, so the app's palette and the imported design cannot\n\
         //! drift apart.\n\
         //!\n\
         //! Roles the terminal does not draw are listed in\n\
         //! `crates/review/src/tokens.rs` with the reason; there are {}.\n\n\
         use ratatui::style::Color;\n\n\
         use crate::palette::Palette;\n\
         use crate::Theme;\n\n",
        UNCARRIED.len()
    )
}

/// The grid, from `layout.css`. `1ch` is a cell and `--row: 24px` is a row.
fn grid(layout: &str) -> Result<String> {
    let resolved = layout_tokens(layout);
    let mut out = String::from(
        "// ---- The grid, from tokens/layout.css -------------------------------\n\
         //\n\
         // Cell counts. `--body-x` is the one derived value the design states\n\
         // outright (\"prose: margin + mark column\"), and it is emitted as the\n\
         // design states it rather than re-derived here, so a disagreement between\n\
         // the two would surface as a stage-3 diff rather than hide in a sum.\n\n",
    );
    for (token, name) in GRID {
        let value = resolved
            .get(token)
            .ok_or_else(|| Error::Design(format!("layout.css declares no --{token} in ch")))?;
        out.push_str(&format!("pub(crate) const {name}: usize = {value};\n"));
    }
    out.push_str(&format!(
        "/// The context bar's segments — ten `━` in every frame.\n\
         pub(crate) const GAUGE_SEGMENTS: usize = {GAUGE_SEGMENTS};\n\
         /// How many of them a percentage fills: `ContextBar.jsx`'s `round(percent / 10)`, clamped.\n\
         pub(crate) fn gauge_filled(percent: u8) -> usize {{\n    \
             ((f64::from(percent) / 10.0).round() as usize).min(GAUGE_SEGMENTS)\n\
         }}\n"
    ));
    Ok(out)
}

/// The context bar's ten segments.
const GAUGE_SEGMENTS: usize = 10;

/// `ContextBar.jsx`: with `n` segments filled, segment `i` (left to right) is
/// `--fill` at `100 - (n - 1 - i) * 60 / n` percent over `--track`; a segment
/// past the run is `--track` (`None`).
fn gauge_mix(n: usize, i: usize) -> Option<f64> {
    if n == 0 || i >= n {
        return None;
    }
    let step = 60.0 / n as f64;
    Some(100.0 - (n - 1 - i) as f64 * step)
}

/// The closed glyph table, and the glyphs a recorded design contradiction
/// licenses on top of it.
///
/// `MARKS` has two sources and both are the design's own: the fourteen marks
/// `guidelines/glyphs.html` lists, and every non-ASCII character the frame
/// draws inside a window — `↑↓` in a footer, `⌄` on an open disclosure, the
/// `−` of a removed count, the punctuation prose carries. The mark's `▀` is
/// added last: the frame draws that cell as CSS, not as a character, so it
/// is the one glyph the app needs that no text in the design contains.
///
/// `MARKS_BY_EXCEPTION` is there because the design contradicts itself and
/// `crates/review/baseline.json` records where.
fn glyphs(card: &str, frame: &str, baseline: &Baseline) -> Result<String> {
    let mut marks = BTreeSet::new();
    // Each entry of the card is `…width:3ch">X</span>`; the glyph is what
    // sits between the closing bracket and the closing tag.
    for chunk in card.split("width:3ch\">").skip(1) {
        let Some(end) = chunk.find("</span>") else {
            continue;
        };
        marks.extend(chunk[..end].chars().filter(|c| !c.is_whitespace()));
    }
    if marks.is_empty() {
        return Err(Error::Design(
            "guidelines/glyphs.html lists no glyphs".into(),
        ));
    }
    for window in frame_windows(frame) {
        marks.extend(text_of(&window).chars().filter(|c| !c.is_ascii()));
    }
    marks.insert('▀');

    let mut excepted: BTreeSet<char> = BTreeSet::new();
    for c in &baseline.contradictions {
        excepted.extend(c.glyphs.chars());
    }
    let cite: Vec<&str> = baseline
        .contradictions
        .iter()
        .filter(|c| !c.glyphs.is_empty())
        .map(|c| c.id.as_str())
        .collect();

    let list = |set: &BTreeSet<char>| {
        set.iter()
            .map(|c| format!("{c:?}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    Ok(format!(
        "\n// ---- Glyphs ---------------------------------------------------------\n\
         //\n\
         // The design's closed table: guidelines/glyphs.html, plus every character\n\
         // the frame draws inside a window, plus the mark's half block.\n\
         pub(crate) const MARKS: [char; {}] = [{}];\n\
         \n\
         // Glyphs a recorded design contradiction licenses on top of it. Each is\n\
         // a bug in .claude/design/, not in the app; see crates/review/baseline.json\n\
         // ({}).\n\
         pub(crate) const MARKS_BY_EXCEPTION: [char; {}] = [{}];\n",
        marks.len(),
        list(&marks),
        cite.join(", "),
        excepted.len(),
        list(&excepted),
    ))
}

// ---- The frame --------------------------------------------------------------

/// One cell of the mark: the percentage of `--fill` mixed over `--win` in
/// each half, or `None` for the ground.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MarkCell {
    top: Option<u8>,
    bottom: Option<u8>,
}

/// The mark's height in a terminal, in rows. The frame's row is 24px on a
/// 14px font, so its half-cell is 8.4 × 12px; the cell `measure` pins
/// (8 × 18) has an 8 × 9 half-cell. On the frame's 18 columns, which keep the
/// letter's width and stroke, its 12 half-rows would need 16 to keep its
/// height — but the frame's facts sit on 24px rows too, and against a
/// terminal's shorter ones 8 rows of mark outweighs them. 7 is the balance:
/// the letter a little wide (1.14 : 1 against the frame's 1.05), the facts
/// 1.75 : 1 against the frame's 1.5. Only the mark is redrawn — it is the one
/// picture on the grid (`mark-is-drawn-for-a-terminal-cell` in baseline.json).
const MARK_TERMINAL_ROWS: usize = 7;

/// The frame's A, redrawn `rows` rows tall on the frame's columns.
///
/// Stretching the frame's half-rows would repeat some and not others. The
/// letter is read as strokes instead: every half-row of the frame is one run
/// per side, the frame's stroke wide and cut off at the centre, so the apex
/// is where the two strokes meet. What varies is the outer edge's distance
/// from the centre. That distance is sampled along the new half-rows,
/// between the frame's own, and each new half-row takes the fill of the
/// frame half-row nearest it. At the frame's own height this is the frame.
fn fit_to_terminal(mark: &[Vec<MarkCell>], rows: usize) -> Result<Vec<Vec<MarkCell>>> {
    let invalid = |why: &str| Error::Design(format!("the mark {why}"));
    let centre = mark[0].len() / 2;

    // Each frame half-row's left run, as (outer edge's distance from the
    // centre, run length, fill).
    let runs = mark
        .iter()
        .flat_map(|row| {
            [
                row.iter().map(|c| c.top).collect::<Vec<_>>(),
                row.iter().map(|c| c.bottom).collect(),
            ]
        })
        .map(|halves| {
            let left = &halves[..centre];
            let start = left.iter().position(Option::is_some);
            let end = left.iter().rposition(Option::is_some).map(|i| i + 1);
            match (start, end) {
                (Some(start), Some(end)) => Ok((
                    centre - start,
                    end - start,
                    halves[start].unwrap_or_default(),
                )),
                _ => Err(invalid("has an empty half-row")),
            }
        })
        .collect::<Result<Vec<_>>>()?;
    let stroke = runs.iter().map(|&(_, len, _)| len).max().unwrap_or(0);
    if runs.iter().any(|&(edge, len, _)| len != edge.min(stroke)) {
        return Err(invalid("is not one stroke per side"));
    }

    let last = runs.len() - 1;
    let halves: Vec<Vec<Option<u8>>> = (0..rows * 2)
        .map(|t| {
            let at = (t * last) as f64 / (rows * 2 - 1) as f64;
            let below = at.floor() as usize;
            let above = (below + 1).min(last);
            let (low, high) = (runs[below].0 as f64, runs[above].0 as f64);
            let edge = (low + (high - low) * (at - below as f64)).round() as usize;
            let fill = runs[at.round() as usize].2;
            let mut left = vec![None; centre];
            for cell in &mut left[centre - edge..(centre - edge + stroke).min(centre)] {
                *cell = Some(fill);
            }
            let right = left.iter().rev().copied().collect::<Vec<_>>();
            left.into_iter().chain(right).collect()
        })
        .collect();

    Ok(halves
        .chunks(2)
        .map(|pair| {
            pair[0]
                .iter()
                .zip(&pair[1])
                .map(|(&top, &bottom)| MarkCell { top, bottom })
                .collect()
        })
        .collect())
}

/// The mark's cells, `[row][col]`, from the launch frame.
///
/// The frame draws each row as a flex `div` of 1ch spans. A filled span's
/// background is `linear-gradient(<top> 50%, <bottom> 50%)` where each half is
/// `transparent` or `color-mix(in oklch, var(--fill) N%, var(--win))`.
fn mark_cells(frame: &str) -> Result<Vec<Vec<MarkCell>>> {
    let launch = frame_windows(frame)
        .into_iter()
        .find(|w| w.contains("data-screen-label=\"A launch\""))
        .ok_or_else(|| Error::Design("the frame has no `A launch` window".into()))?;

    let mut rows: Vec<Vec<MarkCell>> = Vec::new();
    for row_html in launch
        .split("<div style=\"display:flex;height:24px\">")
        .skip(1)
    {
        let row_html = row_html.split("</div>").next().unwrap_or("");
        let mut row = Vec::new();
        for span in row_html
            .split("<span style=\"width:1ch;height:24px")
            .skip(1)
        {
            let style = span.split('"').next().unwrap_or("");
            let cell = match style.find("linear-gradient(") {
                None => MarkCell {
                    top: None,
                    bottom: None,
                },
                Some(at) => {
                    let inner = &style[at + "linear-gradient(".len()..];
                    let (top, bottom) = split_gradient(inner)
                        .ok_or_else(|| Error::Design(format!("unreadable mark cell: {style}")))?;
                    MarkCell {
                        top: mix_percent(top),
                        bottom: mix_percent(bottom),
                    }
                }
            };
            row.push(cell);
        }
        if !row.is_empty() {
            rows.push(row);
        }
    }
    if rows.is_empty() {
        return Err(Error::Design("the launch frame has no mark rows".into()));
    }
    let width = rows[0].len();
    if rows.iter().any(|r| r.len() != width) {
        return Err(Error::Design(
            "the mark's rows are not all the same width".into(),
        ));
    }
    Ok(rows)
}

/// `<top> 50%, <bottom> 50%)…` → (`<top>`, `<bottom>`), each without its stop.
/// The halves may themselves contain commas (inside `color-mix(...)`), so the
/// split is on the ` 50%, ` between them rather than on a comma.
fn split_gradient(inner: &str) -> Option<(&str, &str)> {
    let (top, rest) = inner.split_once(" 50%, ")?;
    let bottom = rest.split(" 50%)").next()?;
    Some((top.trim(), bottom.trim()))
}

/// `color-mix(in oklch, var(--fill) 45%, var(--win))` → `Some(45)`;
/// `transparent` → `None`.
fn mix_percent(half: &str) -> Option<u8> {
    let rest = half.strip_prefix("color-mix(in oklch, var(--fill) ")?;
    rest.split('%').next()?.trim().parse().ok()
}

/// Every context bar the frame draws, as (filled percentages left to right,
/// empty segment count, the percentage shown) — one per window.
fn frame_gauges(frame: &str) -> Vec<(Vec<f64>, usize, u8)> {
    let mut out = Vec::new();
    for window in frame_windows(frame) {
        let Some(at) = window.find("Context ") else {
            continue;
        };
        let bar = &window[at..];
        let filled: Vec<f64> = bar
            .split("color-mix(in oklch, var(--fill) ")
            .skip(1)
            .filter_map(|chunk| {
                let (pct, rest) = chunk.split_once('%')?;
                rest.trim_start()
                    .starts_with(", var(--track)")
                    .then(|| pct.trim().parse().ok())?
            })
            .collect();
        let empty = bar
            .split("color:var(--track)\">")
            .nth(1)
            .and_then(|rest| rest.split('<').next())
            .map_or(0, |run| run.chars().filter(|&c| c == '━').count());
        let shown = bar
            .split("</span> ")
            .find_map(|s| s.split('%').next()?.trim().parse::<u8>().ok())
            .unwrap_or(0);
        out.push((filled, empty, shown));
    }
    out
}

/// The generator's gauge arithmetic is `ContextBar.jsx`'s, and the frame is
/// where the design's own rendering of it can be read back. Every bar in the
/// frame has to match what the formula gives for the percentage it shows —
/// a formula that drifted from the frame would otherwise generate cleanly.
fn check_gauge_against_frame(frame: &str) -> Result<()> {
    let gauges = frame_gauges(frame);
    if gauges.is_empty() {
        return Err(Error::Design("the frame draws no context bar".into()));
    }
    for (filled, empty, shown) in gauges {
        let n = ((f64::from(shown) / 10.0).round() as usize).min(GAUGE_SEGMENTS);
        let wanted: Vec<f64> = (0..n).filter_map(|i| gauge_mix(n, i)).collect();
        let close = filled.len() == wanted.len()
            && filled
                .iter()
                .zip(&wanted)
                .all(|(a, b)| (a - b).abs() < 0.01);
        if !close || empty != GAUGE_SEGMENTS - n {
            return Err(Error::Design(format!("the frame's {shown}% context bar is {filled:?} + {empty} empty; ContextBar.jsx's rule gives {wanted:?} + {}", GAUGE_SEGMENTS - n)));
        }
    }
    Ok(())
}

/// Every window in the frame — the balanced `<div … data-screen-label="…">`
/// subtree — so the canvas captions around them are never read as design.
fn frame_windows(frame: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(rel) = frame[from..].find("data-screen-label=\"") {
        let attr_at = from + rel;
        // Back up to the `<div` that carries the attribute.
        let Some(open) = frame[..attr_at].rfind("<div") else {
            break;
        };
        let end = balanced_div_end(frame, open).unwrap_or(frame.len());
        out.push(frame[open..end].to_string());
        from = end.max(attr_at + 1);
    }
    out
}

/// The index just past the `</div>` that closes the `<div` at `start`.
fn balanced_div_end(html: &str, start: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut at = start;
    loop {
        let open = html[at..].find("<div");
        let close = html[at..].find("</div>");
        match (open, close) {
            (Some(o), Some(c)) if o < c => {
                depth += 1;
                at += o + 4;
            }
            (_, Some(c)) => {
                depth -= 1;
                at += c + 6;
                if depth == 0 {
                    return Some(at);
                }
            }
            (Some(o), None) => {
                depth += 1;
                at += o + 4;
            }
            (None, None) => return None,
        }
    }
}

/// The rendered text of an HTML fragment: tags removed, entities decoded.
fn text_of(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

// ---- CSS ------------------------------------------------------------------

/// `--name: value;` pairs of one scope.
fn declarations(css: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for line in css.lines() {
        let Some(rest) = line.trim().strip_prefix("--") else {
            continue;
        };
        let Some((name, value)) = rest.split_once(':') else {
            continue;
        };
        map.insert(
            name.trim().to_string(),
            value.trim().trim_end_matches(';').trim().to_string(),
        );
    }
    map
}

/// The declarations of one colour scope, which must exist and declare
/// something: an empty light scope would otherwise inherit every role and
/// generate a light theme that is the dark one.
fn scope_declarations(css: &str, selector: &str) -> Result<BTreeMap<String, String>> {
    let found = declarations(scope(css, selector));
    if found.is_empty() {
        return Err(Error::Design(format!(
            "tokens/colors.css declares no roles in `{selector}`"
        )));
    }
    Ok(found)
}

fn scope<'a>(css: &'a str, selector: &str) -> &'a str {
    let Some(start) = css
        .find(&format!("{selector} {{"))
        .or_else(|| css.find(&format!("{selector}{{")))
    else {
        return "";
    };
    let body = &css[start..];
    match body.find('}') {
        Some(end) => &body[..end],
        None => body,
    }
}

fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        match rest[start..].find("*/") {
            Some(end) => rest = &rest[start + end + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// `--tui-add-code` would have been `add_code`; the tokens are now flat
/// (`--addcode`, `--label2`) and the field names mirror them exactly.
fn field(role: &str) -> String {
    role.replace('-', "_")
}

/// `layout.css` in cells: a `Nch` value is `N` cells. `px` values are the
/// mock's window measures and are not cells, so they are left out — asking
/// for one is an error rather than a wrong number.
fn layout_tokens(css: &str) -> BTreeMap<String, u16> {
    let mut out = BTreeMap::new();
    for (name, value) in declarations(css) {
        if let Some(n) = value
            .strip_suffix("ch")
            .and_then(|v| v.trim().parse::<u16>().ok())
        {
            out.insert(name, n);
        }
    }
    out
}

// ---- Colour -----------------------------------------------------------------

/// An sRGB colour, 8 bits a channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Rgb(u8, u8, u8);

impl Rgb {
    fn literal(self) -> String {
        format!(
            "Color::Rgb(0x{:02x}, 0x{:02x}, 0x{:02x})",
            self.0, self.1, self.2
        )
    }
}

/// A colour in OKLCH: lightness 0–1, chroma, hue in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Oklch {
    l: f64,
    c: f64,
    h: f64,
}

impl Oklch {
    fn to_rgb(self) -> Result<Rgb> {
        let (a, b) = (
            self.c * self.h.to_radians().cos(),
            self.c * self.h.to_radians().sin(),
        );
        oklab_to_rgb(self.l, a, b).map_err(|delta| {
            Error::Design(format!(
                    "oklch({} {} {}) is outside sRGB by ΔE OK {delta:.3}, past CSS Color 4's {GAMUT_JND}: clipping it would draw a visibly different colour",
                    self.l, self.c, self.h
                ))
        })
    }
}

/// `oklch(0.64 0.2 255)` or `#1e8a3c` → sRGB. `None` when the value is
/// neither; an error when it is an OKLCH colour sRGB cannot hold.
fn parse_color(value: &str) -> Option<Result<Rgb>> {
    if let Some(hex) = parse_hex(value) {
        return Some(Ok(hex));
    }
    parse_oklch(value).map(Oklch::to_rgb)
}

/// The same, but kept in OKLCH for mixing. A hex is lifted into OKLCH.
fn parse_oklch_or_hex(value: &str) -> Option<Oklch> {
    parse_oklch(value).or_else(|| parse_hex(value).map(rgb_to_oklch))
}

fn parse_oklch(value: &str) -> Option<Oklch> {
    let inner = value.strip_prefix("oklch(")?.strip_suffix(')')?;
    let mut parts = inner.split_whitespace();
    let l = parts.next()?.parse().ok()?;
    let c = parts.next()?.parse().ok()?;
    let h = parts.next()?.parse().ok()?;
    Some(Oklch { l, c, h })
}

fn parse_hex(value: &str) -> Option<Rgb> {
    let v = value.strip_prefix('#')?;
    if v.len() != 6 {
        return None;
    }
    Some(Rgb(
        u8::from_str_radix(&v[0..2], 16).ok()?,
        u8::from_str_radix(&v[2..4], 16).ok()?,
        u8::from_str_radix(&v[4..6], 16).ok()?,
    ))
}

/// CSS Color 4's just-noticeable difference, in ΔE OK.
///
/// A token in OKLCH can sit a little outside sRGB — `--del`, a red at
/// `oklch(0.7 0.2 27)`, does — and a terminal can draw only sRGB. CSS Color 4
/// maps such a colour by clipping each channel when the clipped colour is
/// within this distance of the original, because nobody can see the
/// difference; the generator does the same. Every role and mix the design
/// declares today clips within it. Further out, CSS reduces chroma to find a
/// different colour, and a terminal palette that silently drew a different
/// colour from the design's is the drift stage 4 exists to prevent — so the
/// generator refuses instead, and the design has to say what it means.
const GAMUT_JND: f64 = 0.02;

/// Björn Ottosson's OKLab → linear sRGB, clipped, then the sRGB transfer
/// curve. The error is the clip's ΔE OK when it is not below [`GAMUT_JND`].
fn oklab_to_rgb(l: f64, a: f64, b: f64) -> std::result::Result<Rgb, f64> {
    let l_ = l + 0.396_337_777_4 * a + 0.215_803_757_3 * b;
    let m_ = l - 0.105_561_345_8 * a - 0.063_854_172_8 * b;
    let s_ = l - 0.089_484_177_5 * a - 1.291_485_548_0 * b;
    let (l3, m3, s3) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    let r = 4.076_741_662_1 * l3 - 3.307_711_591_3 * m3 + 0.230_969_929_2 * s3;
    let g = -1.268_438_004_6 * l3 + 2.609_757_401_1 * m3 - 0.341_319_396_5 * s3;
    let bl = -0.004_196_086_3 * l3 - 0.703_418_614_7 * m3 + 1.707_614_701_0 * s3;
    let [r, g, bl] = [r, g, bl].map(|c| c.clamp(0.0, 1.0));
    let (cl, ca, cb) = linear_to_oklab(r, g, bl);
    let delta = ((l - cl).powi(2) + (a - ca).powi(2) + (b - cb).powi(2)).sqrt();
    if delta >= GAMUT_JND {
        return Err(delta);
    }
    Ok(Rgb(encode(r), encode(g), encode(bl)))
}

fn encode(linear: f64) -> u8 {
    let c = linear.clamp(0.0, 1.0);
    let v = if c <= 0.003_130_8 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (v * 255.0).round() as u8
}

fn decode(channel: u8) -> f64 {
    let c = f64::from(channel) / 255.0;
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear sRGB → OKLab.
fn linear_to_oklab(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
    let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
    let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
    (
        0.210_454_255_3 * l + 0.793_617_785_0 * m - 0.004_072_046_8 * s,
        1.977_998_495_1 * l - 2.428_592_205_0 * m + 0.450_593_709_9 * s,
        0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766_0 * s,
    )
}

fn rgb_to_oklch(rgb: Rgb) -> Oklch {
    let (lab_l, lab_a, lab_b) = linear_to_oklab(decode(rgb.0), decode(rgb.1), decode(rgb.2));
    let c = (lab_a * lab_a + lab_b * lab_b).sqrt();
    let h = lab_b.atan2(lab_a).to_degrees().rem_euclid(360.0);
    Oklch { l: lab_l, c, h }
}

/// CSS `color-mix(in oklch, a p%, b)`: lightness and chroma interpolate
/// linearly, hue along the shorter arc. Neither role this is used on is
/// achromatic, so the powerless-hue rule never applies.
fn mix_oklch(a: Oklch, b: Oklch, percent: u8) -> Oklch {
    mix_oklch_f(a, b, f64::from(percent))
}

fn mix_oklch_f(a: Oklch, b: Oklch, percent: f64) -> Oklch {
    let p = percent / 100.0;
    let mut dh = b.h - a.h;
    if dh > 180.0 {
        dh -= 360.0;
    } else if dh < -180.0 {
        dh += 360.0;
    }
    Oklch {
        l: a.l * p + b.l * (1.0 - p),
        c: a.c * p + b.c * (1.0 - p),
        h: (a.h + dh * (1.0 - p)).rem_euclid(360.0),
    }
}

// ---- The stage --------------------------------------------------------------

/// The stage itself: regenerate, and report the first line that differs.
///
/// The inner result is the stage's verdict: `Ok` with a count of the colours
/// and constants carried when the committed file is current, `Err` naming
/// the first stale line when it is not.
///
/// # Errors
///
/// As [`generate`]. A stale or missing committed file is the inner `Err`,
/// not this one.
pub fn check(
    root: &Path,
    design_dir: &Path,
    baseline: &Baseline,
) -> Result<std::result::Result<usize, String>> {
    let wanted = generate(design_dir, baseline)?;
    let path = output_path(root);
    let found = std::fs::read_to_string(&path).unwrap_or_default();
    if wanted == found {
        return Ok(Ok(wanted
            .lines()
            .filter(|l| l.contains("Color::Rgb") || l.starts_with("pub(crate) const"))
            .count()));
    }
    let at = wanted.lines().zip(found.lines()).position(|(a, b)| a != b);
    Ok(Err(match at {
        Some(n) => format!(
            "{} is stale at line {}:\n    design says  {}\n    file has     {}",
            OUTPUT,
            n + 1,
            wanted.lines().nth(n).unwrap_or("").trim(),
            found.lines().nth(n).unwrap_or("(end of file)").trim()
        ),
        None => format!(
            "{OUTPUT} is stale: {} lines expected, {} found",
            wanted.lines().count(),
            found.lines().count()
        ),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_role_the_design_declares_is_carried_or_explained() {
        let text = generate(&design_dir(), &Baseline::load().unwrap()).expect("tokens generate");
        let colors = strip_comments(
            &std::fs::read_to_string(design_dir().join("tokens/colors.css")).unwrap(),
        );
        for role in declarations(scope(&colors, ":root")).keys() {
            let carried = text.contains(&format!("    {}: Color::Rgb", field(role)));
            let excused = UNCARRIED.iter().any(|(name, _)| name == role);
            assert!(
                carried || excused,
                "--{role} is neither generated nor on the uncarried list"
            );
        }
    }

    /// The light scope declares fewer roles than the dark one (`--onfill`
    /// is inherited), and every carried role still resolves.
    #[test]
    fn the_light_theme_inherits_what_it_does_not_redeclare() {
        let text = generate(&design_dir(), &Baseline::load().unwrap()).expect("tokens generate");
        let light = text
            .split("pub(crate) const LIGHT: Palette")
            .nth(1)
            .expect("a LIGHT palette");
        assert!(
            light.contains("    onfill: Color::Rgb"),
            "--onfill must reach the light palette by inheritance"
        );
    }

    fn rgb(value: &str) -> Rgb {
        parse_color(value).expect("a colour").expect("inside sRGB")
    }

    /// A white and a black, and the round trip through OKLCH.
    #[test]
    fn oklch_conversion_hits_the_ends_of_the_scale() {
        assert_eq!(rgb("oklch(0.99 0 0)"), Rgb(252, 252, 252));
        assert_eq!(rgb("oklch(0 0 0)"), Rgb(0, 0, 0));
        assert_eq!(rgb("oklch(1 0 0)"), Rgb(255, 255, 255));
        for hex in ["#1e8a3c", "#d84040", "#f1d2cf", "#183020"] {
            let rgb = parse_hex(hex).unwrap();
            let back = rgb_to_oklch(rgb).to_rgb().unwrap();
            let close = |a: u8, b: u8| (i16::from(a) - i16::from(b)).abs() <= 1;
            assert!(
                close(rgb.0, back.0) && close(rgb.1, back.1) && close(rgb.2, back.2),
                "{hex} → {back:?}"
            );
        }
    }

    /// Generate from a copy of the design whose `colors.css` has been edited.
    fn generate_with_colors(name: &str, edit: impl Fn(&str) -> String) -> Result<String> {
        let dir = std::env::temp_dir().join(format!(
            "aldwin-review-tokens-{name}-{}",
            std::process::id()
        ));
        for file in [
            "tokens/colors.css",
            "tokens/layout.css",
            "guidelines/glyphs.html",
            FRAME,
        ] {
            let text = std::fs::read_to_string(design_dir().join(file))?;
            let text = if file == "tokens/colors.css" {
                edit(&text)
            } else {
                text
            };
            let path = dir.join(file);
            std::fs::create_dir_all(path.parent().expect("a file has a parent"))?;
            std::fs::write(path, text)?;
        }
        let generated = generate(&dir, &Baseline::load()?);
        let _ = std::fs::remove_dir_all(&dir);
        generated
    }

    /// A light value the generator cannot read used to fall back to the
    /// dark one, drawing a dark-theme colour in the light theme.
    #[test]
    fn an_unreadable_light_value_is_an_error_not_the_dark_one() {
        let result = generate_with_colors("unreadable", |css| {
            css.replace(
                "--label3: oklch(0.66 0.006 260);",
                "--label3: oklch(0.66 0.006);",
            )
        });
        let error = result.expect_err("an unreadable light --label3");
        assert!(error.to_string().contains("--label3"), "{error}");
    }

    /// With no light scope at all, every role would be inherited and the
    /// light theme would be the dark one.
    #[test]
    fn a_missing_light_scope_is_an_error() {
        let result = generate_with_colors("no-light", |css| {
            css.replace(".tui-light {", ".elsewhere {")
        });
        let error = result.expect_err("no .tui-light scope");
        assert!(error.to_string().contains(".tui-light"), "{error}");
    }

    /// A hair outside sRGB is clipped; well outside is refused, because a
    /// clip that far draws a colour the design never named.
    #[test]
    fn a_colour_well_outside_srgb_is_refused_rather_than_clipped() {
        assert!(parse_color("oklch(0.7 0.4 150)").expect("parses").is_err());
        assert!(parse_color("oklch(0.64 0.2 255)").expect("parses").is_ok());
    }

    /// The accent, checked against the value Firefox rendered the frame with
    /// (measured off the screenshot's `›`): a bright, saturated blue.
    #[test]
    fn the_accent_is_a_blue() {
        let Rgb(r, g, b) = rgb("oklch(0.64 0.2 255)");
        assert!(
            b > 200 && r < 80 && g > 100 && g < 160,
            "expected a blue, got ({r}, {g}, {b})"
        );
    }

    /// `color-mix` at 100% is the first colour and at 0% the second.
    #[test]
    fn a_mix_at_the_ends_is_one_of_its_inputs() {
        let fill = parse_oklch("oklch(0.53 0.2 258)").unwrap();
        let win = parse_oklch("oklch(0.19 0.006 260)").unwrap();
        assert_eq!(
            mix_oklch(fill, win, 100).to_rgb().unwrap(),
            fill.to_rgb().unwrap()
        );
        assert_eq!(
            mix_oklch(fill, win, 0).to_rgb().unwrap(),
            win.to_rgb().unwrap()
        );
        let mid = mix_oklch(fill, win, 50);
        assert!(
            (mid.l - 0.36).abs() < 0.001,
            "lightness interpolates linearly: {mid:?}"
        );
    }

    /// The mark as the frame draws it: six rows of eighteen, an open A whose
    /// two legs meet at the top and part at the bottom, ramping brighter
    /// downward (40 % at the apex, 100 % at the feet).
    #[test]
    fn the_mark_is_an_open_a() {
        let frame = std::fs::read_to_string(design_dir().join(FRAME)).unwrap();
        let mark = mark_cells(&frame).unwrap();
        assert_eq!((mark.len(), mark[0].len()), (6, 18));
        let filled = |row: &Vec<MarkCell>| {
            row.iter()
                .filter(|c| c.top.is_some() || c.bottom.is_some())
                .count()
        };
        assert_eq!(
            mark.iter().map(filled).collect::<Vec<_>>(),
            vec![6, 8, 6, 6, 8, 8]
        );
        assert_eq!(
            mark[0][6],
            MarkCell {
                top: None,
                bottom: Some(45)
            },
            "the apex starts as a lower half"
        );
        assert_eq!(
            mark[5][0],
            MarkCell {
                top: None,
                bottom: Some(100)
            },
            "the foot ends at full fill"
        );
        assert!(
            mark[2][7..11]
                .iter()
                .all(|c| c.top.is_none() && c.bottom.is_none()),
            "the A is open between its legs"
        );
    }

    /// The mark read as strokes: at the frame's own height it is the frame,
    /// and at a terminal's it keeps the frame's apex, stroke and fills.
    #[test]
    fn the_terminal_mark_is_the_frames_a_redrawn() {
        let frame = std::fs::read_to_string(design_dir().join(FRAME)).unwrap();
        let wide = mark_cells(&frame).unwrap();
        assert_eq!(fit_to_terminal(&wide, wide.len()).unwrap(), wide);

        let mark = fit_to_terminal(&wide, MARK_TERMINAL_ROWS).unwrap();
        let halves: Vec<Vec<Option<u8>>> = mark
            .iter()
            .flat_map(|row| {
                [
                    row.iter().map(|c| c.top).collect(),
                    row.iter().map(|c| c.bottom).collect(),
                ]
            })
            .collect();
        let picture: Vec<String> = halves
            .iter()
            .map(|h| {
                h.iter()
                    .map(|c| if c.is_some() { '#' } else { '.' })
                    .collect()
            })
            .collect();
        assert_eq!(
            picture,
            [
                ".......####.......",
                "......######......",
                "......######......",
                ".....###..###.....",
                ".....###..###.....",
                "....###....###....",
                "....###....###....",
                "...###......###...",
                "...###......###...",
                "..###........###..",
                "..###........###..",
                ".###..........###.",
                ".###..........###.",
                "###............###",
            ]
        );
        let fills = |h: &Vec<Option<u8>>| h.iter().flatten().copied().collect::<Vec<u8>>();
        assert_eq!(fills(&halves[0]), [40; 4], "the apex is the frame's");
        assert_eq!(fills(&halves[13]), [100; 6], "the feet are full fill");
    }

    /// The two bars the frame draws — four segments at 38–44 % and five at
    /// 46 % — are what `ContextBar.jsx`'s rule gives, and the check that
    /// proves it is the one `generate` runs.
    #[test]
    fn the_gauge_rule_reproduces_every_bar_in_the_frame() {
        let frame = std::fs::read_to_string(design_dir().join(FRAME)).unwrap();
        let gauges = frame_gauges(&frame);
        assert!(
            gauges
                .iter()
                .any(|(f, e, s)| f == &[55.0, 70.0, 85.0, 100.0] && *e == 6 && *s == 41),
            "{gauges:?}"
        );
        assert!(
            gauges
                .iter()
                .any(|(f, e, s)| f == &[52.0, 64.0, 76.0, 88.0, 100.0] && *e == 5 && *s == 46),
            "{gauges:?}"
        );
        assert!(
            gauges
                .iter()
                .any(|(f, e, s)| f.is_empty() && *e == 10 && *s == 0),
            "{gauges:?}"
        );
        check_gauge_against_frame(&frame).expect("the rule matches the frame");
        assert_eq!(gauge_mix(4, 0), Some(55.0));
        assert_eq!(gauge_mix(1, 0), Some(100.0));
        assert_eq!(gauge_mix(0, 0), None);
    }

    #[test]
    fn the_grid_is_read_in_cells() {
        let text = generate(&design_dir(), &Baseline::load().unwrap()).expect("tokens generate");
        for (name, value) in [
            ("MARGIN_X", 3),
            ("BODY_X", 5),
            ("MARK_COL", 2),
            ("TREE_W", 28),
            ("GUTTER_LN", 5),
        ] {
            assert!(
                text.contains(&format!("pub(crate) const {name}: usize = {value};")),
                "{name} should be {value}"
            );
        }
    }

    /// The card's fourteen marks and the frame's own characters both reach
    /// the table; the canvas captions around the frames do not.
    #[test]
    fn the_glyph_table_is_the_card_plus_the_frame() {
        let text = generate(&design_dir(), &Baseline::load().unwrap()).expect("tokens generate");
        let marks = text
            .split("pub(crate) const MARKS: [char; ")
            .nth(1)
            .unwrap()
            .split("];")
            .next()
            .unwrap();
        for glyph in [
            '›', '✓', '●', '○', '▎', '◆', '⋯', '━', '↩', '⌃', '⎋', '↺', '/', '?', '↑', '↓', '⌄',
            '−', '▀',
        ] {
            assert!(
                marks.contains(&format!("{glyph:?}")),
                "{glyph} missing from MARKS"
            );
        }
    }
}

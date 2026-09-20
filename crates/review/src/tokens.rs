//! Stage 3 — the design tokens, generated rather than transcribed.
//!
//! `crates/tui/src/tokens.rs` is emitted from `.claude/design/tokens/*.css`
//! and committed. The stage passes when regenerating produces no diff, which
//! makes drift between the app's palette and the design system impossible by
//! construction rather than detectable after the fact.
//!
//! Two levels of indirection are resolved, the same two the design system
//! itself uses: `semantic.css` maps a role to a ramp step
//! (`--tui-text: var(--color-neutral-100)`) and `palette.css` maps that step
//! to a value. The light theme is the same role map over a different set of
//! steps.
//!
//! **Three roles are deliberately not carried.** `--tui-add-bg` and
//! `--tui-del-bg` are `rgba()` tints for a browser; a terminal cell has one
//! opaque background, and the design ships `--tui-add-row` / `--tui-del-row`
//! beside them as the solid fills for exactly this reason. `--tui-line` is
//! marked legacy in `semantic.css` itself. A role that is neither carried nor
//! on that list is an error, not a silent omission — see [`generate`].

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Error, ErrorKind, Result};
use std::path::Path;

/// Roles the terminal cannot express, with the reason it cannot.
const UNCARRIED: [(&str, &str); 3] = [
    ("add-bg", "an rgba tint; a cell background is opaque, and --tui-add-row is the design's solid fill for it"),
    ("del-bg", "an rgba tint; --tui-del-row is the design's solid fill for it"),
    ("line", "marked legacy in semantic.css"),
];

/// Where the generated file lands, relative to the workspace root.
pub const OUTPUT: &str = "crates/tui/src/tokens.rs";

/// The imported design system. Everything the loop knows about the design is
/// read from here and from nowhere else — there used to be a second parser
/// (`design.rs`, 286 lines) resolving the same three files for a cell check
/// that has since become a hermetic test in `crates/tui`.
pub fn design_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.claude/design")
}

pub fn output_path(root: &Path) -> std::path::PathBuf {
    root.join(OUTPUT)
}

/// Emit the file. Returns its full text.
pub fn generate(design_dir: &Path) -> Result<String> {
    let palette = strip_comments(&std::fs::read_to_string(design_dir.join("tokens/palette.css"))?);
    let semantic = strip_comments(&std::fs::read_to_string(design_dir.join("tokens/semantic.css"))?);
    let cells = strip_comments(&std::fs::read_to_string(design_dir.join("tokens/cells.css"))?);

    let steps = hex_literals(&palette);
    let dark_roles = roles(scope(&semantic, ":root"));
    let light_roles = roles(scope(&semantic, ".tui-light"));

    // Every role the dark scope declares is a role the app must carry, unless
    // it is on the uncarried list. A role added upstream that nothing here
    // knows about is the case this check exists for: it would otherwise
    // regenerate cleanly and the app would simply not have the colour.
    let mut unknown: Vec<&str> = dark_roles
        .keys()
        .map(|k| k.as_str())
        .filter(|role| !UNCARRIED.iter().any(|(name, _)| name == role))
        .filter(|role| !steps.contains_key(dark_roles.get(*role).map(|s| s.as_str()).unwrap_or("")))
        .collect();
    unknown.sort_unstable();
    if !unknown.is_empty() {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("roles whose palette step is not a hex literal and which are not on the uncarried list: {unknown:?}"),
        ));
    }

    let resolve = |scope_roles: &BTreeMap<String, String>, role: &str| -> Option<(u8, u8, u8)> {
        steps.get(scope_roles.get(role)?).copied()
    };

    let carried: Vec<&String> = dark_roles
        .keys()
        .filter(|role| !UNCARRIED.iter().any(|(name, _)| *name == role.as_str()))
        .collect();

    let mut out = String::new();
    out.push_str(&header(&cells));

    for (name, theme, scope_roles) in [("DARK", "Dark", &dark_roles), ("LIGHT", "Light", &light_roles)] {
        out.push_str(&format!("pub(crate) const {name}: Palette = Palette {{\n    theme: Theme::{theme},\n"));
        for role in &carried {
            // The light scope overrides most roles and inherits the rest, so
            // a role it does not declare resolves against the dark map. That
            // is the design system's own arrangement, not a fallback.
            let rgb = resolve(scope_roles, role).or_else(|| resolve(&dark_roles, role)).ok_or_else(|| {
                Error::new(ErrorKind::InvalidData, format!("--tui-{role} resolves to no value in either scope"))
            })?;
            out.push_str(&format!("    {}: Color::Rgb(0x{:02x}, 0x{:02x}, 0x{:02x}),\n", field(role), rgb.0, rgb.1, rgb.2));
        }
        out.push_str("};\n\n");

        // Every value of that palette as a flat list, so a conformance test
        // can ask "is this colour in the design system?" without naming
        // forty-two fields — and without going stale when the design gains a
        // forty-third.
        out.push_str(&format!("pub(crate) const {name}_VALUES: [Color; {}] = [\n", carried.len()));
        for role in &carried {
            let rgb = resolve(scope_roles, role).or_else(|| resolve(&dark_roles, role)).expect("resolved above");
            out.push_str(&format!("    Color::Rgb(0x{:02x}, 0x{:02x}, 0x{:02x}), // --tui-{role}\n", rgb.0, rgb.1, rgb.2));
        }
        out.push_str("];\n\n");
    }

    out.push_str(&grid(&cells)?);
    out.push_str(&glyphs(&std::fs::read_to_string(design_dir.join("HANDOFF.md"))?)?);
    Ok(out)
}

fn header(cells: &str) -> String {
    let _ = cells;
    format!(
        "//! The design system, in Rust. **Generated — do not edit.**\n\
         //!\n\
         //! Emitted by `mjolnir-review tokens --write` from\n\
         //! `.claude/design/tokens/`. The review loop's stage 3 regenerates\n\
         //! this file and fails if the result differs, so the app's palette\n\
         //! and the imported design cannot drift apart.\n\
         //!\n\
         //! Roles the terminal cannot express are listed in\n\
         //! `crates/review/src/tokens.rs` with the reason; there are {}.\n\n\
         use ratatui::style::Color;\n\n\
         use crate::palette::Palette;\n\
         use crate::Theme;\n\n",
        UNCARRIED.len()
    )
}

/// The grid, from `cells.css`. Cell counts, not pixels: `--cell-w` and
/// `--cell-h` are one cell each by definition.
fn grid(cells: &str) -> Result<String> {
    let resolved = cell_tokens(cells);
    let mut out = String::from(
        "// ---- The grid, from tokens/cells.css --------------------------------\n\
         //\n\
         // Cell counts. Body text lands at MARGIN_X + LABEL_COL_WIDTH +\n\
         // LABEL_GUTTER as a consequence of the three, which is why cells.css\n\
         // declares no --body-col and nothing here restates one.\n\
         //\n\
         // Only tokens the app consumes are emitted. cells.css declares more\n\
         // — panel and bar heights among them — and generating a constant\n\
         // nothing reads would be this file asserting a layout rule rather\n\
         // than carrying a value. Whether the app *should* consume one of\n\
         // them is stage 5's question, not stage 3's.\n\n",
    );
    for (token, name) in [
        ("margin-x", "MARGIN_X"),
        ("label-col", "LABEL_COL_WIDTH"),
        ("label-gutter", "LABEL_GUTTER"),
        ("group-gap", "GROUP_GAP"),
        ("option-label-col", "OPTION_LABEL_COL"),
        ("step-mark-col", "STEP_MARK_COL"),
        ("step-content-col", "STEP_CONTENT_COL"),
        ("gutter-line-no-inline", "GUTTER_LINE_NO_INLINE"),
        ("diff-sign-col", "DIFF_SIGN_COL"),
    ] {
        let value = resolved
            .get(token)
            .ok_or_else(|| Error::new(ErrorKind::InvalidData, format!("cells.css declares no --{token}")))?;
        out.push_str(&format!("pub(crate) const {name}: usize = {value};\n"));
    }
    Ok(out)
}

/// The closed glyph table, and the glyphs a recorded design contradiction
/// licenses on top of it.
///
/// Two sources, deliberately kept apart in the output. `MARKS` is the
/// design's own table, parsed from `HANDOFF.md`'s Glyphs section. Anything in
/// `MARKS_BY_EXCEPTION` is there because the design contradicts itself and
/// `crates/review/baseline.json` records where — the `·` its own copy
/// mandates but its table omits, ADR 0002's box-drawing set. Every one of
/// those is a bug upstream, and the entry leaves the baseline when the design
/// is fixed.
///
/// Generating both is what lets the conformance test live in `crates/tui`
/// and read no files at all.
fn glyphs(handoff: &str) -> Result<String> {
    let mut marks = BTreeSet::new();
    let start = handoff
        .find("### Glyphs")
        .ok_or_else(|| Error::new(ErrorKind::InvalidData, "HANDOFF.md has no `### Glyphs` section"))?;
    for line in handoff[start..].lines().skip(1) {
        if line.starts_with("##") {
            break;
        }
        let Some(cell) = line.strip_prefix("| ") else { continue };
        let Some((first, _)) = cell.split_once('|') else { continue };
        for chunk in first.split('`').skip(1).step_by(2) {
            marks.extend(chunk.chars());
        }
    }
    if marks.is_empty() {
        return Err(Error::new(ErrorKind::InvalidData, "HANDOFF.md's Glyphs section parsed to nothing"));
    }

    let baseline = crate::Baseline::load()?;
    let mut excepted: BTreeSet<char> = BTreeSet::new();
    for c in &baseline.contradictions {
        excepted.extend(c.glyphs.chars());
    }
    let cite: Vec<&str> = baseline.contradictions.iter().filter(|c| !c.glyphs.is_empty()).map(|c| c.id.as_str()).collect();

    let list = |set: &BTreeSet<char>| set.iter().map(|c| format!("{c:?}")).collect::<Vec<_>>().join(", ");
    Ok(format!(
        "\n// ---- Glyphs ---------------------------------------------------------\n\
         //\n\
         // The design system's closed table, from HANDOFF.md's Glyphs section.\n\
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

/// `--tui-bar-bottom` -> `bar_bottom`, and `break` -> `break_` because it is
/// a Rust keyword.
fn field(role: &str) -> String {
    let name = role.replace('-', "_");
    if name == "break" { "break_".into() } else { name }
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

fn hex_literals(css: &str) -> BTreeMap<String, (u8, u8, u8)> {
    let mut map = BTreeMap::new();
    for line in css.lines() {
        let Some(rest) = line.trim().strip_prefix("--") else { continue };
        let Some((name, value)) = rest.split_once(':') else { continue };
        if let Some(rgb) = hex(value.trim().trim_end_matches(';').trim()) {
            map.insert(name.trim().to_string(), rgb);
        }
    }
    map
}

fn roles(css: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for line in css.lines() {
        let Some(rest) = line.trim().strip_prefix("--tui-") else { continue };
        let Some((name, value)) = rest.split_once(':') else { continue };
        let value = value.trim().trim_end_matches(';').trim();
        if let Some(step) = value.strip_prefix("var(--").and_then(|v| v.split(')').next()) {
            map.insert(name.trim().to_string(), step.to_string());
        }
    }
    map
}

fn scope<'a>(css: &'a str, selector: &str) -> &'a str {
    let Some(start) = css.find(&format!("{selector}{{")) else { return "" };
    let body = &css[start..];
    match body.find('}') {
        Some(end) => &body[..end],
        None => body,
    }
}

fn hex(value: &str) -> Option<(u8, u8, u8)> {
    let v = value.strip_prefix('#')?;
    if v.len() != 6 {
        return None;
    }
    Some((
        u8::from_str_radix(&v[0..2], 16).ok()?,
        u8::from_str_radix(&v[2..4], 16).ok()?,
        u8::from_str_radix(&v[4..6], 16).ok()?,
    ))
}

/// `cells.css` resolved to cell counts. The file is primitives plus `calc()`,
/// with a comment forbidding a derived value from being restated as a
/// literal, so resolving it is substitution to a fixed point rather than a
/// parser.
fn cell_tokens(css: &str) -> BTreeMap<String, u16> {
    let mut raw: Vec<(String, String)> = Vec::new();
    for line in css.lines() {
        let Some(rest) = line.trim().strip_prefix("--") else { continue };
        let Some((name, value)) = rest.split_once(':') else { continue };
        raw.push((name.trim().to_string(), value.trim().trim_end_matches(';').trim().to_string()));
    }
    let mut out = BTreeMap::from([("cell-w".to_string(), 1u16), ("cell-h".to_string(), 1)]);
    for _ in 0..raw.len() + 1 {
        let mut progressed = false;
        for (name, value) in &raw {
            if out.contains_key(name) {
                continue;
            }
            if let Some(n) = eval(value, &out) {
                out.insert(name.clone(), n);
                progressed = true;
            }
        }
        if !progressed {
            break;
        }
    }
    out
}

fn eval(value: &str, known: &BTreeMap<String, u16>) -> Option<u16> {
    let body = value.strip_prefix("calc(").and_then(|v| v.strip_suffix(')')).unwrap_or(value).trim();
    if let Some((left, right)) = body.split_once('*') {
        return Some(eval(left.trim(), known)? * right.trim().parse::<u16>().ok()?);
    }
    if body.contains('+') {
        let mut total = 0u16;
        for term in body.split('+') {
            total += eval(term.trim(), known)?;
        }
        return Some(total);
    }
    if let Some(name) = body.strip_prefix("var(--").and_then(|v| v.strip_suffix(')')) {
        return known.get(name.trim()).copied();
    }
    body.parse::<u16>().ok()
}

/// The stage itself: regenerate, and report the first line that differs.
pub fn check(root: &Path, design_dir: &Path) -> Result<std::result::Result<usize, String>> {
    let wanted = generate(design_dir)?;
    let path = output_path(root);
    let found = std::fs::read_to_string(&path).unwrap_or_default();
    if wanted == found {
        return Ok(Ok(wanted.lines().filter(|l| l.contains("Color::Rgb") || l.starts_with("pub(crate) const")).count()));
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
        None => format!("{OUTPUT} is stale: {} lines expected, {} found", wanted.lines().count(), found.lines().count()),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn design() -> std::path::PathBuf {
        design_dir()
    }

    #[test]
    fn every_role_the_design_declares_is_carried_or_explained() {
        let text = generate(&design()).expect("tokens generate");
        let semantic = strip_comments(&std::fs::read_to_string(design().join("tokens/semantic.css")).unwrap());
        for role in roles(scope(&semantic, ":root")).keys() {
            let carried = text.contains(&format!("    {}: Color::Rgb", field(role)));
            let excused = UNCARRIED.iter().any(|(name, _)| name == role);
            assert!(carried || excused, "--tui-{role} is neither generated nor on the uncarried list");
        }
    }

    /// The resolver handles every shape `cells.css` uses, including tokens
    /// the app does not consume and so does not emit — a `calc()` this could
    /// not evaluate would surface as a missing constant rather than as a
    /// wrong one, so it is worth asserting separately from the output.
    #[test]
    fn the_resolver_handles_every_calc_in_the_file() {
        let css = strip_comments(&std::fs::read_to_string(design().join("tokens/cells.css")).unwrap());
        let resolved = cell_tokens(&css);
        for (token, value) in [("margin-x", 3), ("step-content-col", 29), ("panel-permission-h", 18), ("pane-commands-w", 48)] {
            assert_eq!(resolved.get(token), Some(&value), "--{token} should resolve to {value}");
        }
    }

    /// The derived grid values that reach the app: one is a sum of two
    /// tokens and one a sum of three, one of which is itself a sum.
    #[test]
    fn the_grid_resolves_through_calc() {
        let text = generate(&design()).expect("tokens generate");
        for (name, value) in [
            ("MARGIN_X", 3),
            ("LABEL_COL_WIDTH", 8),
            ("LABEL_GUTTER", 2),
            ("GROUP_GAP", 6),
            ("STEP_MARK_COL", 10),
            ("STEP_CONTENT_COL", 29),
        ] {
            assert!(
                text.contains(&format!("pub(crate) const {name}: usize = {value};")),
                "{name} should resolve to {value}"
            );
        }
    }
}

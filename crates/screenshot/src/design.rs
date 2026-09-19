//! The design system, read from `.claude/design/` at run time.
//!
//! Nothing here is a constant restated in the harness. The palette, the role
//! map and the glyph table are parsed from the files the design system is
//! imported into, so a re-sync moves the gates with it rather than leaving
//! them asserting last month's design.
//!
//! Two levels of indirection are resolved: `semantic.css` maps a role to a
//! ramp step (`--tui-text: var(--color-neutral-100)`) and `palette.css` maps
//! that step to a value. The light theme is the same role map over a
//! different set of steps, which is why a theme is one class on the frame and
//! one lookup here.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Error, ErrorKind, Result};
use std::path::{Path, PathBuf};

use crate::geometry::Theme;

pub struct Design {
    /// Role name (without the `--tui-` prefix) to RGB, per theme.
    dark:  BTreeMap<String, (u8, u8, u8)>,
    light: BTreeMap<String, (u8, u8, u8)>,
    /// The closed glyph table — the **marks**. Key hints and typography are
    /// not in it and are carried as a cited exception in the baseline file.
    pub marks: BTreeSet<char>,
}

impl Design {
    pub fn dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.claude/design")
    }

    pub fn load() -> Result<Self> {
        Self::load_from(&Self::dir())
    }

    pub fn load_from(dir: &Path) -> Result<Self> {
        let palette = strip_comments(&std::fs::read_to_string(dir.join("tokens/palette.css"))?);
        let semantic = strip_comments(&std::fs::read_to_string(dir.join("tokens/semantic.css"))?);
        let handoff = std::fs::read_to_string(dir.join("HANDOFF.md"))?;

        let steps = literals(&palette);
        // Every declaration, hex or not. A role pointing at an `rgba()` step —
        // the diff backgrounds are translucent by design — is *known* and
        // simply outside what a cell-grid harness can resolve; a role pointing
        // at a name that does not exist at all is a rename nobody noticed.
        let declared = declarations(&palette);
        let dark_roles = roles(scope(&semantic, ":root"));
        let light_roles = roles(scope(&semantic, ".tui-light"));

        let resolve = |roles: &BTreeMap<String, String>| -> BTreeMap<String, (u8, u8, u8)> {
            roles.iter().filter_map(|(k, v)| steps.get(v).map(|rgb| (k.clone(), *rgb))).collect()
        };

        let dark = resolve(&dark_roles);
        let light_resolved = resolve(&light_roles);

        // Every role `.tui-light` declares must actually resolve. Inheriting
        // the rest of the dark map is correct — the light theme overrides most
        // roles and leans on the base for the others — but a *declared* light
        // role that failed to resolve would leave a dark value sitting in the
        // light map, and every light-theme colour and role-pairing result
        // would then be computed against the wrong theme while the run
        // reported normally. The old `dark.is_empty() || light.is_empty()`
        // check could never catch it, because `light` was seeded from `dark`.
        let missing: Vec<&str> = light_roles
            .iter()
            .filter(|(role, step)| !light_resolved.contains_key(*role) && !declared.contains(step.as_str()))
            .map(|(role, _)| role.as_str())
            .collect();
        if !missing.is_empty() {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!(".tui-light declares roles whose palette step does not exist: {missing:?}"),
            ));
        }

        let mut light = dark.clone();
        light.extend(light_resolved);

        if dark.is_empty() || light_roles.is_empty() {
            return Err(Error::new(ErrorKind::InvalidData, "no roles resolved from the design tokens"));
        }

        Ok(Design { dark, light, marks: glyph_table(&handoff) })
    }

    pub fn roles(&self, theme: Theme) -> &BTreeMap<String, (u8, u8, u8)> {
        match theme {
            Theme::Dark => &self.dark,
            Theme::Light => &self.light,
        }
    }

    /// The ground roles — the seven-rung ladder a band's identity is a step
    /// on. Dimming blends toward one of these, so they are the only legal
    /// far end of a blend.
    pub fn grounds(&self, theme: Theme) -> Vec<(u8, u8, u8)> {
        const GROUNDS: [&str; 7] = ["ground", "scrim", "bar", "bar-bottom", "recess", "break", "panel-title"];
        self.roles(theme).iter().filter(|(k, _)| GROUNDS.contains(&k.as_str())).map(|(_, v)| *v).collect()
    }

    /// The ink ramp — the roles that exist to be *read*, never to be a band.
    pub const INK: [&'static str; 8] = ["text", "body", "code", "quiet", "value", "label", "dim", "context"];

    /// True only when every role this colour resolves to is a *surface* — a
    /// ground rung, or the legacy rule colour that is not ink either.
    ///
    /// Aliasing makes this subtler than it looks, in both directions.
    /// `--tui-reverse-ink` and `--tui-scrim` share a value, so a bare "is this
    /// a ground" flags the wordmark — ink on a reversed band — as a band
    /// colour painted on text. And `--tui-bar` and `--tui-line` share
    /// `#474251` in the dark theme, so requiring *only* ground names made the
    /// check fail open: a glyph painted in the top-bar ground could never be
    /// reported, which is one of the two defects this gate exists to catch.
    /// Naming the non-ink surfaces explicitly fixes both.
    pub fn is_only_ground(&self, theme: Theme, rgb: (u8, u8, u8)) -> bool {
        const SURFACES: [&str; 8] = ["ground", "scrim", "bar", "bar-bottom", "recess", "break", "panel-title", "line"];
        let names = self.role_names(theme, rgb);
        !names.is_empty() && names.iter().all(|n| SURFACES.contains(n))
    }

    /// Whether a colour is a palette role *dimmed toward a ground*, and by how
    /// much.
    ///
    /// The transcript dims while a decision panel is open — the design calls
    /// it a dimmed-and-scrimmed session — and a blend is by construction not
    /// a palette value. Without this the colour gate reports every dimmed
    /// cell: 33 violations on `prompt`, 53 on `approval`, all of them correct
    /// rendering. Each one measured as an exact 45% blend of a role toward
    /// `--tui-ground`, which is what this reconstructs rather than trusts.
    pub fn dimmed(&self, theme: Theme, colour: (u8, u8, u8)) -> Option<(String, u8)> {
        let grounds = self.grounds(theme);
        for (name, role) in self.roles(theme) {
            for ground in &grounds {
                if let Some(alpha) = blend_factor(*role, *ground, colour) {
                    return Some((name.clone(), alpha));
                }
            }
        }
        None
    }

    /// Which roles, if any, a colour belongs to. A colour in no role at all is
    /// a colour from outside the design system.
    pub fn role_names(&self, theme: Theme, rgb: (u8, u8, u8)) -> Vec<&str> {
        self.roles(theme).iter().filter(|(_, v)| **v == rgb).map(|(k, _)| k.as_str()).collect()
    }
}

/// Solve `target = role·a + ground·(1-a)` for `a`, and accept only if every
/// channel agrees to within a rounding step.
fn blend_factor(role: (u8, u8, u8), ground: (u8, u8, u8), target: (u8, u8, u8)) -> Option<u8> {
    let channels = [(role.0, ground.0, target.0), (role.1, ground.1, target.1), (role.2, ground.2, target.2)];
    let (r, g, t) = *channels
        .iter()
        .max_by_key(|(r, g, _)| (*r as i32 - *g as i32).unsigned_abs())?;
    if r == g {
        return None;
    }
    let alpha = (t as f32 - g as f32) / (r as f32 - g as f32);
    if !(0.02..=0.98).contains(&alpha) {
        return None;
    }
    for (r, g, t) in channels {
        let expected = (r as f32 * alpha + g as f32 * (1.0 - alpha)).round();
        if (expected - t as f32).abs() > 1.0 {
            return None;
        }
    }
    Some((alpha * 100.0).round() as u8)
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

/// Every `--name:` declared in the palette, whatever its value.
fn declarations(css: &str) -> std::collections::BTreeSet<String> {
    css.lines()
        .filter_map(|line| line.trim().strip_prefix("--")?.split_once(':').map(|(name, _)| name.trim().to_string()))
        .collect()
}

/// `--name:#rrggbb` pairs.
fn literals(css: &str) -> BTreeMap<String, (u8, u8, u8)> {
    let mut map = BTreeMap::new();
    for line in css.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("--") else { continue };
        let Some((name, value)) = rest.split_once(':') else { continue };
        let value = value.trim().trim_end_matches(';').trim();
        if let Some(rgb) = hex(value) {
            map.insert(name.trim().to_string(), rgb);
        }
    }
    map
}

/// `--tui-name: var(--color-step)` pairs inside one scope, keyed without the
/// `--tui-` prefix.
fn roles(css: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for line in css.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("--tui-") else { continue };
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

/// The `### Glyphs` table: every glyph in a leading `` `x` `` cell.
fn glyph_table(handoff: &str) -> BTreeSet<char> {
    let mut marks = BTreeSet::new();
    let Some(start) = handoff.find("### Glyphs") else { return marks };
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
    marks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_glyph_table_is_read_from_the_handoff_not_restated_here() {
        let design = Design::load().expect("design tokens are checked in");
        for mark in ['▌', '●', '◐', '○', '✔', '▶', '█', '+', '-'] {
            assert!(design.marks.contains(&mark), "{mark} missing from the parsed glyph table");
        }
        assert!(!design.marks.contains(&'─'), "box drawing is not in the closed table");
    }

    #[test]
    fn both_themes_resolve_two_levels_down_to_a_value() {
        let design = Design::load().expect("design tokens are checked in");
        // `--tui-ground: var(--color-ground-3)` → `#27232f`, the frame ground
        // every dark capture is mostly made of.
        assert_eq!(design.roles(Theme::Dark).get("ground"), Some(&(0x27, 0x23, 0x2f)));
        assert_eq!(design.roles(Theme::Light).get("ground"), Some(&(0xf7, 0xf5, 0xfa)));
    }
}

//! What a frame declares, as data rather than as a picture.
//!
//! This exists because of a measured waste. On `run-1789850385` six blind
//! judges spent 890K tokens and 225 tool calls, and the largest single
//! activity was **decoding PNGs to recover values the harness already held**:
//! three of them independently solved the panel's dim blend back to α = 0.450,
//! which is `palette::PANEL_TRANSCRIPT_OPACITY` and which the colour gate
//! already reconstructs per cell.
//!
//! Worse than the cost is the error class it creates. A judge reading a pixel
//! sees `#9a95a4` and infers `--tui-dim`; the cell's *declared* role is
//! `--tui-context`, which resolves to the same neutral-500. That inference is
//! "What a judge will raise again" item 2 in the conformance spec — a
//! standing, recurring defect report about a correct frame. With the declared
//! role in hand it cannot be made at all.
//!
//! So every frame writes a `.facts.json` beside its PNG: one entry per
//! **span** — a maximal run of cells sharing one foreground, background and
//! attribute set — carrying the cells it covers, its text, the semantic roles
//! its two colours resolve to, the band behind it, and the contrast between
//! them. A judge is handed this instead of a decoder.
//!
//! Spans, not cells, because a span is the unit a reader actually reasons
//! about: `read  (call-1)` is one span, which is how you can see at a glance
//! that the name and the target share a colour where the design splits them
//! (conformance Class A item 37). A per-cell dump hides that in 3,000 rows.

use serde::Serialize;

use crate::contrast;
use crate::design::Design;
use crate::geometry::Theme;
use crate::regions::{Map, Zone};
use crate::vt::{Cell, Color, Grid};

/// One maximal run of cells sharing a foreground, a background and an
/// attribute set.
#[derive(Debug, Clone, Serialize)]
pub struct Span {
    pub row:      u16,
    /// First cell, inclusive.
    pub from:     u16,
    /// Last cell, inclusive. A wide glyph's continuation cell is counted
    /// here but contributes no character to `text`.
    pub to:       u16,
    pub text:     String,
    /// The zone the span *starts* in. A span that crosses a zone boundary is
    /// reported by its start, which is the only one of its columns a reader
    /// is likely to be checking against the grid.
    pub zone:     Zone,
    /// `--tui-*` role the foreground resolves to, `dim(role, α%)` when it is
    /// a role blended toward a ground, or the bare hex when it is neither.
    pub ink:      String,
    pub ink_hex:  String,
    /// Same, for the background.
    pub ground:   String,
    pub ground_hex: String,
    /// The role of the band this row sits in — which is not always the
    /// span's own background: a diff row, a selection band and a recessed
    /// field all paint over the band they sit on.
    pub band:     Option<String>,
    /// WCAG contrast of ink against its own background, to three decimals.
    /// `None` for a span with no glyphs, where the number would be noise.
    pub contrast: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Facts {
    pub scene: String,
    pub size:  String,
    pub theme: String,
    pub cols:  u16,
    pub rows:  u16,
    /// The derived grid, restated here so a reader of this file alone does
    /// not have to open `cells.css` to know what column 13 is.
    pub grid:  Grid_,
    pub spans: Vec<Span>,
    /// Every distinct (ink role, ground role) pair the frame actually paints,
    /// with its ratio — the input to the `contrast` gate, kept here so a
    /// reader can see what was measured rather than only what failed.
    pub adjacencies: Vec<Adjacency>,
}

/// Named with a trailing underscore to avoid colliding with [`crate::vt::Grid`],
/// which is the cell buffer rather than the design's column scheme.
#[derive(Debug, Clone, Serialize)]
pub struct Grid_ {
    pub margin:   u16,
    pub label:    (u16, u16),
    pub gutter:   (u16, u16),
    pub body:     u16,
    pub body_end: u16,
}

#[derive(Debug, Clone, Serialize)]
pub struct Adjacency {
    pub ink:      String,
    pub ground:   String,
    pub ratio:    f64,
    /// How many spans paint this pair, so a one-cell accident is
    /// distinguishable from the frame's dominant body text.
    pub spans:    usize,
    /// Where to look for the first one.
    pub first_at: (u16, u16),
}

/// Describe a colour the way a reader needs it: the role if it is one, the
/// role and the blend if it is dimmed, the raw hex if it is neither.
fn describe(design: &Design, theme: Theme, colour: Color) -> (String, String) {
    match colour {
        Color::Rgb(r, g, b) => {
            let hex = format!("#{r:02x}{g:02x}{b:02x}");
            let names = design.role_names(theme, (r, g, b));
            if !names.is_empty() {
                return (format!("--tui-{}", names.join("/")), hex);
            }
            match design.dimmed(theme, (r, g, b)) {
                Some((role, alpha)) => (format!("dim(--tui-{role}, {alpha}%)"), hex),
                None => ("(off-palette)".to_string(), hex),
            }
        }
        Color::Indexed(n) => (format!("(ansi {n})"), format!("ansi-{n}")),
        Color::Default => ("(terminal default)".to_string(), "default".to_string()),
    }
}

fn same_run(a: &Cell, b: &Cell) -> bool {
    a.effective() == b.effective() && a.attrs == b.attrs
}

pub fn derive(grid: &Grid, map: &Map, theme: Theme, design: &Design, scene: &str, size: &str) -> Facts {
    let mut spans: Vec<Span> = Vec::new();

    for row in 0..grid.rows {
        let mut col = 0u16;
        while col < grid.cols {
            let head = grid.get(row, col);
            let start = col;
            let mut text = String::new();
            if !head.is_continuation() {
                text.push(head.ch);
            }
            col += 1;
            while col < grid.cols {
                let next = grid.get(row, col);
                // A continuation cell belongs to the glyph before it whatever
                // its own attributes say — the terminal owns that cell, not
                // the app — so it never breaks a run.
                if !next.is_continuation() && !same_run(&head, &next) {
                    break;
                }
                if !next.is_continuation() {
                    text.push(next.ch);
                }
                col += 1;
            }

            let (fg, bg) = head.effective();
            let (ink, ink_hex) = describe(design, theme, fg);
            let (ground, ground_hex) = describe(design, theme, bg);
            let has_glyphs = text.chars().any(|c| c != ' ');
            spans.push(Span {
                row,
                from: start,
                to: col - 1,
                text,
                zone: map.zone(start),
                ink,
                ink_hex,
                ground,
                ground_hex,
                band: map.band(row).and_then(|b| b.role.clone()),
                contrast: has_glyphs.then(|| contrast::of(fg, bg)).flatten(),
            });
        }
    }

    Facts {
        scene: scene.to_string(),
        size: size.to_string(),
        theme: theme.to_string(),
        cols: grid.cols,
        rows: grid.rows,
        grid: Grid_ {
            margin:   crate::regions::MARGIN_X,
            label:    (crate::regions::MARGIN_X, crate::regions::MARGIN_X + crate::regions::LABEL_COL_WIDTH),
            gutter:   (crate::regions::MARGIN_X + crate::regions::LABEL_COL_WIDTH, crate::regions::BODY_COL),
            body:     crate::regions::BODY_COL,
            body_end: grid.cols - crate::regions::MARGIN_X,
        },
        adjacencies: adjacencies(&spans),
        spans,
    }
}

/// Every distinct ink-on-ground pair the frame paints, most-contrasting last
/// so the worst is at the top of the list a reader opens.
fn adjacencies(spans: &[Span]) -> Vec<Adjacency> {
    let mut out: Vec<Adjacency> = Vec::new();
    for span in spans.iter().filter(|s| s.contrast.is_some() && s.text.chars().any(|c| c != ' ')) {
        let ratio = span.contrast.unwrap_or(0.0);
        match out.iter_mut().find(|a| a.ink == span.ink && a.ground == span.ground) {
            Some(existing) => existing.spans += 1,
            None => out.push(Adjacency {
                ink:      span.ink.clone(),
                ground:   span.ground.clone(),
                ratio,
                spans:    1,
                first_at: (span.row, span.from),
            }),
        }
    }
    out.sort_by(|a, b| a.ratio.partial_cmp(&b.ratio).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// A compact rendering for a human, since the JSON is for a judge and a
/// judge is not the only reader.
impl Facts {
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("{} {} {} — {}×{} cells\n", self.scene, self.size, self.theme, self.cols, self.rows));
        out.push_str(&format!(
            "margin {}  label {}..{}  gutter {}..{}  body {}..{}\n\n",
            self.grid.margin, self.grid.label.0, self.grid.label.1, self.grid.gutter.0, self.grid.gutter.1, self.grid.body, self.grid.body_end
        ));
        out.push_str("ink on ground, worst first\n");
        for a in &self.adjacencies {
            out.push_str(&format!("  {:>7.3}:1  {:<34} on {:<28} ×{}\n", a.ratio, a.ink, a.ground, a.spans));
        }
        out.push_str("\nspans with glyphs\n");
        for s in self.spans.iter().filter(|s| s.text.chars().any(|c| c != ' ')) {
            out.push_str(&format!(
                "  r{:>2} c{:>3}..{:<3} {:<30} {:<34} on {}\n",
                s.row,
                s.from,
                s.to,
                format!("{:?}", s.text),
                s.ink,
                s.ground
            ));
        }
        out
    }
}

//! The regression gate.
//!
//! It is the one gate that compares two revisions rather than inspecting one
//! frame, and it does not capture the old revision to do it. `render.snap`
//! already records every cell of every scene at four sizes in both themes, so
//! the question "did anything outside the focus set move" is answerable by
//! diffing that file against the merge-base — no second build, no second
//! capture set, and no exposure to the timing volatility a live capture has,
//! because `TestBackend` renders statically.
//!
//! The cost is stated rather than hidden: this is an `App`-level check. A
//! regression that appears *only* through a real terminal is not what this
//! gate catches — that is what the other five are for.

use std::collections::BTreeSet;
use std::io::{Error, Result};
use std::path::Path;
use std::process::Command;

use serde::Serialize;

pub const SNAPSHOT: &str = "crates/tui/tests/snapshots/render.snap";

/// One `=== Dark conversation 120x36` block of the snapshot.
#[derive(Debug, Clone, Serialize)]
pub struct Change {
    pub section: String,
    pub scene:   String,
    pub rows:    Vec<usize>,
}

/// A focus entry: a scene, optionally narrowed to a row range.
#[derive(Debug, Clone)]
pub struct Focus {
    pub scene: String,
    pub rows:  Option<(usize, usize)>,
}

impl Focus {
    pub fn parse(spec: &str) -> std::result::Result<Vec<Focus>, String> {
        spec.split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|entry| match entry.split_once(':') {
                None => Ok(Focus { scene: entry.to_string(), rows: None }),
                Some((scene, range)) => {
                    let (from, to) = range.split_once('-').ok_or_else(|| format!("bad row range in {entry:?}, want scene:from-to"))?;
                    let parse = |s: &str| s.trim().parse::<usize>().map_err(|_| format!("bad row number in {entry:?}"));
                    Ok(Focus { scene: scene.to_string(), rows: Some((parse(from)?, parse(to)?)) })
                }
            })
            .collect()
    }

    fn covers(&self, change: &Change) -> bool {
        if self.scene != change.scene {
            return false;
        }
        match self.rows {
            None => true,
            Some((from, to)) => change.rows.iter().all(|r| *r >= from && *r <= to),
        }
    }
}

pub fn merge_base(repo: &Path, branch: &str) -> Result<String> {
    let out = Command::new("git").current_dir(repo).args(["merge-base", "HEAD", branch]).output()?;
    if !out.status.success() {
        return Err(Error::other(format!("no merge-base with {branch}: {}", String::from_utf8_lossy(&out.stderr).trim()),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Which snapshot sections differ from the baseline revision.
///
/// Whole sections are compared rather than a textual diff being parsed: the
/// snapshot is already delimited by `=== theme scene WxH` headers, so reading
/// both versions and comparing block for block says exactly which scene at
/// which size moved, with no hunk arithmetic to get wrong.
pub fn changes(repo: &Path, baseline_rev: &str) -> Result<Vec<Change>> {
    let show = Command::new("git")
        .current_dir(repo)
        .args(["show", &format!("{baseline_rev}:{SNAPSHOT}")])
        .output()?;
    if !show.status.success() {
        return Err(Error::other(format!("{SNAPSHOT} is not in {baseline_rev}: {}", String::from_utf8_lossy(&show.stderr).trim()),
        ));
    }
    let before = String::from_utf8_lossy(&show.stdout).into_owned();
    let after = std::fs::read_to_string(repo.join(SNAPSHOT))?;

    let (old, new) = (sections(&before), sections(&after));
    let names: BTreeSet<&String> = old.iter().map(|(n, _)| n).chain(new.iter().map(|(n, _)| n)).collect();

    let mut changes = Vec::new();
    for name in names {
        let a = old.iter().find(|(n, _)| n == name).map(|(_, b)| b.as_slice()).unwrap_or(&[]);
        let b = new.iter().find(|(n, _)| n == name).map(|(_, b)| b.as_slice()).unwrap_or(&[]);
        let rows: Vec<usize> = (0..a.len().max(b.len())).filter(|i| a.get(*i) != b.get(*i)).collect();
        if !rows.is_empty() {
            changes.push(Change { section: name.clone(), scene: scene_of(name), rows });
        }
    }
    Ok(changes)
}

/// Changes that no focus entry accounts for. These are the regression: the
/// change reached a region it was not supposed to touch.
pub fn unaccounted(changes: &[Change], focus: &[Focus]) -> Vec<Change> {
    changes.iter().filter(|c| !focus.iter().any(|f| f.covers(c))).cloned().collect()
}

fn sections(snapshot: &str) -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for line in snapshot.lines() {
        if let Some(header) = line.strip_prefix("=== ") {
            out.push((header.trim().to_string(), Vec::new()));
        } else if let Some((_, body)) = out.last_mut() {
            body.push(line.to_string());
        }
    }
    out
}

/// `Dark conversation 120x36` → `conversation`.
fn scene_of(section: &str) -> String {
    section.split_whitespace().nth(1).unwrap_or(section).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_focus_entry_without_rows_covers_its_whole_scene() {
        let change = Change { section: "Dark approval 120x36".into(), scene: "approval".into(), rows: vec![3, 9] };
        let focus = Focus::parse("approval").unwrap();
        assert!(unaccounted(&[change], &focus).is_empty());
    }

    #[test]
    fn a_change_outside_the_declared_rows_is_a_regression() {
        let change = Change { section: "Dark approval 120x36".into(), scene: "approval".into(), rows: vec![3, 30] };
        let focus = Focus::parse("approval:1-10").unwrap();
        assert_eq!(unaccounted(&[change], &focus).len(), 1);
    }

    #[test]
    fn a_scene_nobody_declared_is_a_regression_whatever_moved_in_it() {
        let change = Change { section: "Light long 80x24".into(), scene: "long".into(), rows: vec![0] };
        let focus = Focus::parse("approval,conversation").unwrap();
        assert_eq!(unaccounted(&[change], &focus).len(), 1);
    }

    #[test]
    fn sections_split_on_the_snapshots_own_headers() {
        let snap = "=== Dark empty 80x24\nrow one\nrow two\n=== Dark empty 120x36\nrow three\n";
        let parsed = sections(snap);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].1, vec!["row one", "row two"]);
        assert_eq!(scene_of(&parsed[1].0), "empty");
    }
}

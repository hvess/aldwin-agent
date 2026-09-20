//! The run's record — `review.md`, written into the frames directory.
//!
//! Small on purpose. What it holds is what a reader needs tomorrow or on a
//! pull request: which stages ran, what they measured, and what the judge
//! concluded. The frames sit beside it rather than inside it.
//!
//! The split is the one thing here worth defending. **This file writes only
//! what was measured; the judge's score and findings are appended by the
//! skill.** The agent that made the change is the one that would otherwise
//! write the verdict sentence, and "close, two stages clean" is nothing
//! false and much less useful than the numbers.

use std::io::Result;
use std::path::Path;

use crate::stages::Outcome;

/// Stats a stage reports when it passes, parsed from what the tool said
/// rather than counted again here.
pub fn test_counts(output: &str) -> (u32, u32, u32) {
    let (mut passed, mut failed, mut ignored) = (0, 0, 0);
    for line in output.lines().filter(|l| l.starts_with("test result:")) {
        let mut fields = line.split_whitespace();
        while let Some(word) = fields.next() {
            let n = word.parse::<u32>().ok();
            match (n, fields.clone().next()) {
                (Some(n), Some("passed;")) => passed += n,
                (Some(n), Some("failed;")) => failed += n,
                (Some(n), Some("ignored;")) => ignored += n,
                _ => {}
            }
        }
    }
    (passed, failed, ignored)
}

pub struct Run<'a> {
    pub goal:     &'a str,
    pub focus:    &'a str,
    pub commit:   &'a str,
    pub outcomes: &'a [Outcome],
    pub frames:   Option<&'a Path>,
    pub captured: usize,
}

pub fn write(dir: &Path, run: &Run) -> Result<std::path::PathBuf> {
    let mut out = String::from("# Review\n\n");
    out.push_str(&format!("**Goal:** {}\n\n", run.goal));
    out.push_str(&format!("**Focus:** {}\n\n", run.focus));
    out.push_str(&format!("**Commit:** `{}`\n\n", run.commit));

    out.push_str("## Stages 0–4 — deterministic\n\n");
    out.push_str("| stage | result | measured |\n| --- | --- | --- |\n");
    for outcome in run.outcomes {
        let result = if outcome.passed { "ok" } else { "**FAIL**" };
        // A failure's detail is a tool's diagnostic, often many lines; the
        // table gets its first line and the section below gets the rest.
        let measured = outcome.detail.lines().next().unwrap_or("").replace('|', "\\|");
        out.push_str(&format!("| {} | {result} | {measured} |\n", outcome.stage));
    }

    let failures: Vec<&Outcome> = run.outcomes.iter().filter(|o| !o.passed).collect();
    out.push_str(&format!(
        "\n{} of {} stages passed.\n",
        run.outcomes.len() - failures.len(),
        run.outcomes.len()
    ));

    if !failures.is_empty() {
        out.push_str("\n### Issues\n\n");
        for outcome in failures {
            out.push_str(&format!("**{}**\n\n```\n{}\n```\n\n", outcome.stage, outcome.detail.trim_end()));
        }
    }

    match run.frames {
        Some(path) => out.push_str(&format!("\n{} frames captured, in `{}`.\n", run.captured, path.display())),
        None => out.push_str("\nNo frames captured (`--no-capture`); stage 5 needs them.\n"),
    }

    out.push_str(
        "\n## Stage 5 — confidence\n\n\
         _Appended by the review skill once the judge has run. Until then this\n\
         run is incomplete: stages 0–4 say nothing about whether the change\n\
         matches its design._\n",
    );

    let path = dir.join("review.md");
    std::fs::write(&path, out)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parsed from cargo's own summary lines rather than counted here, so the
    /// report cannot disagree with the tool it is reporting on.
    #[test]
    fn test_counts_sum_every_target() {
        let output = "\
test result: ok. 54 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 289 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out
test result: FAILED. 11 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out";
        assert_eq!(test_counts(output), (354, 1, 2));
    }

    #[test]
    fn output_with_no_summary_lines_counts_nothing() {
        assert_eq!(test_counts("error: could not compile"), (0, 0, 0));
    }
}

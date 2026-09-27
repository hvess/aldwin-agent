//! The run's record — `review.html`, written into the run's directory.
//!
//! Small on purpose, and self-contained: no external stylesheet, no script,
//! no embedded frames. What it holds is what a reader needs tomorrow or on a
//! pull request — which stages ran, what they measured, and what the judges
//! concluded. The frames and the diff sit next to it on disk.
//!
//! **The page is plain HTML and stays that way.** Dressing the referee in the
//! players' kit makes it harder to trust, so the design system this loop
//! enforces is deliberately not applied here. That is a rule, not an
//! omission.
//!
//! The other rule worth defending: **this file writes only what was
//! measured.** Each judge's findings are written by the `judge` command into
//! the placeholder left for them, verbatim. The agent that made the change is
//! the one that would otherwise write the verdict sentence, and "close, two
//! stages clean" is nothing false and much less useful than the findings.

use std::path::{Path, PathBuf};

use crate::judges::{Assignment, Judge};
use crate::stages::Outcome;
use crate::{Error, Result};

/// Where a judge's section goes. Left as a comment so a reader of the raw
/// file can see the seam, and so writing it is a string replace rather than
/// a parse.
fn marker(judge: Judge) -> String {
    format!("<!-- stage-{} -->", judge.stage())
}

const STYLE: &str = "\
:root{color-scheme:light dark;--fg:#1a1a1a;--bg:#fff;--muted:#666;--line:#e3e3e3;--ok:#1a7f37;--bad:#b3261e;--code:#f5f5f5}\
@media(prefers-color-scheme:dark){:root{--fg:#e6e6e6;--bg:#16181c;--muted:#9aa0a6;--line:#2c2f34;--ok:#4ac26b;--bad:#f2795f;--code:#1f2227}}\
*{box-sizing:border-box}\
body{margin:0;padding:32px 20px 64px;background:var(--bg);color:var(--fg);\
font:15px/1.6 -apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,sans-serif}\
main{max-width:860px;margin:0 auto}\
h1{font-size:24px;margin:0 0 4px}\
h2{font-size:17px;margin:40px 0 12px;padding-bottom:6px;border-bottom:1px solid var(--line)}\
h3{font-size:15px;margin:24px 0 8px}\
dl{margin:16px 0;display:grid;grid-template-columns:auto 1fr;gap:6px 16px}\
dt{color:var(--muted)}dd{margin:0}\
table{border-collapse:collapse;width:100%;margin:12px 0;font-size:14px}\
th,td{text-align:left;padding:7px 10px;border-bottom:1px solid var(--line);vertical-align:top}\
th{color:var(--muted);font-weight:600}\
code,pre{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:13px}\
pre{background:var(--code);padding:12px 14px;border-radius:6px;overflow-x:auto}\
.ok{color:var(--ok);font-weight:600}.bad{color:var(--bad);font-weight:600}\
.count{margin:12px 0;color:var(--muted)}\
.note{color:var(--muted);font-size:14px}\
@media(max-width:520px){dl{grid-template-columns:1fr;gap:2px 0}dt{margin-top:8px}}";

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

/// Everything one run of stages 1 to 5 hands the report.
#[derive(Debug)]
pub struct Run<'a> {
    /// The short hash of `HEAD` the run was taken on, or empty outside git.
    pub commit: &'a str,
    /// The staged tree the run reviewed — what the pass record is keyed by.
    pub tree: &'a str,
    /// Each deterministic stage's verdict, in the order they ran.
    pub outcomes: &'a [Outcome],
    /// Which judges the change calls for, and why.
    pub assignments: &'a [Assignment],
    /// The directory the frames were captured into, or `None` when capture
    /// was skipped or nothing called for it.
    pub frames: Option<&'a Path>,
    /// How many frames were captured.
    pub captured: usize,
}

impl Run<'_> {
    /// Whether `judge` can be written into this run: the change calls for
    /// it, every deterministic stage passed, and — for the frames judge —
    /// there are frames to look at. Only then does the report leave it a
    /// placeholder, so `judge` has nowhere to write a pass over a failing run:
    /// a judge reading code that does not build, or frames drawn by it,
    /// reports a consequence as a cause.
    ///
    /// # Examples
    ///
    /// ```
    /// use aldwin_review::judges::{Assignment, Judge};
    /// use aldwin_review::report::Run;
    /// use aldwin_review::stages::Outcome;
    /// let outcomes = [Outcome { stage: "3 test", passed: true, detail: String::new() }];
    /// let assignments = [Assignment::new(Judge::Code, true, "a crate changed")];
    /// let run = Run {
    ///     commit: "abc1234",
    ///     tree: "t",
    ///     outcomes: &outcomes,
    ///     assignments: &assignments,
    ///     frames: None,
    ///     captured: 0,
    /// };
    /// assert!(run.reaches(Judge::Code));
    /// assert!(!run.reaches(Judge::Frames));
    /// ```
    pub fn reaches(&self, judge: Judge) -> bool {
        let required = self
            .assignments
            .iter()
            .any(|a| a.judge() == judge && a.standing().required());
        let has_frames = judge != Judge::Frames || (self.frames.is_some() && self.captured > 0);
        required && self.outcomes.iter().all(|o| o.passed) && has_frames
    }
}

/// Text into HTML text. Everything user- or tool-supplied goes through this:
/// a clippy diagnostic is full of `&`, `<` and `>`, and one unescaped `<`
/// silently swallows the rest of a cell.
fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Writes `review.html` for `run` into `dir` and returns its path.
///
/// # Errors
///
/// When the file cannot be written.
pub fn write(dir: &Path, run: &Run) -> Result<PathBuf> {
    let path = dir.join("review.html");
    std::fs::write(&path, render(run))?;
    Ok(path)
}

fn render(run: &Run) -> String {
    let failures: Vec<&Outcome> = run.outcomes.iter().filter(|o| !o.passed).collect();
    let passed = run.outcomes.len() - failures.len();

    let mut out = String::new();
    out.push_str("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">");
    out.push_str("<title>Aldwin review</title><style>");
    out.push_str(STYLE);
    out.push_str("</style></head><body><main>");

    out.push_str("<h1>Review</h1>");
    out.push_str(&format!(
        "<dl><dt>On</dt><dd><code>{}</code></dd><dt>Tree</dt><dd><code>{}</code></dd></dl>",
        esc(run.commit),
        esc(run.tree)
    ));

    out.push_str("<h2>Stages 1&ndash;5 &middot; deterministic</h2>");
    out.push_str("<table><tr><th>stage</th><th>result</th><th>measured</th></tr>");
    for outcome in run.outcomes {
        let (class, word) = if outcome.passed {
            ("ok", "ok")
        } else {
            ("bad", "FAIL")
        };
        // A failure's detail is a tool diagnostic, often many lines; the table
        // takes its first line and the section below takes the rest.
        let measured = outcome.detail.lines().next().unwrap_or("");
        out.push_str(&format!(
            "<tr><td>{}</td><td class=\"{class}\">{word}</td><td>{}</td></tr>",
            esc(outcome.stage),
            esc(measured)
        ));
    }
    out.push_str("</table>");
    out.push_str(&format!(
        "<p class=\"count\">{passed} of {} stages passed.</p>",
        run.outcomes.len()
    ));

    if !failures.is_empty() {
        out.push_str("<h3>Issues</h3>");
        for outcome in failures {
            out.push_str(&format!(
                "<p><strong>{}</strong></p><pre>{}</pre>",
                esc(outcome.stage),
                esc(outcome.detail.trim_end())
            ));
        }
    }

    if let Some(path) = run.frames {
        out.push_str(&format!(
            "<p class=\"note\">{} frames captured, in <code>{}</code>.</p>",
            run.captured,
            esc(&path.display().to_string())
        ));
    }

    out.push_str("<h2>Stages 6&ndash;8 &middot; judges</h2>");
    for assignment in run.assignments {
        let judge = assignment.judge();
        out.push_str(&format!(
            "<h3>Stage {} &middot; {}</h3>",
            judge.stage(),
            judge.title()
        ));
        if !assignment.standing().required() {
            out.push_str(&format!(
                "<p class=\"note\">Not required: {}.</p>",
                esc(assignment.reason())
            ));
        } else if run.reaches(judge) {
            out.push_str(&marker(judge));
            out.push_str(&format!(
                "<p class=\"note\">Required: {}. Written by <code>judge</code> once the \
                 judge has run; until then this run is incomplete.</p>",
                esc(assignment.reason())
            ));
        } else {
            out.push_str(
                "<p class=\"note\">Not reached. A judge reads a run whose deterministic \
                 stages all passed (and, for frames, whose frames were captured), and this \
                 one is not that. Fix and run the loop again.</p>",
            );
        }
    }

    out.push_str("</main></body></html>\n");
    out
}

/// One finding, as a judge reported it.
#[derive(Debug, serde::Deserialize)]
pub struct Finding {
    /// `major` or `minor`; anything unrecognised is read as minor.
    pub severity: String,
    /// The source the finding is measured against — a file, and the section
    /// or line in it. A finding that cannot name its source is not a finding.
    #[serde(default)]
    pub source: String,
    /// What the source requires, cited to where it says so.
    pub expected: String,
    /// What the change does instead.
    pub found: String,
    /// Where: `file:line` for code, the frames for pictures.
    pub at: String,
}

/// What a judge hands back.
///
/// Four lists, and only the first decides. `contradictions` is a source
/// disagreeing with itself and `questions` is the judge saying it could not
/// tell — both are worth recording and neither is the change's fault, so
/// neither fails it. Keeping them out of the verdict is what stops a judge's
/// uncertainty from reading as a defect.
#[derive(Debug, serde::Deserialize)]
pub struct Verdict {
    /// Which pass of the loop this judge ran on, counted from one.
    pub iteration: u32,
    /// Deviations the judge can demonstrate; the only list that decides.
    pub findings: Vec<Finding>,
    /// Anything the judge confirmed holds, one line each.
    #[serde(default)]
    pub matches: Vec<String>,
    /// Places a source disagrees with itself. Candidates for
    /// `baseline.json`; they do not fail the stage.
    #[serde(default)]
    pub contradictions: Vec<String>,
    /// What the judge could not resolve. They do not fail it either.
    #[serde(default)]
    pub questions: Vec<String>,
}

impl Verdict {
    /// A judge passes with no findings at all. A minor finding fails it as a
    /// major one does; severity orders the fixing, not the verdict. This is
    /// the threshold of 100 the developer set on 2026-09-24, stated without
    /// the arithmetic that only ever compared it against zero.
    ///
    /// # Examples
    ///
    /// ```
    /// use aldwin_review::report::Verdict;
    /// let verdict: Verdict = serde_json::from_str(r#"{"iteration": 1, "findings": []}"#).unwrap();
    /// assert!(verdict.passes());
    /// ```
    pub fn passes(&self) -> bool {
        self.findings.is_empty()
    }
}

/// A finding's severity as the report reads it: `major`, and anything else
/// — `minor`, a misspelling, a `Minor` — is a minor.
fn severity(f: &Finding) -> &'static str {
    match f.severity.trim().to_ascii_lowercase().as_str() {
        "major" => "major",
        _ => "minor",
    }
}

/// Renders `judge`'s verdict into the report, replacing its placeholder, and
/// returns whether it passed.
///
/// A command rather than an instruction to edit HTML by hand. The first
/// version of this loop left the append to the skill's discipline and the
/// section came back empty on three consecutive runs — the agent was busy
/// reading findings and fixing code, which is exactly when a manual step
/// gets skipped.
///
/// # Errors
///
/// When the report cannot be read or written, or has no placeholder for
/// `judge` — it was already written, or the run did not reach it.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
/// use aldwin_review::judges::Judge;
/// use aldwin_review::report::{self, Verdict};
/// let verdict: Verdict = serde_json::from_str(&std::fs::read_to_string("code.json")?)?;
/// let passed = report::write_verdict(Path::new("run/review.html"), Judge::Code, &verdict)?;
/// # Ok::<(), aldwin_review::Error>(())
/// ```
pub fn write_verdict(report: &Path, judge: Judge, verdict: &Verdict) -> Result<bool> {
    let text = std::fs::read_to_string(report)?;
    let filled = fill(&text, judge, verdict).ok_or_else(|| {
        Error::Report(format!(
            "{} has no stage {} placeholder: it was already written, or the run did not reach it (a deterministic stage failed, the change does not call for this judge, or no frames were captured)",
            report.display(),
            judge.stage()
        ))
    })?;
    std::fs::write(report, filled)?;
    Ok(verdict.passes())
}

/// The report with `judge`'s verdict rendered over its placeholder, or `None`
/// when the report has no placeholder for it.
fn fill(text: &str, judge: Judge, verdict: &Verdict) -> Option<String> {
    let marker = marker(judge);
    let start = text.find(&marker)?;
    // The placeholder paragraph the marker introduces runs to the next
    // `</p>`; everything after that is the page's own closing tags.
    let end = text[start..]
        .find("</p>")
        .map(|i| start + i + 4)
        .unwrap_or(start + marker.len());

    let count = |s: &str| verdict.findings.iter().filter(|f| severity(f) == s).count();
    let (major, minor) = (count("major"), count("minor"));
    let (class, word) = if verdict.passes() {
        ("ok", "passes")
    } else {
        ("bad", "does not pass")
    };

    let mut out = String::new();
    out.push_str(&format!(
        "<p><strong>Iteration {}.</strong> {major} major, {minor} minor &mdash; \
         stage&nbsp;{} <strong class=\"{class}\">{word}</strong>.</p>",
        verdict.iteration,
        judge.stage()
    ));

    if verdict.findings.is_empty() {
        out.push_str("<p>No findings.</p>");
    } else {
        out.push_str("<table><tr><th>severity</th><th>source</th><th>expected</th><th>found</th><th>at</th></tr>");
        for f in &verdict.findings {
            let class = match severity(f) {
                "minor" => "",
                _ => "bad",
            };
            out.push_str(&format!(
                "<tr><td class=\"{class}\">{}</td><td class=\"note\">{}</td><td>{}</td><td>{}</td><td class=\"note\">{}</td></tr>",
                esc(&f.severity),
                esc(&f.source),
                esc(&f.expected),
                esc(&f.found),
                esc(&f.at)
            ));
        }
        out.push_str("</table>");
    }

    let mut section = |title: &str, items: &[String]| {
        if items.is_empty() {
            return;
        }
        out.push_str(&format!("<h3>{title}</h3><ul>"));
        for item in items {
            out.push_str(&format!("<li>{}</li>", esc(item)));
        }
        out.push_str("</ul>");
    };
    section(
        "Contradictions — a source against itself, not failing",
        &verdict.contradictions,
    );
    section("Questions — unresolved, not failing", &verdict.questions);
    section("Confirmed", &verdict.matches);

    Some(format!("{}{out}{}", &text[..start], &text[end..]))
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

    /// A clippy diagnostic is full of angle brackets — `Vec<String>`, `-->`,
    /// `&str`. One unescaped `<` swallows the rest of the page.
    #[test]
    fn tool_output_is_escaped_into_the_page() {
        let escaped = esc("expected `Vec<String>` & found `&str` --> src/x.rs");
        assert!(!escaped.contains('<') && !escaped.contains('>'));
        assert!(escaped.contains("&lt;String&gt;") && escaped.contains("&amp;"));
    }

    fn outcome(passed: bool) -> Outcome {
        Outcome {
            stage: "3 test",
            passed,
            detail: String::new(),
        }
    }

    fn assignments(required: &[Judge]) -> Vec<Assignment> {
        Judge::ALL
            .into_iter()
            .map(|judge| Assignment::new(judge, required.contains(&judge), "because"))
            .collect()
    }

    fn run<'a>(
        outcomes: &'a [Outcome],
        assignments: &'a [Assignment],
        frames: Option<&'a Path>,
        captured: usize,
    ) -> Run<'a> {
        Run {
            commit: "abc1234",
            tree: "tree",
            outcomes,
            assignments,
            frames,
            captured,
        }
    }

    fn finding(severity: &str) -> Finding {
        Finding {
            severity: severity.into(),
            source: String::new(),
            expected: String::new(),
            found: String::new(),
            at: String::new(),
        }
    }

    fn judged(findings: Vec<Finding>) -> Verdict {
        Verdict {
            iteration: 1,
            findings,
            matches: vec![],
            contradictions: vec![],
            questions: vec![],
        }
    }

    /// A clean run leaves each required judge a placeholder, written once.
    #[test]
    fn a_clean_run_leaves_each_required_judge_its_placeholder() {
        let passed = [outcome(true)];
        let all = assignments(&Judge::ALL);
        let page = render(&run(&passed, &all, Some(Path::new("frames")), 72));
        for judge in Judge::ALL {
            let filled = fill(&page, judge, &judged(vec![])).expect("a placeholder");
            assert!(!filled.contains(&marker(judge)), "filled once, not twice");
        }
    }

    /// The false pass this closes: a judge's pass written over a run whose
    /// suite failed, over a judge the change never called for, or over a
    /// frames judge with nothing captured to look at.
    #[test]
    fn a_failing_unrequired_or_frameless_judge_has_nowhere_to_be_written() {
        let failed = [outcome(true), outcome(false)];
        let passed = [outcome(true)];
        let all = assignments(&Judge::ALL);
        let code_only = assignments(&[Judge::Code]);

        let page = render(&run(&failed, &all, Some(Path::new("frames")), 72));
        assert!(Judge::ALL
            .into_iter()
            .all(|j| fill(&page, j, &judged(vec![])).is_none()));

        let page = render(&run(&passed, &code_only, None, 0));
        assert!(fill(&page, Judge::Code, &judged(vec![])).is_some());
        assert!(fill(&page, Judge::Rust, &judged(vec![])).is_none());

        let page = render(&run(&passed, &all, None, 0));
        assert!(fill(&page, Judge::Frames, &judged(vec![])).is_none());
    }

    /// Any finding fails the stage; only its absence passes.
    #[test]
    fn a_verdict_passes_only_with_no_findings() {
        assert!(judged(vec![]).passes());
        assert!(!judged(vec![finding("minor")]).passes());
        assert!(!judged(vec![finding("major")]).passes());
    }

    /// Severity is read once, case-insensitively, and anything unrecognised
    /// is a minor.
    #[test]
    fn severity_is_read_the_same_way_for_every_count() {
        assert_eq!(severity(&finding(" Major ")), "major");
        assert_eq!(severity(&finding("nit")), "minor");

        let passed = [outcome(true)];
        let code = assignments(&[Judge::Code]);
        let page = render(&run(&passed, &code, None, 0));
        let filled = fill(
            &page,
            Judge::Code,
            &judged(vec![finding("Major"), finding("Minor")]),
        )
        .unwrap();
        assert!(filled.contains("1 major, 1 minor"), "{filled}");
        assert!(filled.contains("does not pass"), "{filled}");
        assert!(filled.contains("<td class=\"bad\">Major</td>"), "{filled}");
    }
}

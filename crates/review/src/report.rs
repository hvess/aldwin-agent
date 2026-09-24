//! The run's record — `review.html`, written into the frames directory.
//!
//! Small on purpose, and self-contained: no external stylesheet, no script,
//! no embedded frames. What it holds is what a reader needs tomorrow or on a
//! pull request — which stages ran, what they measured, and what the judge
//! concluded. The frames sit next to it on disk.
//!
//! **The page is plain HTML and stays that way.** Dressing the referee in the
//! players' kit makes it harder to trust, so the design system this loop
//! enforces is deliberately not applied here. That is a rule, not an
//! omission.
//!
//! The other rule worth defending: **this file writes only what was
//! measured.** The judge's score and findings are appended by the skill,
//! into the placeholder left for them. The agent that made the change is the
//! one that would otherwise write the verdict sentence, and "close, two
//! stages clean" is nothing false and much less useful than the numbers.

use std::io::Result;
use std::path::Path;

use crate::stages::Outcome;

/// Where the skill's section goes. Left as a comment so a reader of the raw
/// file can see the seam, and so an append is a string replace rather than a
/// parse.
pub const STAGE5_MARKER: &str = "<!-- stage-5 -->";

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

pub struct Run<'a> {
    pub goal:     &'a str,
    pub focus:    &'a str,
    pub commit:   &'a str,
    pub outcomes: &'a [Outcome],
    pub frames:   Option<&'a Path>,
    pub captured: usize,
}

/// Text into HTML text. Everything user- or tool-supplied goes through this:
/// a clippy diagnostic is full of `&`, `<` and `>`, and one unescaped `<`
/// silently swallows the rest of a cell.
fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

pub fn write(dir: &Path, run: &Run) -> Result<std::path::PathBuf> {
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
        "<dl><dt>Goal</dt><dd>{}</dd><dt>Focus</dt><dd>{}</dd><dt>Commit</dt><dd><code>{}</code></dd></dl>",
        esc(run.goal),
        esc(run.focus),
        esc(run.commit)
    ));

    out.push_str("<h2>Stages 0&ndash;4 &middot; deterministic</h2>");
    out.push_str("<table><tr><th>stage</th><th>result</th><th>measured</th></tr>");
    for outcome in run.outcomes {
        let (class, word) = if outcome.passed { ("ok", "ok") } else { ("bad", "FAIL") };
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
    out.push_str(&format!("<p class=\"count\">{passed} of {} stages passed.</p>", run.outcomes.len()));

    if !failures.is_empty() {
        out.push_str("<h3>Issues</h3>");
        for outcome in failures {
            out.push_str(&format!("<p><strong>{}</strong></p><pre>{}</pre>", esc(outcome.stage), esc(outcome.detail.trim_end())));
        }
    }

    match run.frames {
        Some(path) => out.push_str(&format!(
            "<p class=\"note\">{} frames captured, in <code>{}</code>.</p>",
            run.captured,
            esc(&path.display().to_string())
        )),
        None => out.push_str("<p class=\"note\">No frames captured (<code>--no-capture</code>); stage&nbsp;5 needs them.</p>"),
    }

    out.push_str("<h2>Stage 5 &middot; confidence</h2>");
    out.push_str(STAGE5_MARKER);
    out.push_str(
        "<p class=\"note\">Appended by the review skill once the judge has run. \
         Until then this run is incomplete: stages 0&ndash;4 say nothing about whether \
         the change matches its design.</p>",
    );

    out.push_str("</main></body></html>\n");

    let path = dir.join("review.html");
    std::fs::write(&path, out)?;
    Ok(path)
}

/// One finding, as the judge reported it.
#[derive(serde::Deserialize)]
pub struct Finding {
    pub severity: String,
    /// Which of the prompt's five ranked sources this is measured against.
    /// Optional so a hand-written file stays valid, but the judge is asked
    /// for it: a finding that cannot name its source is not a finding.
    #[serde(default)]
    pub source:   String,
    pub design:   String,
    pub frame:    String,
    pub frames:   String,
}

/// What the skill hands back from stage 5.
///
/// Three lists, and only the first scores. `contradictions` is the design
/// disagreeing with itself and `questions` is the judge saying it could not
/// tell — both are worth recording and neither is the app's fault, so
/// neither deducts. Keeping them out of the arithmetic is what stops a
/// judge's uncertainty from reading as a defect.
#[derive(serde::Deserialize)]
pub struct Stage5 {
    pub iteration:      u32,
    pub findings:       Vec<Finding>,
    /// Anything the judge confirmed matches, one line each.
    #[serde(default)]
    pub matches:        Vec<String>,
    /// Places the design disagrees with itself. Candidates for
    /// `baseline.json`; they do not score.
    #[serde(default)]
    pub contradictions: Vec<String>,
    /// What the judge could not resolve. They do not score either.
    #[serde(default)]
    pub questions:      Vec<String>,
}

/// The score stage 5 must reach. 100 means no finding of any severity: a
/// minor deviation from a frame fails the review as a major one does, only
/// by less. It was 90 (two minors) until 2026-09-24, when the developer
/// raised it to 95 and then to 100 after a whole-app pass.
pub const THRESHOLD: u32 = 100;

/// The score, derived from severities rather than chosen by the judge.
///
/// A model picking "80" cannot say what makes it 80 rather than 70. This
/// can, and it makes [`THRESHOLD`] mean something concrete.
pub fn score(findings: &[Finding]) -> u32 {
    let deducted: u32 = findings
        .iter()
        .map(|f| match severity(f) {
            "blocking" => 25,
            "major" => 15,
            _ => 5,
        })
        .sum();
    100u32.saturating_sub(deducted)
}

/// A finding's severity as the score reads it: `blocking` or `major`, and
/// anything else — `minor`, a misspelling, a `Minor` — is a minor. The
/// score and the report's counts both go through this, so a finding can
/// never be deducted without being counted.
fn severity(f: &Finding) -> &'static str {
    match f.severity.trim().to_ascii_lowercase().as_str() {
        "blocking" => "blocking",
        "major" => "major",
        _ => "minor",
    }
}

/// Render stage 5 into the report, replacing the placeholder.
///
/// A command rather than an instruction to edit HTML by hand. The first
/// version of this loop left the append to the skill's discipline and the
/// section came back empty on three consecutive runs — the agent was busy
/// reading findings and fixing code, which is exactly when a manual step
/// gets skipped.
pub fn write_stage5(report: &Path, stage5: &Stage5) -> Result<u32> {
    let text = std::fs::read_to_string(report)?;
    let start = text.find(STAGE5_MARKER).ok_or_else(|| {
        std::io::Error::other(format!("{} has no {STAGE5_MARKER} — already filled in?", report.display()))
    })?;
    // The placeholder paragraph the marker introduces runs to the next
    // `</p>`; everything after that is the page's own closing tags.
    let end = text[start..].find("</p>").map(|i| start + i + 4).unwrap_or(start + STAGE5_MARKER.len());

    let value = score(&stage5.findings);
    let counts = |s: &str| stage5.findings.iter().filter(|f| severity(f) == s).count();
    let (blocking, major, minor) = (counts("blocking"), counts("major"), counts("minor"));
    let verdict = if value >= THRESHOLD { ("ok", "passes") } else { ("bad", "does not pass") };

    let mut out = String::new();
    out.push_str(&format!("<p><strong>Iteration {}.</strong></p>", stage5.iteration));
    out.push_str(&format!(
        "<p class=\"count\">Score <strong class=\"{}\">{value}</strong> — \
         100 &minus; (25 &times; {blocking} blocking) &minus; (15 &times; {major} major) &minus; (5 &times; {minor} minor). \
         Threshold is {THRESHOLD}, so stage&nbsp;5 <strong>{}</strong>.</p>",
        verdict.0, verdict.1
    ));

    if stage5.findings.is_empty() {
        out.push_str("<p>No findings.</p>");
    } else {
        out.push_str("<table><tr><th>severity</th><th>source</th><th>design</th><th>frame</th><th>frames</th></tr>");
        for f in &stage5.findings {
            let class = match f.severity.trim() {
                "blocking" | "major" => "bad",
                _ => "",
            };
            out.push_str(&format!(
                "<tr><td class=\"{class}\">{}</td><td class=\"note\">{}</td><td>{}</td><td>{}</td><td class=\"note\">{}</td></tr>",
                esc(&f.severity),
                esc(&f.source),
                esc(&f.design),
                esc(&f.frame),
                esc(&f.frames)
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
    section("Contradictions — the design against itself, not scored", &stage5.contradictions);
    section("Questions — unresolved, not scored", &stage5.questions);
    section("Confirmed matching", &stage5.matches);

    std::fs::write(report, format!("{}{out}{}", &text[..start], &text[end..]))?;
    Ok(value)
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
}

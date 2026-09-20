//! The report a human reads.
//!
//! The split is deliberate and it is the point of the file: **the crate fills
//! in everything measured, and the agent supplies narrative as data.** The
//! verdict, the gate results, the scores and the frames are assembled from
//! what is already in the run directory. "What I changed and why" arrives as a
//! field in `session.json` and lands in its own section.
//!
//! The risk that arrangement guards against is not fabrication, it is
//! emphasis: the agent that wrote the code is the one that would otherwise
//! write the sentence at the top, and a capped run at 71 gets described as
//! "close, two gates clean". Nothing false, and much less useful than the
//! number.
//!
//! The page is plain HTML and stays that way. Dressing the referee in the
//! players' kit makes it harder to trust, so the design system is not applied
//! here — that is a rule, not an omission.

use std::io::Result;
use std::path::Path;

use crate::gates::Report as GateReport;

/// The gate report as it is read back from a run directory.
pub type GateReportShape = GateReport;
use crate::session::Session;

pub const ITERATION_CAP: u32 = 5;
/// Kept for the advisory line the report still prints, and for nothing else.
/// See [`verdict`] for why it is no longer a gate.
pub const THRESHOLD: u32 = 90;

pub struct Frame {
    pub scene:      String,
    pub size:       String,
    pub theme:      String,
    pub png:        std::path::PathBuf,
    pub annotated:  Option<std::path::PathBuf>,
    pub gates:      GateReport,
    pub expect:     crate::expect::Outcome,
}

/// The exit condition, computed rather than narrated.
///
/// `preflight clean ∧ regression in focus ∧ no gate violation ∧ no failed
/// assertion ∧ iterations ≤ cap` — a conjunction, not a number.
///
/// # Why the judge's score is no longer in it
///
/// It was, for five runs, as `min score ≥ 90` — and the loop never once
/// exited. The reason is not that the TUI was that far off; it is that the
/// quantity was never calibrated. A fresh blind model with no anchors gave
/// `empty` 64 and `first_run` 86 on substantially the same finding set
/// (`run-1789850385`), and on `run-1789829088` the means *rose* while the
/// minimum fell 58 → 48. Taking the minimum across 72 frames of that is
/// gating on the harshest reading of the harshest judge, and the spec's own
/// Progress entry called it "the noisiest statistic available" while
/// continuing to gate on it.
///
/// What replaces it is [`crate::expect`]: the design's stated geometry, as
/// assertions that cite the `HANDOFF.md` line they come from. Reproducible
/// bit-for-bit, comparable between runs, and a failure arrives already
/// knowing what it violated.
///
/// The judge stays, and its scores are still recorded and still printed —
/// but as **advisory**. An assertion suite only checks what somebody wrote
/// down; the judge is what finds the deviation nobody enumerated, and on
/// `run-1789850385` that was two of the twelve new findings. Demoting it is
/// not distrusting it. It is refusing to let an uncalibrated number decide a
/// yes/no question.
pub fn verdict(session: &Session, frames: &[Frame]) -> (String, bool) {
    let violations: usize = frames.iter().map(|f| f.gates.violations.len()).sum();
    let failures: usize = frames.iter().map(|f| f.expect.failures.len()).sum();
    let capped = session.iterations.len() as u32 >= ITERATION_CAP;

    match &session.regression {
        None => return ("Not scored — the regression gate has not run for this session".to_string(), false),
        Some(r) if !r.unaccounted.is_empty() => {
            return (
                format!(
                    "Failed — {} snapshot sections moved outside the focus set, first {}",
                    r.unaccounted.len(),
                    r.unaccounted[0]
                ),
                false,
            )
        }
        Some(_) => {}
    }

    if violations > 0 {
        let first = frames.iter().flat_map(|f| f.gates.violations.iter()).next().expect("counted above");
        return (
            format!("Failed — {violations} gate violations, first at {} ({}, {})", first.gate, first.row, first.col),
            false,
        );
    }
    if failures > 0 {
        let first = frames.iter().flat_map(|f| f.expect.failures.iter()).next().expect("counted above");
        return (
            format!(
                "Failed — {failures} design assertions, first: {} expects {} ({})",
                first.screen, first.rule, first.cite
            ),
            false,
        );
    }

    let checked: usize = frames.iter().map(|f| f.expect.checked).sum();
    if checked == 0 {
        return ("Not scored — no design assertions ran, so nothing was checked against the reference".to_string(), false);
    }

    // The advisory half. It cannot fail the run; it is here so a reader sees
    // what the judge thought next to what the reference proved.
    let advisory = match session.minimum() {
        Some(min) if min >= THRESHOLD => format!("; judge minimum {min}"),
        Some(min) => format!("; judge minimum {min}, advisory only (threshold {THRESHOLD})"),
        None => "; no judge scores recorded".to_string(),
    };

    if capped {
        return (format!("Capped at {ITERATION_CAP} iterations — {checked} assertions clean{advisory}"), false);
    }
    (format!("Passed — no gate violations, {checked} design assertions clean{advisory}"), true)
}

pub fn write(run: &Path, session: &Session, frames: &[Frame]) -> Result<std::path::PathBuf> {
    let template = include_str!("../report.html");
    let (verdict, passed) = verdict(session, frames);
    let class = if passed { "pass" } else { "fail" };

    let page = template
        .replace("{{TITLE}}", "Screenshot session")
        .replace("{{GOAL}}", &escape(&session.goal))
        .replace("{{VERDICT}}", &escape(&verdict))
        .replace("{{VERDICT_CLASS}}", class)
        .replace("{{FOCUS}}", &escape(&session.focus))
        .replace("{{BASELINE}}", &escape(&session.baseline))
        .replace("{{FRAME_COUNT}}", &frames.len().to_string())
        .replace(
            "{{ITERATION_COUNT}}",
            &match session.iterations.len() {
                1 => "1 iteration".to_string(),
                n => format!("{n} iterations"),
            },
        )
        .replace("{{PREFLIGHT}}", &preflight_section(frames))
        .replace("{{GATES}}", &format!("{}{}", gates_section(frames), regression_section(session)))
        .replace("{{CONFORMANCE}}", &conformance_section(frames))
        .replace("{{SCORES}}", &scores_section(session))
        .replace("{{FRAMES}}", &frames_section(frames)?)
        .replace("{{ITERATIONS}}", &iterations_section(session));

    let path = run.join("report.html");
    std::fs::write(&path, page)?;
    Ok(path)
}

fn preflight_section(frames: &[Frame]) -> String {
    let checked: usize = frames.len();
    format!(
        "<p>Cleared — every frame came back at its exact geometry, with the parser and the picture reconciled.</p>\
         <p class=\"note\">{checked} captures; a run that reaches a report has cleared preflight by construction, \
         because a failure there stops the session before anything is scored.</p>"
    )
}

fn gates_section(frames: &[Frame]) -> String {
    let mut rows = String::new();
    let mut applied: Vec<&str> = Vec::new();
    for frame in frames {
        for violation in &frame.gates.violations {
            rows.push_str(&format!(
                "<tr><td class=\"mono\">{} {} {}</td><td>{}</td><td class=\"mono\">{},{}</td><td>{}</td></tr>",
                escape(&frame.scene),
                escape(&frame.size),
                escape(&frame.theme),
                escape(&violation.gate),
                violation.row,
                violation.col,
                escape(&violation.detail)
            ));
        }
        for note in &frame.gates.applied {
            if !applied.contains(&note.as_str()) {
                applied.push(note);
            }
        }
    }

    let table = if rows.is_empty() {
        "<p>The five per-frame gates are clean on every frame. The sixth, regression, is below.</p>".to_string()
    } else {
        format!("<table><tr><th>frame</th><th>gate</th><th>cell</th><th>detail</th></tr>{rows}</table>")
    };

    let exemptions = if applied.is_empty() {
        String::new()
    } else {
        format!(
            "<h3 class=\"note\">Exceptions applied</h3><ul class=\"note\">{}</ul>",
            applied.iter().map(|a| format!("<li>{}</li>", escape(a))).collect::<String>()
        )
    };
    format!("{table}{exemptions}")
}

/// The regression gate reports once per run, not per frame, so it gets its own
/// paragraph rather than a row in the table above.
fn regression_section(session: &Session) -> String {
    match &session.regression {
        None => "<p><strong>regression — not run.</strong> The verdict cannot pass without it.</p>".into(),
        Some(r) if r.unaccounted.is_empty() => format!(
            "<p><strong>regression — clean.</strong> {} snapshot sections moved against <span class=\"mono\">{}</span>, all inside the focus set.</p>",
            r.moved,
            escape(&r.baseline)
        ),
        Some(r) => format!(
            "<p><strong>regression — failed.</strong> {} of {} moved sections are outside the focus set:</p><ul>{}</ul>",
            r.unaccounted.len(),
            r.moved,
            r.unaccounted.iter().map(|s| format!("<li class=\"mono\">{}</li>", escape(s))).collect::<String>()
        ),
    }
}

/// The deterministic half of the report, and the half the verdict is
/// computed from.
fn conformance_section(frames: &[Frame]) -> String {
    let checked: usize = frames.iter().map(|f| f.expect.checked).sum();
    let failures: Vec<(&Frame, &crate::expect::Failure)> =
        frames.iter().flat_map(|f| f.expect.failures.iter().map(move |v| (f, v))).collect();

    if checked == 0 {
        return "<p>No assertions ran. Nothing was checked against the design reference, so a clean gate result here means only that the frames are well-formed.</p>".into();
    }
    if failures.is_empty() {
        return format!(
            "<p><strong>{checked} assertions clean.</strong> Every position, tone and \
             row count the design system states for these screens holds in every frame.</p>"
        );
    }
    let rows: String = failures
        .iter()
        .map(|(frame, failure)| {
            format!(
                "<tr><td class=\"mono\">{}-{}-{}{}</td><td>{}</td><td>{}</td><td class=\"note\">{}</td></tr>",
                escape(&frame.scene),
                escape(&frame.size),
                escape(&frame.theme),
                failure.row.map(|r| format!(" r{r}")).unwrap_or_default(),
                escape(&failure.screen),
                escape(&failure.detail),
                escape(&failure.cite),
            )
        })
        .collect();
    format!(
        "<table><tr><th>frame</th><th>screen</th><th>measured</th><th>stated</th></tr>{rows}</table>\
         <p class=\"note\"><strong>{} failed</strong> of {checked} checked. Each row cites the \
         <code>HANDOFF.md</code> line it comes from; none of it is a judgement. This is what the \
         verdict is computed from.</p>",
        failures.len()
    )
}

fn scores_section(session: &Session) -> String {
    if session.scores.is_empty() {
        return "<p>No scores recorded. The judge writes these into <code>session.json</code>.</p>".into();
    }
    let rows: String = session
        .scores
        .iter()
        .map(|s| {
            format!(
                "<tr><td class=\"mono\">{} {} {}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                escape(&s.scene),
                escape(&s.size),
                escape(&s.theme),
                s.spatial,
                s.component,
                escape(&s.note)
            )
        })
        .collect();
    let minimum = session.minimum().unwrap_or(0);
    format!(
        "<table><tr><th>frame</th><th>spatial</th><th>component</th><th>note</th></tr>{rows}</table>\
         <p class=\"note\">Minimum across every frame: <strong>{minimum}</strong>, against a reference point of {THRESHOLD}. \
         <strong>Advisory.</strong> These numbers no longer gate the run and have not since the assertion suite \
         replaced them: a 0–100 from a fresh blind model with no anchors is uncalibrated, and taking the minimum \
         across every frame gates on the harshest reading of the harshest judge. What the judge is for is the \
         deviation nobody thought to write an assertion about; that is why its findings still matter and its \
         arithmetic does not.</p>"
    )
}

fn frames_section(frames: &[Frame]) -> Result<String> {
    let mut out = String::new();
    for frame in frames {
        let source = frame.annotated.as_ref().unwrap_or(&frame.png);
        let marked = if frame.annotated.is_some() { " · violations outlined in magenta" } else { "" };
        out.push_str(&format!(
            "<figure><figcaption>{} · {} · {}{}</figcaption><img alt=\"{} {} {}\" src=\"data:image/png;base64,{}\"></figure>",
            escape(&frame.scene),
            escape(&frame.size),
            escape(&frame.theme),
            marked,
            escape(&frame.scene),
            escape(&frame.size),
            escape(&frame.theme),
            base64(&std::fs::read(source)?)
        ));
    }
    Ok(out)
}

fn iterations_section(session: &Session) -> String {
    if session.iterations.is_empty() {
        return "<p class=\"note\">Nothing recorded.</p>".into();
    }
    format!(
        "<ul>{}</ul>",
        session
            .iterations
            .iter()
            .map(|i| format!("<li><strong>{}</strong> — {}</li>", i.n, escape(&i.changed)))
            .collect::<String>()
    )
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Frames are embedded rather than linked, so the report outlives the run
/// directory it was generated in — which is deleted when a human accepts it.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { ALPHABET[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { ALPHABET[n as usize & 63] as char } else { '=' });
    }
    out
}

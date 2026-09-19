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
pub const THRESHOLD: u32 = 90;

pub struct Frame {
    pub scene:      String,
    pub size:       String,
    pub theme:      String,
    pub png:        std::path::PathBuf,
    pub annotated:  Option<std::path::PathBuf>,
    pub gates:      GateReport,
}

/// The exit condition, computed rather than narrated.
///
/// `preflight clean ∧ no gate violation ∧ min score ≥ 90 ∧ iterations ≤ cap` —
/// a conjunction, not a number. It is also what keeps self-scoring honest: an
/// agent can rationalise a score, but it cannot exit on the score alone.
pub fn verdict(session: &Session, frames: &[Frame]) -> (String, bool) {
    let violations: usize = frames.iter().map(|f| f.gates.violations.len()).sum();
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
    match session.minimum() {
        Some(min) if min >= THRESHOLD => (format!("Passed — no gate violations, minimum score {min}"), true),
        Some(min) if capped => (
            format!("Capped at {ITERATION_CAP} iterations — minimum score {min}, below the threshold of {THRESHOLD}"),
            false,
        ),
        Some(min) => (format!("Below threshold — minimum score {min}, needs {THRESHOLD}"), false),
        None => ("Not scored — gates are clean, but no judge scores were recorded".to_string(), false),
    }
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
         <p class=\"note\">Minimum across every frame: <strong>{minimum}</strong>, against a threshold of {THRESHOLD}. \
         The minimum, never the mean — an average of {THRESHOLD} can hide one frame at forty.</p>"
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

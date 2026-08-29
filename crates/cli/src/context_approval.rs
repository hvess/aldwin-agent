use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use mjolnir_permissions::{CheckOutcome, ContextFileTier, Engine};

const CANDIDATE_FILENAMES: &[&str] = &["CLAUDE.md", "AGENTS.md"];

/// Finds CLAUDE.md/AGENTS.md in `project_root` and, for any not already
/// approved, asks the developer — synchronously, over stdin/stdout, since
/// there is no TUI yet at this point in the startup sequence. Matches
/// mjolnir-permissions.md: "The session initializer (cli crate) tests each
/// candidate file before composing the additional-context string." Returns
/// every path now approved, at any scope (project-persisted or
/// session-only) — `Engine::effective_view` is the one place both live.
pub fn resolve(project_root: &Path, engine: &Engine) -> Vec<PathBuf> {
    let candidates: Vec<PathBuf> = CANDIDATE_FILENAMES.iter().map(|f| project_root.join(f)).filter(|p| p.is_file()).collect();
    resolve_with_io(&candidates, engine, &mut std::io::stdin().lock(), &mut std::io::stdout())
}

fn resolve_with_io(candidates: &[PathBuf], engine: &Engine, input: &mut impl BufRead, output: &mut impl Write) -> Vec<PathBuf> {
    for candidate in candidates {
        if let CheckOutcome::PromptRequired(_) = engine.check_context_file(candidate) {
            prompt_and_record(candidate, engine, input, output);
        }
    }
    engine.effective_view().context_files.into_iter().map(|f| f.path).collect()
}

/// Two-tier prompt per mjolnir-permissions.md: persist project / just this
/// session / decline (no "once", no global — see that spec's Decisions on
/// why). A closed/EOF stdin (non-interactive invocation) declines rather
/// than hanging forever waiting for an answer that can't come.
fn prompt_and_record(path: &Path, engine: &Engine, input: &mut impl BufRead, output: &mut impl Write) {
    loop {
        let _ = write!(output, "Include {} in this session's context? [p]roject / [s]ession / [n]o: ", path.display());
        let _ = output.flush();

        let mut line = String::new();
        if input.read_line(&mut line).unwrap_or(0) == 0 {
            let _ = engine.record_context_file_decision(path, false, None);
            return;
        }

        match line.trim().chars().next().map(|c| c.to_ascii_lowercase()) {
            Some('p') => {
                let _ = engine.record_context_file_decision(path, true, Some(ContextFileTier::Project));
                return;
            }
            Some('s') => {
                let _ = engine.record_context_file_decision(path, true, Some(ContextFileTier::Session));
                return;
            }
            Some('n') => {
                let _ = engine.record_context_file_decision(path, false, None);
                return;
            }
            _ => {
                let _ = writeln!(output, "please answer p, s, or n");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mjolnir_config::Config;
    use std::io::Cursor;

    fn engine() -> (tempfile::TempDir, tempfile::TempDir, Engine) {
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let config = Config::open_at(project.path(), global.path()).unwrap();
        (project, global, Engine::new(config))
    }

    #[test]
    fn already_approved_files_are_not_prompted_for_again() {
        let (project, _global, engine) = engine();
        let path = project.path().join("CLAUDE.md");
        std::fs::write(&path, "notes").unwrap();
        engine.record_context_file_decision(&path, true, Some(ContextFileTier::Project)).unwrap();

        let mut input = Cursor::new(Vec::new()); // nothing to read from
        let mut output = Vec::new();
        let approved = resolve_with_io(std::slice::from_ref(&path), &engine, &mut input, &mut output);

        assert!(output.is_empty(), "must not prompt for an already-approved file");
        assert_eq!(approved, vec![path]);
    }

    #[test]
    fn answering_p_approves_at_project_scope() {
        let (project, _global, engine) = engine();
        let path = project.path().join("CLAUDE.md");
        std::fs::write(&path, "notes").unwrap();

        let mut input = Cursor::new(b"p\n".to_vec());
        let mut output = Vec::new();
        let approved = resolve_with_io(std::slice::from_ref(&path), &engine, &mut input, &mut output);

        assert_eq!(approved, vec![path.clone()]);
        assert!(engine.effective_view().context_files.iter().any(|f| f.path == path && f.scope == mjolnir_permissions::GrantScope::Project));
    }

    #[test]
    fn answering_s_approves_session_only_and_does_not_persist() {
        let (project, global, engine) = engine();
        let path = project.path().join("CLAUDE.md");
        std::fs::write(&path, "notes").unwrap();

        let mut input = Cursor::new(b"s\n".to_vec());
        let mut output = Vec::new();
        let approved = resolve_with_io(std::slice::from_ref(&path), &engine, &mut input, &mut output);

        assert_eq!(approved, vec![path.clone()]);
        // Reopening the config must not see a session-only approval.
        let reopened = Config::open_at(project.path(), global.path()).unwrap();
        assert!(!reopened.project_context_files().approved.contains(&path));
    }

    #[test]
    fn answering_n_declines_and_the_file_is_excluded() {
        let (project, _global, engine) = engine();
        let path = project.path().join("CLAUDE.md");
        std::fs::write(&path, "notes").unwrap();

        let mut input = Cursor::new(b"n\n".to_vec());
        let mut output = Vec::new();
        let approved = resolve_with_io(std::slice::from_ref(&path), &engine, &mut input, &mut output);

        assert!(approved.is_empty());
    }

    #[test]
    fn an_invalid_answer_reprompts_before_accepting_a_valid_one() {
        let (project, _global, engine) = engine();
        let path = project.path().join("CLAUDE.md");
        std::fs::write(&path, "notes").unwrap();

        let mut input = Cursor::new(b"bogus\np\n".to_vec());
        let mut output = Vec::new();
        let approved = resolve_with_io(std::slice::from_ref(&path), &engine, &mut input, &mut output);

        assert_eq!(approved, vec![path]);
        assert!(String::from_utf8_lossy(&output).contains("please answer"));
    }

    #[test]
    fn closed_stdin_declines_instead_of_hanging() {
        let (project, _global, engine) = engine();
        let path = project.path().join("CLAUDE.md");
        std::fs::write(&path, "notes").unwrap();

        let mut input = Cursor::new(Vec::new()); // immediate EOF
        let mut output = Vec::new();
        let approved = resolve_with_io(&[path], &engine, &mut input, &mut output);

        assert!(approved.is_empty());
    }

    #[test]
    fn resolve_filters_candidates_to_files_that_actually_exist() {
        let (project, _global, engine) = engine();
        std::fs::write(project.path().join("CLAUDE.md"), "notes").unwrap();
        // AGENTS.md deliberately not created — resolve() must not even
        // consider it (and so never prompts for it, which matters here:
        // there's no stdin available in a test process to answer such a
        // prompt, so a bug that tried would hang or panic). Approve
        // CLAUDE.md up front so resolve() has nothing left to ask either.
        engine.record_context_file_decision(&project.path().join("CLAUDE.md"), true, Some(ContextFileTier::Session)).unwrap();

        let approved = resolve(project.path(), &engine);
        assert_eq!(approved, vec![project.path().join("CLAUDE.md")]);
    }
}

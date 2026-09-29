use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Output;

use chrono::NaiveDate;
use serde::Deserialize;

/// The additional-context string handed to aldwin-core: the cwd, the date
/// the session started, any extra workspace roots, the platform facts, the
/// full text of each file in `approved`, in order, then the workspace's
/// skills by name and description. `bootstrap::context_files` chooses the
/// files and `skills` finds the skills.
///
/// The date is stated because the model's own sense of "now" is its training
/// cutoff. The roots are named so the model can act on an "outside the
/// workspace" refusal; the platform facts (bash version, sed flavour) spare it learning
/// them by failing. A skill is listed, not read: the model reads the one a
/// task calls for.
pub fn build(
    cwd: &Path,
    today: NaiveDate,
    roots: &[PathBuf],
    approved: &[PathBuf],
    skills: &[Skill],
) -> String {
    let mut sections = vec![
        format!("Working directory: {}", cwd.display()),
        format!("Today's date: {}", today.format("%Y-%m-%d")),
    ];

    if roots.len() > 1 {
        let extra: Vec<String> = roots
            .iter()
            .skip(1)
            .map(|r| r.display().to_string())
            .collect();
        sections.push(format!(
            "Also in the workspace (declared in .aldwin/permissions.yaml): {}\nEvery tool refuses a path outside the workspace, and a command you run cannot write outside it.",
            extra.join(", ")
        ));
    }

    sections.push(platform_facts(roots));

    for path in approved {
        // A file removed since the caller found it is skipped, not an error.
        if let Ok(contents) = std::fs::read_to_string(path) {
            sections.push(format!("--- {} ---\n{}", path.display(), contents));
        }
    }

    if !skills.is_empty() {
        let rows: Vec<String> = skills
            .iter()
            .map(|s| format!("- {} ({}): {}", s.name, s.path.display(), s.description))
            .collect();
        sections.push(format!(
            "Skills in this workspace. Before a task one describes, read its file with `read` and follow it:\n{}",
            rows.join("\n")
        ));
    }

    sections.join("\n\n")
}

/// A `SKILL.md` the workspace offers, by its front matter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Skill {
    /// The front matter's `name`, else the directory's.
    pub(crate) name: String,
    /// The front matter's `description`, on one line.
    pub(crate) description: String,
    /// The file, relative to the project root, for the model to read.
    pub(crate) path: PathBuf,
}

/// Where a project keeps its skills, each `<dir>/<name>/SKILL.md`. The
/// vendor-neutral location first; a `.claude/skills` linked to it is the
/// same skills once.
const SKILL_DIRS: [&str; 2] = [".agents/skills", ".claude/skills"];

/// Every skill under `SKILL_DIRS` in `cwd`, by name. A directory without a
/// `SKILL.md`, or one with no `description`, is not a skill, and neither is
/// one linked from outside every root in `roots`: `read` refuses it
/// (ADR 0007).
pub(crate) fn skills(cwd: &Path, roots: &[PathBuf]) -> Vec<Skill> {
    let roots: Vec<PathBuf> = roots.iter().filter_map(|r| r.canonicalize().ok()).collect();
    let candidates = SKILL_DIRS.iter().flat_map(|dir| {
        std::fs::read_dir(cwd.join(dir))
            .into_iter()
            .flatten()
            .flatten()
            .map(move |entry| {
                let relative = Path::new(dir).join(entry.file_name()).join("SKILL.md");
                (entry.path().join("SKILL.md"), relative)
            })
    });
    // By canonical path, so a linked directory lists once, at its first
    // location.
    let mut seen = HashSet::new();
    let mut skills: Vec<Skill> = candidates
        .filter(|(file, _)| {
            file.canonicalize()
                .is_ok_and(|c| roots.iter().any(|r| c.starts_with(r)) && seen.insert(c))
        })
        .filter_map(|(file, relative)| skill_from(&file, relative))
        .collect();
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

/// The `---` front matter of a `SKILL.md`, as the Agent Skills convention
/// writes it.
#[derive(Deserialize)]
struct FrontMatter {
    name: Option<String>,
    description: Option<String>,
}

/// The skill `file` describes, from its front matter; `None` without one,
/// or without a description. A folded or literal description is one line.
fn skill_from(file: &Path, relative: PathBuf) -> Option<Skill> {
    let text = std::fs::read_to_string(file).ok()?;
    let (front, _) = text.strip_prefix("---")?.split_once("\n---")?;
    let front: FrontMatter = serde_yaml_ng::from_str(front).ok()?;
    let one_line = |s: String| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let directory = relative
        .parent()
        .and_then(Path::file_name)
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Some(Skill {
        name: front
            .name
            .map(one_line)
            .filter(|n| !n.is_empty())
            .unwrap_or(directory),
        description: front.description.map(one_line).filter(|d| !d.is_empty())?,
        path: relative,
    })
}

/// Facts a shell script must get right on this machine. Detected, never
/// assumed: a wrong fact is worse than none.
fn platform_facts(roots: &[PathBuf]) -> String {
    let mut facts = vec![format!("Platform: {}", std::env::consts::OS)];

    if let Some(version) = program_version("bash", &["--version"], roots) {
        facts.push(format!("bash: {version}"));
    }
    facts.extend(
        probe("sed", &["--version"], roots)
            .and_then(|out| {
                sed_fact(
                    &String::from_utf8_lossy(&out.stdout),
                    &String::from_utf8_lossy(&out.stderr),
                )
            })
            .map(str::to_string),
    );
    facts.join("\n")
}

/// The `sed` fact from what `sed --version` printed. GNU `sed -i` takes no
/// argument, BSD's requires one. Ask the `sed` on PATH, not the OS: a Mac
/// can have GNU sed. Only a flavour's own words are evidence: BSD sed
/// rejects `--version` with its usage line, and a launcher that could not
/// start `sed` (macOS's `sandbox-exec`) also exits non-zero.
fn sed_fact(stdout: &str, stderr: &str) -> Option<&'static str> {
    if stdout.contains("(GNU sed)") {
        Some("sed: GNU (in-place edit is `sed -i`)")
    } else if stderr.contains("usage: sed") {
        Some("sed: BSD (in-place edit is `sed -i ''`, not `sed -i`)")
    } else {
        None
    }
}

/// First line of `<program> <args>`, or `None` if it cannot be run or fails.
fn program_version(program: &str, args: &[&str], roots: &[PathBuf]) -> Option<String> {
    let out = probe(program, args, roots).filter(|out| out.status.success())?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
}

/// What `<program> <args>` printed; `None` if it could not be started. Must
/// run in the sandbox, as every process Aldwin starts does (ADR 0011).
fn probe(program: &str, args: &[&str], roots: &[PathBuf]) -> Option<Output> {
    aldwin_tools::sandbox::std_command(program, args, roots)
        .ok()?
        .stdin(std::process::Stdio::null())
        .output()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, 29).expect("a real date")
    }

    #[test]
    fn leads_with_the_cwd_and_carries_no_file_text_when_none_are_approved() {
        let out = build(Path::new("/some/project"), day(), &[], &[], &[]);
        assert!(out.starts_with("Working directory: /some/project"));
        assert!(out.contains("Today's date: 2026-09-29"));
        assert!(out.contains("Platform:"));
        assert!(!out.contains("---"), "no approved file sections");
    }

    /// Regression: the probes ran outside the sandbox (ADR 0011). Only
    /// `sandbox::std_command` sets `AGENT`, so seeing it proves the route.
    #[test]
    fn a_probe_runs_through_the_sandbox() {
        let dir = tempfile::tempdir().unwrap();
        let seen = program_version(
            "sh",
            &["-c", "printf %s \"$AGENT\""],
            &[dir.path().to_path_buf()],
        );
        assert_eq!(seen.as_deref(), Some("aldwin"));
    }

    /// Regression: a `sed` that could not be started was reported as BSD.
    #[test]
    fn a_sed_that_cannot_be_run_is_no_evidence_of_bsd() {
        assert_eq!(
            sed_fact("", "sandbox-exec: execvp() of 'sed' failed: No such file"),
            None
        );
        let bsd = "sed: illegal option -- -\nusage: sed script [-Ealnru] [-i extension]";
        assert!(sed_fact("", bsd).is_some_and(|f| f.contains("BSD")));
        assert!(sed_fact("sed (GNU sed) 4.9\n", "").is_some_and(|f| f.contains("GNU")));
        assert_eq!(
            sed_fact("This is not GNU sed version 4.0\n", ""),
            None,
            "busybox"
        );
    }

    #[test]
    fn states_the_platform_facts_a_shell_script_has_to_be_right_about() {
        let out = build(Path::new("/some/project"), day(), &[], &[], &[]);
        assert!(out.contains(std::env::consts::OS));
        assert!(out.contains("sed:"));
        // The flavour depends on the `sed` on PATH, so only its wording is
        // asserted.
        assert!(
            out.contains("sed -i"),
            "the fact must say how to edit in place: {out}"
        );
    }

    #[test]
    fn names_every_reachable_root_when_more_than_one_is_declared() {
        let out = build(
            Path::new("/some/project"),
            day(),
            &[
                PathBuf::from("/some/project"),
                PathBuf::from("/other/checkout"),
            ],
            &[],
            &[],
        );
        assert!(out.contains("/other/checkout"));
        assert!(out.contains("cannot write outside it"));
    }

    #[test]
    fn a_single_root_adds_no_reachability_section() {
        let out = build(
            Path::new("/some/project"),
            day(),
            &[PathBuf::from("/some/project")],
            &[],
            &[],
        );
        assert!(!out.contains("Also in the workspace"));
    }

    #[test]
    fn includes_the_full_text_of_each_approved_file_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let claude_md = dir.path().join("CLAUDE.md");
        std::fs::write(&claude_md, "# Project notes\nBe careful.").unwrap();

        let out = build(
            dir.path(),
            day(),
            &[],
            std::slice::from_ref(&claude_md),
            &[],
        );
        assert!(out.contains("Working directory:"));
        assert!(out.contains(&claude_md.display().to_string()));
        assert!(out.contains("Be careful."));
    }

    fn write_skill(cwd: &Path, dir: &str, name: &str, front_matter: &str) {
        let skill = cwd.join(dir).join(name);
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(skill.join("SKILL.md"), front_matter).unwrap();
    }

    #[test]
    fn skills_are_listed_by_name_and_description_and_named_after_their_directory_without_one() {
        let dir = tempfile::tempdir().unwrap();
        write_skill(
            dir.path(),
            ".agents/skills",
            "ux",
            "---\nname: ux\ndescription: \"The usability bar.\"\n---\n# UX\n",
        );
        write_skill(
            dir.path(),
            ".agents/skills",
            "comments",
            "---\ndescription: >-\n  How to write\n  a comment.\n---\n",
        );
        write_skill(
            dir.path(),
            ".agents/skills",
            "nameless",
            "# No front matter\n",
        );
        write_skill(
            dir.path(),
            ".agents/skills",
            "mute",
            "---\nname: mute\n---\n",
        );
        std::fs::create_dir_all(dir.path().join(".agents/skills/empty")).unwrap();

        let found = skills(dir.path(), &[dir.path().to_path_buf()]);
        assert_eq!(
            found,
            vec![
                Skill {
                    name: "comments".into(),
                    description: "How to write a comment.".into(),
                    path: PathBuf::from(".agents/skills/comments/SKILL.md"),
                },
                Skill {
                    name: "ux".into(),
                    description: "The usability bar.".into(),
                    path: PathBuf::from(".agents/skills/ux/SKILL.md"),
                },
            ],
            "no front matter, no description and no file are not skills"
        );

        let out = build(dir.path(), day(), &[], &[], &found);
        assert!(out.contains("Skills in this workspace"));
        assert!(out.contains("- ux (.agents/skills/ux/SKILL.md): The usability bar."));
        assert!(
            !build(dir.path(), day(), &[], &[], &[]).contains("Skills"),
            "no section without skills"
        );
    }

    /// `.claude/skills` linked to `.agents/skills` is the same skills once.
    #[cfg(unix)]
    #[test]
    fn a_linked_skills_directory_lists_each_skill_once() {
        let dir = tempfile::tempdir().unwrap();
        write_skill(
            dir.path(),
            ".agents/skills",
            "rust",
            "---\nname: rust\ndescription: Rust rules.\n---\n",
        );
        std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
        std::os::unix::fs::symlink("../.agents/skills", dir.path().join(".claude/skills")).unwrap();
        let found = skills(dir.path(), &[dir.path().to_path_buf()]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, PathBuf::from(".agents/skills/rust/SKILL.md"));

        std::fs::remove_file(dir.path().join(".claude/skills")).unwrap();
        write_skill(
            dir.path(),
            ".claude/skills",
            "deploy",
            "---\nname: deploy\ndescription: How to deploy.\n---\n",
        );
        let names: Vec<String> = skills(dir.path(), &[dir.path().to_path_buf()])
            .into_iter()
            .map(|s| s.name)
            .collect();
        let outside = tempfile::tempdir().unwrap();
        write_skill(
            outside.path(),
            "skills",
            "elsewhere",
            "---\nname: elsewhere\ndescription: Outside.\n---\n",
        );
        std::os::unix::fs::symlink(
            outside.path().join("skills/elsewhere"),
            dir.path().join(".claude/skills/elsewhere"),
        )
        .unwrap();
        assert!(
            skills(dir.path(), &[dir.path().to_path_buf()])
                .iter()
                .all(|s| s.name != "elsewhere"),
            "a skill linked from outside the workspace is not listed: read refuses it"
        );
        assert_eq!(
            names,
            vec!["deploy", "rust"],
            "a separate directory adds its own"
        );
    }

    #[test]
    fn a_path_that_no_longer_exists_is_skipped_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("gone.md");

        let out = build(dir.path(), day(), &[], &[missing], &[]); // must not panic
        assert!(out.starts_with(&format!("Working directory: {}", dir.path().display())));
        assert!(!out.contains("gone.md"));
    }
}

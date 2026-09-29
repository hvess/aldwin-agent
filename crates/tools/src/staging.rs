//! The staged changeset (ADR 0009 §4): every edit of a turn lands here, and
//! nothing reaches disk until the developer approves the review.
//!
//! An overlay, never write-then-revert: `edit` and `read` see staged content,
//! `run` sees disk, and the changeset is reviewed before any call that would
//! observe disk (`Dispatcher::before_step`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use aldwin_core::{ChangedFile, Changeset};

use crate::error::ToolError;
use crate::paths::Workspace;

/// One staged file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Staged {
    /// The path as the model named it; the review shows it.
    pub rel: String,
    /// The disk content when first staged; `None` for a new file.
    /// `Staging::write_all` skips the file if disk no longer matches.
    pub before: Option<String>,
    pub after: String,
}

#[derive(Debug, Default)]
struct Inner {
    files: BTreeMap<PathBuf, Staged>,
    /// Review comments not yet reported resolved; the Saved row shows how
    /// many the approve closed.
    pending_comments: usize,
    /// What an approve last wrote to each path, until the next reload
    /// forgets it: `reload` takes new roots only from a `permissions.yaml`
    /// the review wrote and no reload has applied since, so a text the
    /// developer has moved on from cannot be restored and trusted again.
    /// Every path, since staging does not know which file that is; it is a
    /// record of writes, not an approval any later edit could reuse.
    written: BTreeMap<PathBuf, String>,
}

/// The current turn's changeset, shared by every tool and the dispatcher.
/// Locked across each staged read-modify-insert, so two edits to one file
/// cannot interleave.
#[derive(Debug)]
pub struct Staging {
    inner: Mutex<Inner>,
    /// Re-resolves each staged path at write time.
    workspace: Workspace,
}

impl Staging {
    /// An empty changeset whose writes are resolved through `workspace`.
    pub fn new(workspace: Workspace) -> Self {
        Self {
            inner: Mutex::default(),
            workspace,
        }
    }

    /// Whether nothing is staged.
    pub fn is_empty(&self) -> bool {
        self.lock().files.is_empty()
    }

    /// The staged content of `resolved`, if any edit has touched it.
    pub fn current(&self, resolved: &Path) -> Option<String> {
        self.lock().files.get(resolved).map(|s| s.after.clone())
    }

    /// Applies `change` to the file's staged content, else its disk content
    /// (`None` if absent), and stages the result.
    ///
    /// # Errors
    ///
    /// [`ToolError::Io`] when the file exists but cannot be read; any error
    /// `change` returns. Nothing is staged on error.
    pub async fn edit(
        &self,
        resolved: PathBuf,
        rel: &str,
        change: impl FnOnce(Option<&str>) -> Result<String, ToolError>,
    ) -> Result<(), ToolError> {
        // Read disk before locking: a `std::sync::Mutex` must not be held
        // across an await. It is used only when nothing is staged yet.
        let on_disk = match tokio::fs::read_to_string(&resolved).await {
            Ok(text) => Some(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(source) => {
                return Err(ToolError::Io {
                    path: resolved,
                    source,
                })
            }
        };
        let mut inner = self.lock();
        let (before, current) = match inner.files.get(&resolved) {
            Some(staged) => (staged.before.clone(), Some(staged.after.clone())),
            None => (on_disk.clone(), on_disk),
        };
        let after = change(current.as_deref())?;
        inner.files.insert(
            resolved,
            Staged {
                rel: rel.to_string(),
                before,
                after,
            },
        );
        Ok(())
    }

    /// The changeset as the review draws it, in path order.
    pub fn changeset(&self) -> Changeset {
        let inner = self.lock();
        Changeset {
            files: inner
                .files
                .values()
                .map(|s| ChangedFile {
                    path: s.rel.clone(),
                    before: s.before.clone(),
                    after: s.after.clone(),
                })
                .collect(),
        }
    }

    /// Records `count` review comments; the next approve reports them
    /// resolved.
    pub fn note_comments(&self, count: usize) {
        self.lock().pending_comments += count;
    }

    /// Writes every staged file and empties the staging area. Skips, and
    /// reports, a file whose disk content no longer matches `before` or
    /// whose path no longer resolves to where it was staged.
    ///
    /// The re-resolve is the workspace boundary at write time: a symlink
    /// swapped in while the review was open must not carry the write out.
    pub async fn write_all(&self) -> Written {
        let (files, comments) = {
            let mut inner = self.lock();
            (
                std::mem::take(&mut inner.files),
                std::mem::take(&mut inner.pending_comments),
            )
        };
        let mut written = Written {
            files: Vec::new(),
            comments_resolved: comments,
            skipped: Vec::new(),
        };
        for (resolved, staged) in files {
            if self.workspace.resolve(&staged.rel).ok().as_ref() != Some(&resolved) {
                written.skipped.push((
                    staged.rel,
                    "its path no longer resolves inside the workspace to where it was staged"
                        .into(),
                ));
                continue;
            }
            let now = match tokio::fs::read_to_string(&resolved).await {
                Ok(text) => Some(text),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => {
                    written.skipped.push((staged.rel, e.to_string()));
                    continue;
                }
            };
            if now != staged.before {
                written
                    .skipped
                    .push((staged.rel, "changed on disk since it was staged".into()));
                continue;
            }
            if let Some(parent) = resolved.parent() {
                if let Err(e) = tokio::fs::create_dir_all(parent).await {
                    written.skipped.push((staged.rel, e.to_string()));
                    continue;
                }
            }
            match tokio::fs::write(&resolved, &staged.after).await {
                Ok(()) => {
                    self.lock().written.insert(resolved, staged.after);
                    written.files.push(staged.rel);
                }
                Err(e) => written.skipped.push((staged.rel, e.to_string())),
            }
        }
        written
    }

    /// What the last approve wrote to `resolved`, if an approve has written
    /// it this session.
    pub(crate) fn approved(&self, resolved: &Path) -> Option<String> {
        self.lock().written.get(resolved).cloned()
    }

    /// Forgets what approves wrote, once a reload has applied the settings
    /// (ADR 0017 §3).
    pub fn forget_approved(&self) {
        self.lock().written.clear();
    }

    /// Drops everything staged. Returns the paths that were.
    pub fn discard(&self) -> Vec<String> {
        let mut inner = self.lock();
        inner.pending_comments = 0;
        std::mem::take(&mut inner.files)
            .into_values()
            .map(|s| s.rel)
            .collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// What an approve did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    pub files: Vec<String>,
    pub comments_resolved: usize,
    pub skipped: Vec<(String, String)>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn an_edit_is_staged_not_written_and_read_sees_it() {
        let dir = tempdir().unwrap();
        let path = dir.path().canonicalize().unwrap().join("f.rs");
        std::fs::write(&path, "old\n").unwrap();
        let staging = Staging::new(Workspace::new(dir.path()));

        staging
            .edit(path.clone(), "f.rs", |cur| {
                Ok(cur.unwrap().replace("old", "new"))
            })
            .await
            .unwrap();

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "old\n",
            "nothing on disk changes"
        );
        assert_eq!(staging.current(&path).as_deref(), Some("new\n"));
        let cs = staging.changeset();
        assert_eq!(cs.files.len(), 1);
        assert_eq!(cs.files[0].before.as_deref(), Some("old\n"));
        assert_eq!(cs.files[0].after, "new\n");
    }

    #[tokio::test]
    async fn a_second_edit_builds_on_the_first() {
        let dir = tempdir().unwrap();
        let path = dir.path().canonicalize().unwrap().join("f.rs");
        std::fs::write(&path, "a\nb\n").unwrap();
        let staging = Staging::new(Workspace::new(dir.path()));
        staging
            .edit(path.clone(), "f.rs", |cur| {
                Ok(cur.unwrap().replace("a", "A"))
            })
            .await
            .unwrap();
        staging
            .edit(path.clone(), "f.rs", |cur| {
                Ok(cur.unwrap().replace("b", "B"))
            })
            .await
            .unwrap();
        assert_eq!(staging.current(&path).as_deref(), Some("A\nB\n"));
        assert_eq!(
            staging.changeset().files[0].before.as_deref(),
            Some("a\nb\n"),
            "before is the disk, not the first edit"
        );
    }

    #[tokio::test]
    async fn a_new_file_is_staged_with_no_before() {
        let dir = tempdir().unwrap();
        let path = dir.path().canonicalize().unwrap().join("new/limit.rs");
        let staging = Staging::new(Workspace::new(dir.path()));
        staging
            .edit(path.clone(), "new/limit.rs", |cur| {
                assert!(cur.is_none());
                Ok("fn x() {}\n".into())
            })
            .await
            .unwrap();
        assert_eq!(staging.changeset().files[0].before, None);

        let written = staging.write_all().await;
        assert_eq!(written.files, vec!["new/limit.rs".to_string()]);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "fn x() {}\n",
            "approve creates the directory and the file"
        );
        assert!(staging.is_empty());
    }

    #[tokio::test]
    async fn approve_writes_and_reports_the_comments_it_closed() {
        let dir = tempdir().unwrap();
        let path = dir.path().canonicalize().unwrap().join("f.rs");
        std::fs::write(&path, "old\n").unwrap();
        let staging = Staging::new(Workspace::new(dir.path()));
        staging
            .edit(path.clone(), "f.rs", |_| Ok("new\n".into()))
            .await
            .unwrap();
        staging.note_comments(2);

        let written = staging.write_all().await;
        assert_eq!(written.files, vec!["f.rs".to_string()]);
        assert_eq!(written.comments_resolved, 2);
        assert!(written.skipped.is_empty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new\n");
        assert_eq!(staging.approved(&path).as_deref(), Some("new\n"));
    }

    #[tokio::test]
    async fn a_file_that_changed_under_the_review_is_not_overwritten() {
        let dir = tempdir().unwrap();
        let path = dir.path().canonicalize().unwrap().join("f.rs");
        std::fs::write(&path, "old\n").unwrap();
        let staging = Staging::new(Workspace::new(dir.path()));
        staging
            .edit(path.clone(), "f.rs", |_| Ok("new\n".into()))
            .await
            .unwrap();
        std::fs::write(&path, "someone else\n").unwrap();

        let written = staging.write_all().await;
        assert!(written.files.is_empty());
        assert_eq!(written.skipped[0].0, "f.rs");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "someone else\n");
        assert_eq!(
            staging.approved(&path),
            None,
            "a skipped file was not approved"
        );
    }

    #[tokio::test]
    async fn discard_drops_everything_and_names_it() {
        let dir = tempdir().unwrap();
        let path = dir.path().canonicalize().unwrap().join("f.rs");
        std::fs::write(&path, "old\n").unwrap();
        let staging = Staging::new(Workspace::new(dir.path()));
        staging
            .edit(path.clone(), "f.rs", |_| Ok("new\n".into()))
            .await
            .unwrap();
        assert_eq!(staging.discard(), vec!["f.rs".to_string()]);
        assert!(staging.is_empty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old\n");
    }

    /// Covers a swapped directory and a swapped file whose target has the
    /// same text, so the `before` check alone would pass.
    #[tokio::test]
    async fn a_symlink_swapped_in_after_staging_does_not_redirect_the_write() {
        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir(root.join("src")).unwrap();
        std::fs::write(root.join("f.rs"), "old\n").unwrap();
        std::fs::write(outside.path().join("f.rs"), "old\n").unwrap();
        let staging = Staging::new(Workspace::new(&root));
        staging
            .edit(
                root.join("src/new.rs"),
                "src/new.rs",
                |_| Ok("new\n".into()),
            )
            .await
            .unwrap();
        staging
            .edit(root.join("f.rs"), "f.rs", |_| Ok("new\n".into()))
            .await
            .unwrap();

        std::fs::remove_dir(root.join("src")).unwrap();
        std::os::unix::fs::symlink(outside.path(), root.join("src")).unwrap();
        std::fs::remove_file(root.join("f.rs")).unwrap();
        std::os::unix::fs::symlink(outside.path().join("f.rs"), root.join("f.rs")).unwrap();

        let written = staging.write_all().await;
        assert!(written.files.is_empty(), "{written:?}");
        assert_eq!(written.skipped.len(), 2);
        assert!(!outside.path().join("new.rs").exists());
        assert_eq!(
            std::fs::read_to_string(outside.path().join("f.rs")).unwrap(),
            "old\n"
        );
    }
}

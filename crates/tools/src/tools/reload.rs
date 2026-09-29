//! `reload`: the model's `/reload`. A root that would widen the workspace is
//! taken in only from a `permissions.yaml` the review wrote (ADR 0017).

use std::path::Path;
use std::sync::Arc;

use aldwin_config::{Config, PermissionsConfig, ReloadFailure};
use aldwin_core::DispatchContext;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::error::ToolError;
use crate::paths::{Widening, Workspace};
use crate::registry::{Tool, ToolDescriptor};
use crate::staging::Staging;

/// Reads the settings files again and takes in `roots:`.
pub struct ReloadTool {
    descriptor: ToolDescriptor,
    config: Config,
    workspace: Workspace,
    staging: Arc<Staging>,
}

impl ReloadTool {
    pub fn new(config: Config, workspace: Workspace, staging: Arc<Staging>) -> Self {
        Self {
            descriptor: ToolDescriptor {
                name: "reload".into(),
                description: "Read Aldwin's settings files again — `.aldwin/` in the project and \
                              `~/.aldwin/` — so a change the review wrote to one takes effect without \
                              a restart. `roots:` in `.aldwin/permissions.yaml` take effect at once, \
                              and the result lists the workspace roots. A root that would widen the \
                              workspace is taken in only while that file is exactly what the review \
                              last wrote; otherwise the result names it, and it waits for the \
                              developer's /reload. The provider and the MCP servers are set up when \
                              Aldwin starts, so a reload does not apply a change to provider.yaml or \
                              mcp.yaml."
                    .into(),
                input_schema: json!({ "type": "object", "properties": {} }),
                // Staged edits to a settings file are reviewed before this
                // reads them (ADR 0009 §4).
                observes_disk: true,
            },
            config,
            workspace,
            staging,
        }
    }

    /// The text of `permissions.yaml` when it is exactly what the last
    /// approve wrote to it.
    ///
    /// The file is read once and the roots come from that text: a second
    /// read would let a process started by `run` swap the file in between.
    async fn reviewed_permissions(&self, path: &Path) -> Option<String> {
        // Canonical first: staging keys a file under the canonical project
        // root, and `path` keeps the form the working directory had.
        let canonical = path.canonicalize().ok()?;
        let resolved = self.workspace.resolve(&canonical.to_string_lossy()).ok()?;
        let approved = self.staging.approved(&resolved)?;
        let on_disk = tokio::fs::read_to_string(path).await.ok()?;
        (on_disk == approved).then_some(on_disk)
    }
}

#[async_trait]
impl Tool for ReloadTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn call(
        &self,
        _call_id: &str,
        _input: Value,
        ctx: &DispatchContext,
    ) -> Result<String, ToolError> {
        let before = self.workspace.roots();
        let path = self.config.project_permissions_path();
        let reviewed = self.reviewed_permissions(&path).await;
        self.config
            .reload_all()
            .map_err(|failures| ToolError::Settings {
                detail: ReloadFailure::describe(&failures),
            })?;
        let taken = match reviewed {
            Some(text) => {
                let permissions =
                    PermissionsConfig::parse(&text, &path).map_err(|e| ToolError::Settings {
                        detail: e.to_string(),
                    })?;
                self.workspace
                    .take_roots(&permissions.roots, Widening::Allowed)
            }
            None => self
                .workspace
                .take_roots(&self.config.project_permissions().roots, Widening::Withheld),
        };
        // Applied now, so a later reload never trusts this text again.
        self.staging.forget_approved();

        // Reach is said to the developer by Aldwin itself (ADR 0007 §1), in
        // the words startup and `/reload` use.
        let changed = self.workspace.roots() != before;
        let told =
            (changed || !taken.withheld.is_empty() || !taken.dropped.is_empty()).then(|| {
                taken
                    .notice()
                    .unwrap_or_else(|| "The workspace is the project alone again.".into())
            });
        if let Some(message) = &told {
            ctx.notice(message.clone()).await;
        }
        let mut said = format!(
            "Settings reloaded. The workspace roots are {}.",
            self.workspace.describe()
        );
        if let Some(message) = told {
            said.push_str(&format!(" The developer was told: {message}"));
        }
        Ok(said)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::dispatch_context;
    use aldwin_core::Event;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::{tempdir, TempDir};

    struct Fixture {
        _base: TempDir,
        /// The project as the developer's shell names it: through a
        /// symlink, as every macOS temp directory is (`/var`).
        project: PathBuf,
        _global: TempDir,
        outside: TempDir,
        workspace: Workspace,
        staging: Arc<Staging>,
        tool: ReloadTool,
    }

    fn fixture() -> Fixture {
        let (base, global, outside) = (tempdir().unwrap(), tempdir().unwrap(), tempdir().unwrap());
        let project = base.path().join("link");
        fs::create_dir(base.path().join("real")).unwrap();
        std::os::unix::fs::symlink(base.path().join("real"), &project).unwrap();
        let config = Config::open_at(&project, global.path()).unwrap();
        let workspace = Workspace::new(&project);
        let staging = Arc::new(Staging::new(workspace.clone()));
        let tool = ReloadTool::new(config, workspace.clone(), staging.clone());
        Fixture {
            _base: base,
            project,
            _global: global,
            outside,
            workspace,
            staging,
            tool,
        }
    }

    impl Fixture {
        fn permissions(&self) -> String {
            format!(
                "version: 2\nroots:\n  - {}\n",
                self.outside.path().display()
            )
        }

        async fn reload(&self) -> String {
            self.reload_telling().await.0
        }

        /// The result, and the notices the developer was sent.
        async fn reload_telling(&self) -> (String, Vec<String>) {
            let (ctx, mut rx, _pending) = dispatch_context();
            let said = self.tool.call("c1", json!({}), &ctx).await.unwrap();
            let mut notices = Vec::new();
            while let Ok(event) = rx.try_recv() {
                if let Event::Notice { message } = event {
                    notices.push(message);
                }
            }
            (said, notices)
        }

        fn outside(&self) -> PathBuf {
            self.outside.path().canonicalize().unwrap()
        }
    }

    #[tokio::test]
    async fn a_root_the_review_wrote_is_taken_in() {
        let f = fixture();
        let path = f.workspace.resolve(".aldwin/permissions.yaml").unwrap();
        let text = f.permissions();
        f.staging
            .edit(path, ".aldwin/permissions.yaml", |_| Ok(text.clone()))
            .await
            .unwrap();
        f.staging.write_all().await;

        let (said, notices) = f.reload_telling().await;

        assert_eq!(f.workspace.roots()[1], f.outside());
        let outside = f.outside().display().to_string();
        assert!(said.contains(&outside), "{said}");
        assert!(
            notices[0].contains(&outside),
            "the developer is told: {notices:?}"
        );
    }

    /// A process started by `run` can write permissions.yaml without the
    /// review; reloading must not let it widen the workspace.
    #[tokio::test]
    async fn a_root_written_any_other_way_waits_for_the_developer() {
        let f = fixture();
        let path = f.project.join(".aldwin/permissions.yaml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, f.permissions()).unwrap();

        let (said, notices) = f.reload_telling().await;

        assert_eq!(f.workspace.roots().len(), 1, "nothing was taken in");
        assert!(notices[0].contains("was not taken in"), "{notices:?}");
        assert!(
            said.contains(&notices[0]),
            "the model reads what the developer was told"
        );
    }

    #[tokio::test]
    async fn a_reviewed_file_changed_since_is_not_trusted() {
        let f = fixture();
        let path = f.workspace.resolve(".aldwin/permissions.yaml").unwrap();
        f.staging
            .edit(path.clone(), ".aldwin/permissions.yaml", |_| {
                Ok("version: 2\n".into())
            })
            .await
            .unwrap();
        f.staging.write_all().await;
        fs::write(&path, f.permissions()).unwrap();

        f.reload().await;

        assert_eq!(f.workspace.roots().len(), 1);
    }

    /// A reviewed text restored after the developer moved on (say by `git
    /// checkout`) must not widen the workspace again.
    #[tokio::test]
    async fn a_reviewed_text_is_trusted_once() {
        let f = fixture();
        let path = f.workspace.resolve(".aldwin/permissions.yaml").unwrap();
        let text = f.permissions();
        f.staging
            .edit(path.clone(), ".aldwin/permissions.yaml", |_| {
                Ok(text.clone())
            })
            .await
            .unwrap();
        f.staging.write_all().await;
        f.reload().await;
        fs::write(&path, "version: 2\n").unwrap();
        f.reload().await;
        assert_eq!(f.workspace.roots().len(), 1, "removed by hand");

        fs::write(&path, &text).unwrap();
        f.reload().await;

        assert_eq!(
            f.workspace.roots().len(),
            1,
            "the old text is not trusted again"
        );
    }

    #[tokio::test]
    async fn a_root_removed_by_hand_is_removed() {
        let f = fixture();
        f.workspace
            .take_roots(&[f.outside.path().into()], Widening::Allowed);
        let path = f.project.join(".aldwin/permissions.yaml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "version: 2\n").unwrap();

        f.reload().await;

        assert_eq!(f.workspace.roots(), vec![f.workspace.project_root()]);
    }

    #[tokio::test]
    async fn a_file_that_cannot_be_read_is_named_and_the_roots_stay() {
        let f = fixture();
        f.workspace
            .take_roots(&[f.outside.path().into()], Widening::Allowed);
        let path = f.project.join(".aldwin/permissions.yaml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "version: [\n").unwrap();

        let (ctx, _rx, _pending) = dispatch_context();
        let err = f.tool.call("c1", json!({}), &ctx).await.unwrap_err();

        assert!(err.to_string().contains("permissions.yaml"), "{err}");
        assert_eq!(f.workspace.roots()[1], f.outside());
    }
}

//! The headless compositor every frame is captured inside: `sway` on the
//! wlroots headless backend, with its own Wayland socket.
//!
//! Two invariants:
//!
//! * **The config must validate.** An invalid line paints sway's config-error
//!   bar across every frame; [`Compositor::start`] refuses a config `sway -C`
//!   rejects.
//! * **`grim` must run inside the session**, through [`Compositor::exec`].
//!   Run outside, it inherits the developer's `WAYLAND_DISPLAY` and silently
//!   captures their desktop.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::{Error, Result};

/// The name wlroots' headless backend gives its single output.
pub const OUTPUT: &str = "HEADLESS-1";

/// No borders and zero gaps make foot's surface exactly the output. Every
/// line must pass `sway -C` (`titlebar_padding 0` does not).
const CONFIG: &str = "\
output HEADLESS-1 mode 1200x800
default_border none
default_floating_border none
gaps inner 0
gaps outer 0
";

/// A running headless sway. Dropping it asks sway to exit, kills it after
/// three seconds, and removes its socket.
#[derive(Debug)]
pub struct Compositor {
    sock: PathBuf,
    child: Child,
}

impl Compositor {
    /// Writes a config into `dir`, validates it, starts sway on the headless
    /// backend and waits for its IPC to answer.
    ///
    /// # Errors
    ///
    /// When the config cannot be written, `sway -C` rejects it, sway cannot be
    /// spawned or its log created, or its IPC does not answer within ten
    /// seconds.
    pub fn start(dir: &Path) -> Result<Self> {
        let cfg = dir.join("sway.cfg");
        std::fs::write(&cfg, CONFIG)?;

        let check = Command::new("sway")
            .arg("-C")
            .arg("-c")
            .arg(&cfg)
            .output()?;
        if !check.status.success() {
            return Err(Error::Compositor(format!(
                "sway rejected the harness config: {}",
                String::from_utf8_lossy(&check.stderr).trim()
            )));
        }

        // In the runtime dir, not the run dir: a unix socket path is capped at
        // ~108 bytes.
        let runtime = runtime_dir();
        // Pid and counter: concurrent runs, and one process opening several
        // compositors (cell measurement, then capture), must not share a socket.
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let sock = runtime.join(format!("aldwin-shot-{}-{n}.sock", std::process::id()));
        let _ = std::fs::remove_file(&sock);

        let log = std::fs::File::create(dir.join("sway.log"))?;
        let child = Command::new("sway")
            .arg("-c")
            .arg(&cfg)
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("DISPLAY")
            .env("WLR_BACKENDS", "headless")
            .env("WLR_LIBINPUT_NO_DEVICES", "1")
            .env("SWAYSOCK", &sock)
            .env("XDG_RUNTIME_DIR", &runtime)
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log)
            .spawn()?;

        let this = Compositor { sock, child };
        this.wait_ready()?;
        Ok(this)
    }

    fn wait_ready(&self) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if self.msg(&["-t", "get_outputs"]).is_ok() {
                return Ok(());
            }
            sleep(Duration::from_millis(100));
        }
        Err(Error::Compositor("sway did not come up".into()))
    }

    /// One `swaymsg` call against this compositor's own socket.
    ///
    /// # Errors
    ///
    /// When `swaymsg` cannot be run or exits unsuccessfully; the error carries
    /// its stderr.
    pub fn msg(&self, args: &[&str]) -> Result<String> {
        let out = Command::new("swaymsg")
            .arg("-s")
            .arg(&self.sock)
            .args(args)
            .output()?;
        if !out.status.success() {
            return Err(Error::Compositor(format!(
                "swaymsg {args:?} failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Runs a shell command inside the session, so it inherits the headless
    /// `WAYLAND_DISPLAY`.
    ///
    /// # Errors
    ///
    /// As [`Compositor::msg`]: sway refused the `exec`. The command's own
    /// failure is not seen.
    pub fn exec(&self, command: &str) -> Result<()> {
        self.msg(&["exec", command]).map(|_| ())
    }

    /// Resizes the output. Callers pass an exact multiple of the measured cell.
    ///
    /// # Errors
    ///
    /// As [`Compositor::msg`]: sway refused the mode.
    pub fn set_mode(&self, width: u32, height: u32) -> Result<()> {
        self.msg(&[
            "--",
            "output",
            OUTPUT,
            "mode",
            "--custom",
            &format!("{width}x{height}"),
        ])?;
        // The mode change is asynchronous; foot must not launch into the old
        // geometry.
        sleep(Duration::from_millis(400));
        Ok(())
    }

    /// How many windows are on the output; a capture is valid only at one.
    ///
    /// # Errors
    ///
    /// As [`Compositor::msg`], or when sway's tree is not valid JSON.
    pub fn surface_count(&self) -> Result<usize> {
        let tree: serde_json::Value = serde_json::from_str(&self.msg(&["-t", "get_tree"])?)?;
        fn walk(node: &serde_json::Value, n: &mut usize) {
            for key in ["nodes", "floating_nodes"] {
                for child in node[key].as_array().into_iter().flatten() {
                    if child["app_id"].is_string() || child["window_properties"].is_object() {
                        *n += 1;
                    }
                    walk(child, n);
                }
            }
        }
        let mut n = 0;
        walk(&tree, &mut n);
        Ok(n)
    }

    /// Closes every foot window, so the next one does not tile beside it.
    ///
    /// # Errors
    ///
    /// Never: a failed kill or a window that outlives the 3 s wait is left
    /// for the next [`Compositor::surface_count`] to report.
    pub fn clear(&self) -> Result<()> {
        let _ = self.msg(&["[app_id=\"foot\"]", "kill"]);
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if self.surface_count().unwrap_or(1) == 0 {
                return Ok(());
            }
            sleep(Duration::from_millis(50));
        }
        Ok(())
    }
}

impl Drop for Compositor {
    fn drop(&mut self) {
        let _ = self.msg(&["exit"]);
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(_)) => break,
                _ => sleep(Duration::from_millis(50)),
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.sock);
    }
}

fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            // SAFETY: getuid takes nothing and cannot fail.
            PathBuf::from(format!("/run/user/{}", unsafe { libc::getuid() }))
        })
}

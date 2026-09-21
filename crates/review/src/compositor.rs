//! The headless compositor every frame is captured inside.
//!
//! `sway` on the wlroots headless backend: no GPU, no window on the
//! developer's desktop, and its own Wayland socket, so a capture run cannot
//! see — or be seen by — whatever the developer has open.
//!
//! Two traps from the 2026-09-19 probe are enforced here rather than
//! documented and hoped for:
//!
//! * **The compositor must be silent.** One invalid line in a sway config
//!   paints a red "errors in your config file" bar across the top of every
//!   frame, and every capture after it is wrong in a way that still looks
//!   plausible. [`Compositor::start`] runs `sway -C` over the config and
//!   refuses to start if it does not validate.
//! * **`grim` must run *inside* the session.** Launched from outside it
//!   inherits the developer's own `WAYLAND_DISPLAY` and screenshots their
//!   real desktop — silently, successfully, with a believable PNG. Every
//!   capture goes through [`Compositor::exec`], which hands the command to
//!   sway to run in its own environment.

use std::io::{Error, ErrorKind, Result};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

/// The headless backend names its single output this. Fixed by wlroots, not
/// by us — every `swaymsg output` and `grim -o` refers to it.
pub const OUTPUT: &str = "HEADLESS-1";

/// Deliberately minimal, and every line of it load-bearing. `default_border
/// none` and zero gaps are what make foot's surface exactly the output, so a
/// capture is the frame and nothing else. Anything added here must survive
/// `sway -C`; the probe lost a run to `titlebar_padding 0`, which does not.
const CONFIG: &str = "\
output HEADLESS-1 mode 1200x800
default_border none
default_floating_border none
gaps inner 0
gaps outer 0
";

pub struct Compositor {
    sock:  PathBuf,
    child: Child,
}

impl Compositor {
    /// Writes a config into `dir`, validates it, starts sway on the headless
    /// backend and waits for its IPC to answer.
    pub fn start(dir: &Path) -> Result<Self> {
        let cfg = dir.join("sway.cfg");
        std::fs::write(&cfg, CONFIG)?;

        let check = Command::new("sway").arg("-C").arg("-c").arg(&cfg).output()?;
        if !check.status.success() {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!("sway rejected the harness config: {}", String::from_utf8_lossy(&check.stderr).trim()),
            ));
        }

        // A unix socket path is capped at ~108 bytes, and a run directory
        // nested under `target/` blows that on its own. The runtime dir keeps
        // it short, and the pid keeps two concurrent runs apart.
        let runtime = runtime_dir();
        // Pid *and* a counter: one process can open more than one compositor
        // in a run (preflight measures a cell, then the session captures), and
        // two of them sharing a socket path would have the second talk to the
        // first.
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
        Err(Error::new(ErrorKind::TimedOut, "sway did not come up"))
    }

    /// One `swaymsg` call against this compositor's own socket.
    pub fn msg(&self, args: &[&str]) -> Result<String> {
        let out = Command::new("swaymsg").arg("-s").arg(&self.sock).args(args).output()?;
        if !out.status.success() {
            return Err(Error::other(format!("swaymsg {args:?} failed: {}", String::from_utf8_lossy(&out.stderr).trim()),
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Run a command inside the session, so it inherits the headless
    /// `WAYLAND_DISPLAY` rather than the developer's.
    pub fn exec(&self, command: &str) -> Result<()> {
        self.msg(&["exec", command]).map(|_| ())
    }

    /// Resize the output. The harness always passes an exact multiple of the
    /// measured cell, so the frame is a whole number of cells with no slack.
    pub fn set_mode(&self, width: u32, height: u32) -> Result<()> {
        self.msg(&["--", "output", OUTPUT, "mode", "--custom", &format!("{width}x{height}")])?;
        // The mode change is asynchronous; foot must not be launched into the
        // old geometry or it starts at one size and is resized under it.
        sleep(Duration::from_millis(400));
        Ok(())
    }

    /// How many surfaces are on the output. A capture is only trustworthy
    /// when the answer is exactly one — anything else means something is
    /// sharing the frame with the app under test.
    pub fn surface_count(&self) -> Result<usize> {
        let tree: serde_json::Value = serde_json::from_str(&self.msg(&["-t", "get_tree"])?)
            .map_err(|e| Error::new(ErrorKind::InvalidData, e))?;
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

    /// Close every terminal on the output, so the next capture starts from an
    /// empty frame rather than tiling beside the last one.
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
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", current_uid())))
}

/// `id -u` — a subprocess rather than an ioctl, because this runs once per run
/// and only as a fallback when `XDG_RUNTIME_DIR` is unset.
fn current_uid() -> String {
    Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "1000".into())
}

//! Pseudoterminal plumbing for the proxy.
//!
//! The harness owns both ends of two ptys. It is the only arrangement that
//! gives all three things capture needs at once: the bytes the app actually
//! emits (so a cell grid can be parsed from them), a way to know when the app
//! has gone quiet, and a way to press keys.
//!
//! libc is reached only for what std has no word for — opening the pair, the
//! line discipline, the window size and the controlling terminal. Once open,
//! the master is a `File` and is read and written through std. The fiddly
//! part is [`Pty::attach_as_controlling`]: a TUI needs its pty to *be* its
//! controlling terminal, which means `setsid` and `TIOCSCTTY` between fork
//! and exec.

use std::ffi::CStr;
use std::fs::{File, OpenOptions};
use std::io::{Error, ErrorKind, Read, Result, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Pty {
    master: File,
    slave_path: PathBuf,
}

impl Pty {
    /// Open a pty pair and put the line discipline in raw mode.
    ///
    /// Raw is not a detail: with `ECHO` on, every byte the harness writes to
    /// the master comes straight back at it as if the far side had typed it.
    /// The probe hit exactly that and read its own output back.
    pub fn open() -> Result<Self> {
        // SAFETY: posix_openpt returns a fresh fd or -1, and a fresh fd is
        // handed straight to OwnedFd so it is closed exactly once.
        let master = unsafe {
            let fd = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
            if fd < 0 {
                return Err(Error::last_os_error());
            }
            File::from(OwnedFd::from_raw_fd(fd))
        };
        let fd = master.as_raw_fd();

        // SAFETY: fd is the open master for the duration of these calls, and
        // buf and tio are valid for the sizes passed.
        let slave_path = unsafe {
            if libc::grantpt(fd) < 0 || libc::unlockpt(fd) < 0 {
                return Err(Error::last_os_error());
            }
            let mut buf = [0 as libc::c_char; 256];
            // ptsname_r returns the error number rather than setting errno.
            let rc = libc::ptsname_r(fd, buf.as_mut_ptr(), buf.len());
            if rc != 0 {
                return Err(Error::from_raw_os_error(rc));
            }
            let slave_path =
                PathBuf::from(CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned());

            let mut tio: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(fd, &mut tio) != 0 {
                return Err(Error::last_os_error());
            }
            libc::cfmakeraw(&mut tio);
            if libc::tcsetattr(fd, libc::TCSANOW, &tio) != 0 {
                return Err(Error::last_os_error());
            }
            slave_path
        };
        Ok(Pty { master, slave_path })
    }

    /// The `/dev/pts/N` path. `foot --pty` takes this one; the app gets the
    /// other pty's, as its controlling terminal.
    pub fn slave_path(&self) -> &Path {
        &self.slave_path
    }

    pub fn try_clone_master(&self) -> Result<File> {
        self.master.try_clone()
    }

    /// Write to the master, as if the far side's keyboard had.
    pub fn write_all(&self, bytes: &[u8]) -> Result<()> {
        (&self.master).write_all(bytes)
    }

    /// Window size as the far side sees it. foot writes this on its pty once
    /// its window is configured, which is how the harness learns the app's
    /// geometry is real rather than foot's initial 80×24.
    pub fn winsize(&self) -> Result<(u16, u16)> {
        // SAFETY: winsize is plain data, and all-zero is a valid value of it.
        let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
        // SAFETY: ws is a valid winsize for the duration of the call.
        if unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCGWINSZ, &mut ws) } < 0 {
            return Err(Error::last_os_error());
        }
        Ok((ws.ws_col, ws.ws_row))
    }

    /// Set the window size, which also delivers `SIGWINCH` to the foreground
    /// process group — how a resize reaches the app.
    pub fn set_winsize(&self, cols: u16, rows: u16) -> Result<()> {
        let ws = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: ws outlives the call.
        if unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ, &ws) } < 0 {
            return Err(Error::last_os_error());
        }
        Ok(())
    }

    /// Spawn `command` with this pty's slave as stdin/stdout/stderr *and* as
    /// its controlling terminal.
    ///
    /// Without `setsid` the child stays in the harness's session and
    /// `TIOCSCTTY` fails; without `TIOCSCTTY` the app has a tty on its fds but
    /// no controlling terminal, and anything that queries the terminal —
    /// which is exactly the layer this harness exists to exercise — behaves
    /// differently or not at all.
    pub fn attach_as_controlling(&self, command: &mut Command) -> Result<std::process::Child> {
        // O_NOCTTY: the harness opens the slave only to hand it on, and must
        // not acquire it as its own controlling terminal on the way.
        let slave = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOCTTY)
            .open(&self.slave_path)?;
        let slave_fd = slave.as_raw_fd();

        // SAFETY: the closure runs in the forked child before exec, where
        // only async-signal-safe calls are allowed. setsid, ioctl, dup2 and
        // close are, and it allocates nothing: `last_os_error` only reads
        // errno. slave_fd stays open in the parent until spawn returns, so
        // the child inherits a valid descriptor.
        unsafe {
            command.pre_exec(move || {
                if libc::setsid() < 0 {
                    return Err(Error::last_os_error());
                }
                if libc::ioctl(slave_fd, libc::TIOCSCTTY, 0) < 0 {
                    return Err(Error::last_os_error());
                }
                for target in 0..=2 {
                    if libc::dup2(slave_fd, target) < 0 {
                        return Err(Error::last_os_error());
                    }
                }
                if slave_fd > 2 {
                    libc::close(slave_fd);
                }
                Ok(())
            });
        }
        let child = command.spawn()?;
        drop(slave);
        Ok(child)
    }
}

/// Read from a pty master. Returns `Ok(0)` at EOF, which for a pty master
/// means the far side closed — the app exited.
pub fn read(master: &mut File, buf: &mut [u8]) -> Result<usize> {
    loop {
        match master.read(buf) {
            // A pty master reports EIO rather than EOF when the slave is gone.
            Err(e) if e.raw_os_error() == Some(libc::EIO) => return Ok(0),
            // Retried like `write_all` does: the pump treats an error as the
            // far side closing, and a signal is not that.
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            other => return other,
        }
    }
}

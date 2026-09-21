//! Pseudoterminal plumbing for the proxy.
//!
//! The harness owns both ends of two ptys. It is the only arrangement that
//! gives all three things the gates need at once: the bytes the app actually
//! emits (so a cell grid can be parsed from them), a way to know when the app
//! has gone quiet, and a way to press keys.
//!
//! Everything here is a thin, checked wrapper over libc. The fiddly part is
//! [`Pty::attach_as_controlling`]: a TUI needs its pty to *be* its controlling
//! terminal, which means `setsid` and `TIOCSCTTY` between fork and exec.

use std::ffi::CStr;
use std::io::{Error, Result};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

pub struct Pty {
    master:     OwnedFd,
    slave_path: PathBuf,
}

impl Pty {
    /// Open a pty pair and put the line discipline in raw mode.
    ///
    /// Raw is not a detail: with `ECHO` on, every byte the harness writes to
    /// the master comes straight back at it as if the far side had typed it.
    /// The probe hit exactly that and read its own output back.
    pub fn open() -> Result<Self> {
        // SAFETY: each call is checked, and the fd is handed to OwnedFd on
        // success so it is closed exactly once.
        unsafe {
            let fd = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
            if fd < 0 {
                return Err(Error::last_os_error());
            }
            let master = OwnedFd::from_raw_fd(fd);
            if libc::grantpt(fd) < 0 || libc::unlockpt(fd) < 0 {
                return Err(Error::last_os_error());
            }
            let mut buf = [0 as libc::c_char; 256];
            if libc::ptsname_r(fd, buf.as_mut_ptr(), buf.len()) != 0 {
                return Err(Error::last_os_error());
            }
            let slave_path = PathBuf::from(CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned());

            let mut tio: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(fd, &mut tio) == 0 {
                libc::cfmakeraw(&mut tio);
                let _ = libc::tcsetattr(fd, libc::TCSANOW, &tio);
            }
            Ok(Pty { master, slave_path })
        }
    }

    pub fn master_fd(&self) -> RawFd {
        self.master.as_raw_fd()
    }

    /// The `/dev/pts/N` path. `foot --pty` takes this one; the app gets the
    /// other pty's, as its controlling terminal.
    pub fn slave_path(&self) -> &std::path::Path {
        &self.slave_path
    }

    pub fn try_clone_master(&self) -> Result<OwnedFd> {
        self.master.try_clone()
    }

    /// Window size as the far side sees it. foot writes this on its pty once
    /// its window is configured, which is how the harness learns the app's
    /// geometry is real rather than foot's initial 80×24.
    pub fn winsize(&self) -> Result<(u16, u16)> {
        let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
        // SAFETY: ws is a valid winsize for the duration of the call.
        if unsafe { libc::ioctl(self.master_fd(), libc::TIOCGWINSZ, &mut ws) } < 0 {
            return Err(Error::last_os_error());
        }
        Ok((ws.ws_col, ws.ws_row))
    }

    /// Set the window size, which also delivers `SIGWINCH` to the foreground
    /// process group — how a resize reaches the app.
    pub fn set_winsize(&self, cols: u16, rows: u16) -> Result<()> {
        let ws = libc::winsize { ws_row: rows, ws_col: cols, ws_xpixel: 0, ws_ypixel: 0 };
        // SAFETY: ws outlives the call.
        if unsafe { libc::ioctl(self.master_fd(), libc::TIOCSWINSZ, &ws) } < 0 {
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
        let slave = std::fs::OpenOptions::new().read(true).write(true).open(&self.slave_path)?;
        let slave_fd = slave.as_raw_fd();

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

/// Read from a raw fd. Returns `Ok(0)` at EOF, which for a pty master means
/// the far side closed — the app exited.
pub fn read(fd: RawFd, buf: &mut [u8]) -> Result<usize> {
    loop {
        // SAFETY: buf is valid for len bytes.
        let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        if n >= 0 {
            return Ok(n as usize);
        }
        let err = Error::last_os_error();
        // A pty master reports EIO rather than EOF when the slave is gone.
        if err.raw_os_error() == Some(libc::EIO) {
            return Ok(0);
        }
        // Retried like `write_all` does: the pump treats an error as the far
        // side closing, and a signal is not that.
        if err.kind() != std::io::ErrorKind::Interrupted {
            return Err(err);
        }
    }
}

pub fn write_all(fd: RawFd, mut buf: &[u8]) -> Result<()> {
    while !buf.is_empty() {
        // SAFETY: buf is valid for len bytes.
        let n = unsafe { libc::write(fd, buf.as_ptr() as *const libc::c_void, buf.len()) };
        if n < 0 {
            let err = Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        buf = &buf[n as usize..];
    }
    Ok(())
}

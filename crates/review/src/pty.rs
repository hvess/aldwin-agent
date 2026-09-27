//! Pseudoterminal plumbing for the proxy, which holds the masters of two ptys
//! (foot's and the app's) to see the app's bytes, detect quiet and press keys.
//!
//! libc only for what std lacks: opening the pair, the line discipline, the
//! window size and the controlling terminal.

use std::ffi::CStr;
use std::fs::{File, OpenOptions};
use std::io::{self, ErrorKind, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

use crate::Result;

/// A pty pair: the master held open as a `File`, the slave known by path
/// and opened only by whoever is handed it.
#[derive(Debug)]
pub struct Pty {
    master: File,
    slave_path: PathBuf,
}

impl Pty {
    /// Opens a pty pair with the line discipline in raw mode.
    ///
    /// Must stay raw: with `ECHO` on, every byte written to the master is read
    /// back from it.
    ///
    /// # Errors
    ///
    /// When the pair cannot be opened, granted, unlocked or named, or its
    /// line discipline cannot be read or set raw.
    pub fn open() -> Result<Self> {
        // SAFETY: posix_openpt returns a fresh fd or -1, and a fresh fd is
        // handed straight to OwnedFd so it is closed exactly once.
        let master = unsafe {
            let fd = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
            if fd < 0 {
                return Err(io::Error::last_os_error().into());
            }
            File::from(OwnedFd::from_raw_fd(fd))
        };
        let fd = master.as_raw_fd();

        // SAFETY: fd is the open master for the duration of these calls, and
        // buf and tio are valid for the sizes passed.
        let slave_path = unsafe {
            if libc::grantpt(fd) < 0 || libc::unlockpt(fd) < 0 {
                return Err(io::Error::last_os_error().into());
            }
            let mut buf = [0 as libc::c_char; 256];
            // ptsname_r returns the error number rather than setting errno.
            let rc = libc::ptsname_r(fd, buf.as_mut_ptr(), buf.len());
            if rc != 0 {
                return Err(io::Error::from_raw_os_error(rc).into());
            }
            let slave_path =
                PathBuf::from(CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned());

            let mut tio: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(fd, &mut tio) != 0 {
                return Err(io::Error::last_os_error().into());
            }
            libc::cfmakeraw(&mut tio);
            if libc::tcsetattr(fd, libc::TCSANOW, &tio) != 0 {
                return Err(io::Error::last_os_error().into());
            }
            slave_path
        };
        Ok(Pty { master, slave_path })
    }

    /// The slave's `/dev/pts/N` path, for `foot --pty`.
    pub fn slave_path(&self) -> &Path {
        &self.slave_path
    }

    /// A second handle on the master, for a pump thread to own.
    ///
    /// # Errors
    ///
    /// When the descriptor cannot be duplicated.
    pub fn try_clone_master(&self) -> Result<File> {
        Ok(self.master.try_clone()?)
    }

    /// Writes to the master, as the slave side's input.
    ///
    /// # Errors
    ///
    /// When the write fails.
    pub fn write_all(&self, bytes: &[u8]) -> Result<()> {
        (&self.master).write_all(bytes)?;
        Ok(())
    }

    /// Window size as columns and rows. foot sets it once its window is
    /// configured, replacing its initial 80×24.
    ///
    /// # Errors
    ///
    /// When the `TIOCGWINSZ` ioctl fails.
    pub fn winsize(&self) -> Result<(u16, u16)> {
        // SAFETY: winsize is plain data, and all-zero is a valid value of it.
        let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
        // SAFETY: ws is a valid winsize for the duration of the call.
        if unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCGWINSZ, &mut ws) } < 0 {
            return Err(io::Error::last_os_error().into());
        }
        Ok((ws.ws_col, ws.ws_row))
    }

    /// Sets the window size, which sends `SIGWINCH` to the foreground process
    /// group.
    ///
    /// # Errors
    ///
    /// When the `TIOCSWINSZ` ioctl fails.
    pub fn set_winsize(&self, cols: u16, rows: u16) -> Result<()> {
        let ws = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: ws outlives the call.
        if unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ, &ws) } < 0 {
            return Err(io::Error::last_os_error().into());
        }
        Ok(())
    }

    /// Spawns `command` with this pty's slave as stdin/stdout/stderr and as its
    /// controlling terminal.
    ///
    /// Both `setsid` and `TIOCSCTTY` are required: without `setsid`,
    /// `TIOCSCTTY` fails; without `TIOCSCTTY`, terminal queries misbehave.
    ///
    /// # Errors
    ///
    /// When the slave cannot be opened or the command cannot be spawned,
    /// including `setsid`, `TIOCSCTTY` or `dup2` failing in the child.
    pub fn attach_as_controlling(&self, command: &mut Command) -> Result<Child> {
        // O_NOCTTY: the harness must not acquire the slave as its own
        // controlling terminal.
        let slave = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOCTTY)
            .open(&self.slave_path)?;
        let slave_fd = slave.as_raw_fd();

        // SAFETY: the closure runs between fork and exec, so it must be
        // async-signal-safe: setsid, ioctl, dup2 and close are, and it
        // allocates nothing (`last_os_error` only reads errno). slave_fd stays
        // open in the parent until spawn returns.
        unsafe {
            command.pre_exec(move || {
                if libc::setsid() < 0 {
                    return Err(io::Error::last_os_error());
                }
                if libc::ioctl(slave_fd, libc::TIOCSCTTY, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
                for target in 0..=2 {
                    if libc::dup2(slave_fd, target) < 0 {
                        return Err(io::Error::last_os_error());
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

/// Reads from a pty master; `Ok(0)` means the slave side closed.
///
/// # Errors
///
/// When the read fails with anything but `EIO` (read as `Ok(0)`) or
/// `Interrupted` (retried).
pub fn read(master: &mut File, buf: &mut [u8]) -> Result<usize> {
    loop {
        match master.read(buf) {
            // A pty master reports EIO rather than EOF when the slave is gone.
            Err(e) if e.raw_os_error() == Some(libc::EIO) => return Ok(0),
            // Retried: the pump reads any error as the far side closing.
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            other => return Ok(other?),
        }
    }
}

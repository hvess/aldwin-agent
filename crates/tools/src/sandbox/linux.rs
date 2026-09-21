//! Landlock, spoken directly to the kernel.
//!
//! Three syscalls and two structs, rather than a crate, for one reason that
//! matters here: the ruleset has to be **built in the parent and engaged in
//! the child**. Building it opens file descriptors and allocates; doing that
//! between `fork` and `execve` in a threaded process is how you get a child
//! that deadlocks in the allocator. So `ReadOnly::build` does all of it up
//! front, and [`ReadOnly::engage`] — the part that actually runs in the
//! forked child — is two syscalls with no allocation, which is what
//! `pre_exec` is allowed to be.

use std::ffi::CString;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::Path;

use super::{incidental_writes, Availability};

const SYS_CREATE_RULESET: libc::c_long = 444;
const SYS_ADD_RULE: libc::c_long = 445;
const SYS_RESTRICT_SELF: libc::c_long = 446;

const CREATE_RULESET_VERSION: u32 = 1;
const RULE_PATH_BENEATH: libc::c_int = 1;

// Filesystem access bits, in ABI order. Everything from EXECUTE to MAKE_SYM
// is ABI 1; REFER is 2, TRUNCATE is 3, IOCTL_DEV is 5.
const FS_EXECUTE: u64 = 1 << 0;
const FS_WRITE_FILE: u64 = 1 << 1;
const FS_READ_FILE: u64 = 1 << 2;
const FS_READ_DIR: u64 = 1 << 3;
const FS_ABI1: u64 = (1 << 13) - 1; // EXECUTE..MAKE_SYM
const FS_REFER: u64 = 1 << 13;
const FS_TRUNCATE: u64 = 1 << 14;
const FS_IOCTL_DEV: u64 = 1 << 15;

const NET_BIND_TCP: u64 = 1 << 0;
const NET_CONNECT_TCP: u64 = 1 << 1;

/// The three rights that let a program run and look at things, and nothing
/// else. Everything a ruleset handles but does not grant is denied.
const READ_RIGHTS: u64 = FS_EXECUTE | FS_READ_FILE | FS_READ_DIR;

#[repr(C)]
struct RulesetAttr {
    handled_access_fs:  u64,
    handled_access_net: u64,
}

#[repr(C, packed)]
struct PathBeneathAttr {
    allowed_access: u64,
    parent_fd:      RawFd,
}

fn abi_version() -> io::Result<i32> {
    // SAFETY: the version probe is defined as a null attr, zero size, and the
    // VERSION flag; it reads nothing and returns the ABI number.
    let v = unsafe {
        libc::syscall(SYS_CREATE_RULESET, std::ptr::null::<RulesetAttr>(), 0usize, CREATE_RULESET_VERSION)
    };
    if v < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(v as i32)
}

pub fn availability() -> Availability {
    match abi_version() {
        Ok(abi) if abi >= 1 => Availability::Enforcing { abi, network: abi >= 4 },
        Ok(_) => Availability::Unavailable { reason: "this kernel reports no usable Landlock ABI" },
        Err(e) if e.raw_os_error() == Some(libc::ENOSYS) => {
            Availability::Unavailable { reason: "this kernel has no Landlock support" }
        }
        Err(_) => Availability::Unavailable {
            reason: "Landlock is present but disabled — add it to the kernel's lsm= list",
        },
    }
}

/// Every filesystem right this ABI knows about. A ruleset must *handle* a
/// right before it can deny it, so anything left out here is a right the
/// sandbox silently permits — which is why this tracks the ABI upward rather
/// than pinning to the version that existed when it was written.
fn handled_fs(abi: i32) -> u64 {
    let mut bits = FS_ABI1;
    if abi >= 2 {
        bits |= FS_REFER;
    }
    if abi >= 3 {
        bits |= FS_TRUNCATE;
    }
    if abi >= 5 {
        bits |= FS_IOCTL_DEV;
    }
    bits
}

/// A built, unengaged ruleset: read anything, write only the incidental
/// paths, and — from ABI 4 — reach no TCP address.
pub struct ReadOnly {
    ruleset: OwnedFd,
    abi:     i32,
}

impl ReadOnly {
    pub fn build(project_root: &Path) -> io::Result<Self> {
        let abi = abi_version()?;
        let handled_net = if abi >= 4 { NET_BIND_TCP | NET_CONNECT_TCP } else { 0 };

        let attr =
            RulesetAttr { handled_access_fs: handled_fs(abi), handled_access_net: handled_net };
        // ABI 1-3 kernels know an 8-byte attr; handing them 16 is E2BIG.
        let attr_size = if abi >= 4 { std::mem::size_of::<RulesetAttr>() } else { 8 };

        // SAFETY: `attr` outlives the call and `attr_size` matches what this
        // ABI defines the struct to be.
        let fd =
            unsafe { libc::syscall(SYS_CREATE_RULESET, &attr as *const RulesetAttr, attr_size, 0) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the syscall returned a fresh, owned file descriptor.
        let ruleset = unsafe { OwnedFd::from_raw_fd(fd as RawFd) };

        // Read the whole filesystem. See the module docs for why this is not
        // the confinement — argument containment is.
        add_path_rule(&ruleset, Path::new("/"), READ_RIGHTS)?;

        // ...and write only here. A path that does not exist is not an error:
        // a project with no `target/` simply has nothing to exempt.
        for path in incidental_writes(project_root) {
            let _ = add_path_rule(&ruleset, &path, writable_rights(abi, &path));
        }

        Ok(Self { ruleset, abi })
    }

    pub fn abi(&self) -> i32 {
        self.abi
    }

    /// Engage the ruleset on the calling process. Everything it goes on to
    /// `exec`, and every child of that, inherits it and cannot widen it.
    ///
    /// # Safety
    /// Intended for `pre_exec`, between `fork` and `execve`. It allocates
    /// nothing and takes no locks — two syscalls — which is what makes it
    /// safe to call there. Calling it on the parent would confine Aldwin
    /// itself, permanently.
    pub unsafe fn engage(&self) -> io::Result<()> {
        if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
            return Err(io::Error::last_os_error());
        }
        if libc::syscall(SYS_RESTRICT_SELF, self.ruleset.as_raw_fd(), 0) != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

/// The rights to grant over a writable exemption.
///
/// Landlock refuses a rule that grants directory-only rights on a file — the
/// whole `add_rule` call fails with `EINVAL`, and since a missing exemption
/// is tolerated here, the failure is silent. That is how `/dev/null` came out
/// unwritable on the first run of this: it was being offered `MAKE_DIR`
/// alongside everything else, so its rule was never added at all.
fn writable_rights(abi: i32, path: &Path) -> u64 {
    if path.is_dir() {
        return handled_fs(abi);
    }
    let mut bits = FS_EXECUTE | FS_READ_FILE | FS_WRITE_FILE;
    if abi >= 3 {
        bits |= FS_TRUNCATE;
    }
    if abi >= 5 {
        bits |= FS_IOCTL_DEV;
    }
    bits
}

fn add_path_rule(ruleset: &OwnedFd, path: &Path, rights: u64) -> io::Result<()> {
    let c_path = CString::new(path.as_os_str().as_encoded_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains a NUL byte"))?;

    // O_PATH: we want a handle to name the directory, never to read it here.
    // SAFETY: `c_path` is a valid NUL-terminated string for the call's life.
    let parent = unsafe { libc::open(c_path.as_ptr(), libc::O_PATH | libc::O_CLOEXEC) };
    if parent < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `open` returned a fresh owned descriptor.
    let parent = unsafe { OwnedFd::from_raw_fd(parent) };

    let attr = PathBeneathAttr { allowed_access: rights, parent_fd: parent.as_raw_fd() };
    // SAFETY: `attr` outlives the call; its layout is the packed struct the
    // kernel documents for LANDLOCK_RULE_PATH_BENEATH.
    let rc = unsafe {
        libc::syscall(
            SYS_ADD_RULE,
            ruleset.as_raw_fd(),
            RULE_PATH_BENEATH,
            &attr as *const PathBeneathAttr,
            0,
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// Every enforcement test below is conditional on the kernel actually
    /// offering Landlock, so this one exists to make a kernel that does not
    /// visible rather than silently skipping the suite.
    #[test]
    fn availability_is_reported_rather_than_assumed() {
        match availability() {
            Availability::Enforcing { abi, .. } => assert!(abi >= 1),
            Availability::Unavailable { reason } => {
                eprintln!("landlock unavailable here: {reason} — enforcement tests skipped");
            }
        }
    }

    /// Deliberately **not** `tempfile::tempdir()`: that lands under `/tmp`,
    /// which is on the incidental-write list, so every assertion below would
    /// pass for the wrong reason — writes allowed by an exemption rather than
    /// denied by the sandbox. This cost a debugging round the first time.
    fn project() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("aldwin-sandbox-")
            .tempdir_in(env!("CARGO_MANIFEST_DIR"))
            .expect("a scratch project outside the incidental paths")
    }

    fn sandboxed<S: AsRef<std::ffi::OsStr>>(root: &Path, argv: &[S]) -> std::process::Output {
        let plan = ReadOnly::build(root).expect("ruleset builds");
        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..]).current_dir(root).stdin(std::process::Stdio::null());
        // SAFETY: two syscalls, no allocation — see `engage`.
        unsafe {
            std::os::unix::process::CommandExt::pre_exec(&mut cmd, move || plan.engage());
        }
        cmd.output().expect("spawns")
    }

    fn enforcing() -> bool {
        availability().enforcing()
    }

    #[test]
    fn a_read_still_reads() {
        if !enforcing() {
            return;
        }
        let dir = project();
        std::fs::write(dir.path().join("hello.txt"), "contents\n").unwrap();

        let out = sandboxed(dir.path(), &["/usr/bin/cat", "hello.txt"]);
        assert!(out.status.success(), "a read must still run: {out:?}");
        assert_eq!(String::from_utf8_lossy(&out.stdout), "contents\n");
    }

    /// The whole model in one assertion: a call that claimed to be a read
    /// cannot write, whatever it claimed.
    #[test]
    fn a_write_cannot_land() {
        if !enforcing() {
            return;
        }
        let dir = project();
        let victim = dir.path().join("untouched.txt");
        std::fs::write(&victim, "original\n").unwrap();

        let out = sandboxed(dir.path(), &["/usr/bin/tee", "untouched.txt"]);
        assert!(!out.status.success(), "writing must fail, not succeed quietly: {out:?}");
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "original\n", "nothing may land");
    }

    #[test]
    fn a_file_cannot_be_deleted_either() {
        if !enforcing() {
            return;
        }
        let dir = project();
        let victim = dir.path().join("keep.txt");
        std::fs::write(&victim, "still here\n").unwrap();

        let out = sandboxed(dir.path(), &["/usr/bin/rm", "-f", "keep.txt"]);
        assert!(!out.status.success(), "rm must fail: {out:?}");
        assert!(victim.exists(), "the file must survive");
    }

    /// Re-running after the developer allows it as a write is only safe
    /// because the first attempt could not have half-finished. This pins that
    /// the failed attempt leaves no partial file behind.
    #[test]
    fn a_refused_write_leaves_nothing_behind() {
        if !enforcing() {
            return;
        }
        let dir = project();

        let out = sandboxed(dir.path(), &["/usr/bin/tee", "brand-new.txt"]);
        assert!(!out.status.success());
        assert!(!dir.path().join("brand-new.txt").exists(), "a refused write creates nothing");
    }

    #[test]
    fn the_incidental_list_lets_a_read_use_dev_null() {
        if !enforcing() {
            return;
        }
        let dir = project();
        let out = sandboxed(dir.path(), &["/usr/bin/tee", "/dev/null"]);
        assert!(out.status.success(), "/dev/null must stay writable: {out:?}");
    }

    /// The other half of a read declaration: it cannot reach the network
    /// either, so a call that "only reads" cannot quietly send what it read
    /// somewhere. Landlock covers TCP from ABI 4; the listener is local so
    /// the test needs no internet, only the refusal.
    #[test]
    fn a_read_cannot_open_a_tcp_connection() {
        let Availability::Enforcing { network: true, .. } = availability() else {
            return;
        };
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a local port");
        let port = listener.local_addr().unwrap().port();

        let dir = project();
        let out = sandboxed(
            dir.path(),
            &["/usr/bin/curl", "--max-time", "5", "-s", &format!("http://127.0.0.1:{port}/")],
        );
        assert!(!out.status.success(), "a read must not reach the network: {out:?}");
    }

    /// `.git/` is deliberately absent from the incidental list, so that a
    /// `git commit` — which touches nothing outside it — cannot succeed while
    /// claiming to be a read.
    #[test]
    fn dot_git_is_not_writable_from_a_read() {
        if !enforcing() {
            return;
        }
        let dir = project();
        std::fs::create_dir(dir.path().join(".git")).unwrap();

        let out = sandboxed(dir.path(), &["/usr/bin/tee", ".git/smuggled"]);
        assert!(!out.status.success(), "a read must not write into .git: {out:?}");
        assert!(!dir.path().join(".git/smuggled").exists());
    }
}

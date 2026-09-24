//! Landlock, spoken directly to the kernel.
//!
//! Three syscalls and two structs, rather than a crate, for one reason that
//! matters here: the ruleset has to be **built in the parent and engaged in
//! the child**. Building it opens file descriptors and allocates; doing that
//! between `fork` and `execve` in a threaded process is how you get a child
//! that deadlocks in the allocator. So [`Sandbox::build`] does all of it up
//! front, and [`Sandbox::engage`] — the part that runs in the forked child —
//! is two syscalls with no allocation, which is what `pre_exec` permits.
//!
//! The ruleset *handles* every write right the kernel's ABI knows and no
//! read right. Handled-but-not-granted is denied, so everything handled is
//! denied everywhere except beneath the rules added for the workspace roots
//! and the incidental paths; everything not handled — execute, read a file,
//! list a directory — stays allowed everywhere.

use std::ffi::CString;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::{Path, PathBuf};

use super::incidental_writes;

const SYS_CREATE_RULESET: libc::c_long = 444;
const SYS_ADD_RULE: libc::c_long = 445;
const SYS_RESTRICT_SELF: libc::c_long = 446;

const CREATE_RULESET_VERSION: u32 = 1;
const RULE_PATH_BENEATH: libc::c_int = 1;

// Filesystem access bits, in the kernel's order. Bits 0–12 are ABI 1;
// REFER is ABI 2 and TRUNCATE ABI 3. The three read-side bits of ABI 1 —
// EXECUTE (0), READ_FILE (2), READ_DIR (3) — are deliberately absent.
const FS_WRITE_FILE: u64 = 1 << 1;
const FS_REMOVE_DIR: u64 = 1 << 4;
const FS_REMOVE_FILE: u64 = 1 << 5;
const FS_MAKE_CHAR: u64 = 1 << 6;
const FS_MAKE_DIR: u64 = 1 << 7;
const FS_MAKE_REG: u64 = 1 << 8;
const FS_MAKE_SOCK: u64 = 1 << 9;
const FS_MAKE_FIFO: u64 = 1 << 10;
const FS_MAKE_BLOCK: u64 = 1 << 11;
const FS_MAKE_SYM: u64 = 1 << 12;
const FS_REFER: u64 = 1 << 13;
const FS_TRUNCATE: u64 = 1 << 14;

const WRITE_ABI1: u64 = FS_WRITE_FILE
    | FS_REMOVE_DIR
    | FS_REMOVE_FILE
    | FS_MAKE_CHAR
    | FS_MAKE_DIR
    | FS_MAKE_REG
    | FS_MAKE_SOCK
    | FS_MAKE_FIFO
    | FS_MAKE_BLOCK
    | FS_MAKE_SYM;

/// Only the filesystem half of the kernel's attr. The kernel accepts any
/// size from this one field upward, so there is no per-ABI size to get
/// wrong — the network field ABI 4 added is simply not sent, which leaves
/// the network unrestricted, as ADR 0011 intends.
#[repr(C)]
struct RulesetAttr {
    handled_access_fs: u64,
}

#[repr(C, packed)]
struct PathBeneathAttr {
    allowed_access: u64,
    parent_fd: RawFd,
}

fn abi_version() -> io::Result<i32> {
    // SAFETY: the version probe is defined as a null attr, zero size, and the
    // VERSION flag; it reads nothing and returns the ABI number.
    let v = unsafe {
        libc::syscall(
            SYS_CREATE_RULESET,
            std::ptr::null::<RulesetAttr>(),
            0usize,
            CREATE_RULESET_VERSION,
        )
    };
    if v < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(v as i32)
}

pub fn unavailable() -> Option<&'static str> {
    match abi_version() {
        Ok(abi) if abi >= 1 => None,
        Ok(_) => Some("this kernel reports no usable Landlock ABI"),
        Err(e) if e.raw_os_error() == Some(libc::ENOSYS) => {
            Some("this kernel has no Landlock support")
        }
        Err(_) => Some("Landlock is present but disabled — add it to the kernel's lsm= list"),
    }
}

/// Every write right this ABI knows. A right must be *handled* before it can
/// be denied, so one left out here is a write the sandbox silently permits —
/// which is why this tracks the ABI upward.
///
/// On ABI 1, which cannot handle REFER, the kernel refuses every rename or
/// link across directories (`EXDEV`), inside the workspace too. Kernels
/// before 5.19 are rare enough that this is stated rather than worked round.
fn handled_fs(abi: i32) -> u64 {
    let mut bits = WRITE_ABI1;
    if abi >= 2 {
        bits |= FS_REFER;
    }
    if abi >= 3 {
        bits |= FS_TRUNCATE;
    }
    bits
}

/// The rights granted beneath one writable path. The kernel refuses a rule
/// that grants a directory-only right on a file — the whole `add_rule` fails
/// with `EINVAL` — which is how `/dev/null` once came out unwritable: it was
/// offered `MAKE_DIR` with everything else.
fn writable_rights(abi: i32, path: &Path) -> u64 {
    if path.is_dir() {
        return handled_fs(abi);
    }
    let mut bits = FS_WRITE_FILE;
    if abi >= 3 {
        bits |= FS_TRUNCATE;
    }
    bits
}

/// A built, unengaged ruleset: write only beneath the roots and the
/// incidental paths.
pub struct Sandbox {
    ruleset: OwnedFd,
}

impl Sandbox {
    /// Every root is writable, not only the project root: a second root
    /// (ADR 0007) is workspace on the same terms as the first.
    pub fn build(roots: &[PathBuf]) -> io::Result<Self> {
        let abi = abi_version()?;
        let attr = RulesetAttr {
            handled_access_fs: handled_fs(abi),
        };
        // SAFETY: `attr` outlives the call and its size is what we pass.
        let fd = unsafe {
            libc::syscall(
                SYS_CREATE_RULESET,
                &attr as *const RulesetAttr,
                std::mem::size_of::<RulesetAttr>(),
                0,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the syscall returned a fresh, owned file descriptor.
        let ruleset = unsafe { OwnedFd::from_raw_fd(fd as RawFd) };

        // A root that cannot be added is an error, not a skip: the workspace
        // would be read-only and every run would fail without saying why.
        for root in roots {
            add_path_rule(&ruleset, root, handled_fs(abi))?;
        }
        // An incidental path that does not exist here has nothing to exempt.
        for path in incidental_writes() {
            let _ = add_path_rule(&ruleset, &path, writable_rights(abi, &path));
        }
        Ok(Self { ruleset })
    }

    /// Unchanged on Linux: the confinement is engaged in the child, not by
    /// wrapping the program.
    pub fn command_line(&self, program: &str, args: &[String]) -> (String, Vec<String>) {
        (program.to_string(), args.to_vec())
    }

    /// Confines whatever `cmd` spawns.
    pub fn install(self, cmd: &mut tokio::process::Command) {
        // SAFETY: `engage` is two syscalls with no allocation, which is what
        // `pre_exec` permits — see its own safety note.
        unsafe {
            cmd.pre_exec(move || self.engage());
        }
    }

    /// Engage the ruleset on the calling process. Everything it goes on to
    /// `exec`, and every child of that, inherits it and cannot widen it.
    ///
    /// # Safety
    /// Intended for `pre_exec`, between `fork` and `execve`. It allocates
    /// nothing and takes no locks, which is what makes it safe there.
    /// Calling it on the parent would confine Aldwin itself, permanently.
    unsafe fn engage(&self) -> io::Result<()> {
        if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
            return Err(io::Error::last_os_error());
        }
        if libc::syscall(SYS_RESTRICT_SELF, self.ruleset.as_raw_fd(), 0) != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

fn add_path_rule(ruleset: &OwnedFd, path: &Path, rights: u64) -> io::Result<()> {
    let c_path = CString::new(path.as_os_str().as_encoded_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains a NUL byte"))?;

    // O_PATH: a handle to name the directory, never to read it here.
    // SAFETY: `c_path` is a valid NUL-terminated string for the call's life.
    let parent = unsafe { libc::open(c_path.as_ptr(), libc::O_PATH | libc::O_CLOEXEC) };
    if parent < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `open` returned a fresh owned descriptor.
    let parent = unsafe { OwnedFd::from_raw_fd(parent) };

    let attr = PathBeneathAttr {
        allowed_access: rights,
        parent_fd: parent.as_raw_fd(),
    };
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
    use crate::test_support::{confinement_or_explicit_skip, scratch_dir};
    use std::process::Command;

    fn confined(root: &Path, argv: &[&str]) -> std::process::Output {
        let sandbox =
            Sandbox::build(std::slice::from_ref(&root.to_path_buf())).expect("ruleset builds");
        let mut cmd = Command::new(argv[0]);
        cmd.args(&argv[1..])
            .current_dir(root)
            .stdin(std::process::Stdio::null());
        // SAFETY: two syscalls, no allocation — see `engage`.
        unsafe {
            std::os::unix::process::CommandExt::pre_exec(&mut cmd, move || sandbox.engage());
        }
        cmd.output().expect("spawns")
    }

    #[test]
    fn a_write_inside_the_workspace_lands() {
        if !confinement_or_explicit_skip() {
            return;
        }
        let root = scratch_dir();
        let out = confined(root.path(), &["/bin/sh", "-c", "mkdir d && echo hi > d/f"]);
        assert!(out.status.success(), "{out:?}");
        assert_eq!(
            std::fs::read_to_string(root.path().join("d/f")).unwrap(),
            "hi\n"
        );
    }

    #[test]
    fn a_write_outside_the_workspace_cannot_land() {
        if !confinement_or_explicit_skip() {
            return;
        }
        let root = scratch_dir();
        let outside = scratch_dir();
        let victim = outside.path().join("victim");
        std::fs::write(&victim, "original\n").unwrap();

        let target = victim.to_str().unwrap();
        for script in [
            format!("echo changed > {target}"),
            format!("rm -f {target}"),
            format!("mv {target} {target}.moved"),
        ] {
            let out = confined(root.path(), &["/bin/sh", "-c", &script]);
            assert!(!out.status.success(), "{script}: {out:?}");
            assert_eq!(std::fs::read_to_string(&victim).unwrap(), "original\n");
        }
        let out = confined(
            root.path(),
            &[
                "/bin/sh",
                "-c",
                &format!("touch {}/new", outside.path().display()),
            ],
        );
        assert!(!out.status.success(), "{out:?}");
        assert!(!outside.path().join("new").exists());
    }

    /// Rules are on the root's inode, not its name, so a symlink inside a
    /// root that points out of it leads to somewhere the rule does not cover.
    #[test]
    fn a_symlink_out_of_the_workspace_does_not_carry_write_access_with_it() {
        if !confinement_or_explicit_skip() {
            return;
        }
        let root = scratch_dir();
        let outside = scratch_dir();
        std::os::unix::fs::symlink(outside.path(), root.path().join("out")).unwrap();

        let out = confined(root.path(), &["/bin/sh", "-c", "echo x > out/smuggled"]);
        assert!(!out.status.success(), "{out:?}");
        assert!(!outside.path().join("smuggled").exists());
    }

    #[test]
    fn reads_outside_the_workspace_and_the_incidental_writes_still_work() {
        if !confinement_or_explicit_skip() {
            return;
        }
        let root = scratch_dir();
        let out = confined(
            root.path(),
            &[
                "/bin/sh",
                "-c",
                "cat /etc/passwd >/dev/null; echo x > /dev/null",
            ],
        );
        assert!(out.status.success(), "{out:?}");
    }

    #[test]
    fn a_file_rule_offers_only_file_rights() {
        assert_eq!(
            writable_rights(3, Path::new("/dev/null")),
            FS_WRITE_FILE | FS_TRUNCATE
        );
        assert_eq!(
            handled_fs(1) & (1 << 0 | 1 << 2 | 1 << 3),
            0,
            "no read right is handled, so reads stay open"
        );
    }
}

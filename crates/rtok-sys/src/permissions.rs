//! Inspect and tighten Unix data permissions without following links.

use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct Finding {
    pub path: PathBuf,
    pub before: Option<u32>,
    pub target: Option<u32>,
    pub repaired: bool,
    pub error: Option<String>,
}

/// Check a path, optionally descending through a dedicated data directory.
/// Missing paths need no repair. Unix permission bits do not describe Windows ACLs.
pub fn check(path: &Path, recursive: bool, apply: bool) -> Vec<Finding> {
    #[cfg(unix)]
    {
        unix::check(path, recursive, apply)
    }
    #[cfg(not(unix))]
    {
        let _ = (path, recursive, apply);
        Vec::new()
    }
}

#[cfg(unix)]
mod unix {
    use super::*;
    use rustix::fs::{self, FileType, Mode, OFlags};
    use std::io;
    use std::os::fd::{AsFd, OwnedFd};
    use std::path::Component;

    const FLAGS: OFlags = OFlags::RDONLY
        .union(OFlags::CLOEXEC)
        .union(OFlags::NOFOLLOW)
        .union(OFlags::NONBLOCK);
    const MAX_ENTRIES: usize = 100_000;

    fn open(path: &Path) -> io::Result<OwnedFd> {
        if !path.is_absolute() {
            return Err(io::Error::other("requires an absolute path"));
        }
        let mut fd = fs::open("/", FLAGS | OFlags::DIRECTORY, Mode::empty())?;
        let mut components = path.components().peekable();
        while let Some(component) = components.next() {
            match component {
                Component::RootDir => continue,
                Component::Normal(name) => {
                    let flags = if components.peek().is_some() {
                        FLAGS | OFlags::DIRECTORY
                    } else {
                        FLAGS
                    };
                    fd = fs::openat(&fd, name, flags, Mode::empty())?;
                }
                _ => return Err(io::Error::other("refusing non-normal path components")),
            }
        }
        Ok(fd)
    }

    fn failure(path: &Path, error: impl ToString) -> Finding {
        Finding {
            path: path.into(),
            before: None,
            target: None,
            repaired: false,
            error: Some(error.to_string()),
        }
    }

    pub(super) fn check(path: &Path, recursive: bool, apply: bool) -> Vec<Finding> {
        let mut out = Vec::new();
        match open(path) {
            Ok(fd) => visit(fd, path, recursive, apply, 0, &mut 0, &mut out),
            Err(e) if e.kind() == io::ErrorKind::NotFound => (),
            Err(e) => out.push(failure(path, e)),
        }
        out
    }

    fn visit(
        fd: OwnedFd,
        path: &Path,
        recursive: bool,
        apply: bool,
        depth: usize,
        count: &mut usize,
        out: &mut Vec<Finding>,
    ) {
        *count += 1;
        if *count > MAX_ENTRIES || depth > 64 {
            out.push(failure(
                path,
                "permission scan limit reached; remaining paths were not checked",
            ));
            return;
        }
        let result = (|| -> io::Result<()> {
            let stat = fs::fstat(&fd)?;
            let kind = FileType::from_raw_mode(stat.st_mode);
            let directory = kind == FileType::Directory;
            if !directory && kind != FileType::RegularFile {
                return Err(io::Error::other("refusing a non-regular data entry"));
            }
            if stat.st_uid != rustix::process::geteuid().as_raw() {
                return Err(io::Error::other("not owned by the current user"));
            }
            if !directory && stat.st_nlink != 1 {
                return Err(io::Error::other("refusing a multiply-linked file"));
            }
            let target = if directory { 0o700 } else { 0o600 };
            let before = stat.st_mode & 0o7777;
            if before != target {
                // mode_t is u16 on macOS and u32 on Linux.
                #[allow(clippy::useless_conversion)]
                let mut finding = Finding {
                    path: path.into(),
                    before: Some(u32::from(before)),
                    target: Some(u32::from(target)),
                    repaired: false,
                    error: None,
                };
                if apply {
                    match fs::fchmod(&fd, Mode::from_raw_mode(target)) {
                        Ok(()) => match fs::fstat(&fd) {
                            Ok(after) if after.st_mode & 0o7777 == target => {
                                finding.repaired = true
                            }
                            Ok(_) => {
                                finding.error = Some("mode changed during verification".into())
                            }
                            Err(e) => finding.error = Some(e.to_string()),
                        },
                        Err(e) => finding.error = Some(e.to_string()),
                    }
                }
                let failed = finding.error.is_some();
                out.push(finding);
                if failed {
                    return Ok(());
                }
            }
            if recursive && directory {
                let entries = fs::Dir::read_from(fd.as_fd())?;
                for entry in entries {
                    let entry = entry?;
                    let name = entry.file_name();
                    if name.to_bytes() == b"." || name.to_bytes() == b".." {
                        continue;
                    }
                    if *count >= MAX_ENTRIES {
                        out.push(failure(
                            path,
                            "permission scan limit reached; remaining paths were not checked",
                        ));
                        break;
                    }
                    use std::os::unix::ffi::OsStrExt;
                    let child = path.join(std::ffi::OsStr::from_bytes(name.to_bytes()));
                    match fs::openat(&fd, name, FLAGS, Mode::empty()) {
                        Ok(child_fd) => visit(child_fd, &child, true, apply, depth + 1, count, out),
                        Err(e) => {
                            *count += 1;
                            out.push(failure(&child, e));
                        }
                    }
                }
            }
            Ok(())
        })();
        if let Err(e) = result {
            out.push(failure(path, e));
        }
    }
}

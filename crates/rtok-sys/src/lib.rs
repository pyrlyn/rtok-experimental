//! Cross-platform process and file-lock helpers for rtok.
//!
//! File locks go through [`fs4`] (safe API). Process helpers use rustix on Unix
//! and `windows-sys` on Windows. This crate allows `unsafe_code`; the workspace
//! lint denies it everywhere else.

#![allow(unsafe_code)]

use std::fs::File;
use std::io;

pub mod proc;

/// Block until an exclusive lock is held on `file`.
pub fn lock_exclusive(file: &File) -> io::Result<()> {
    file.lock()
}

/// Non-blocking exclusive lock. `Ok(true)` acquired; `Ok(false)` held elsewhere.
pub fn try_lock_exclusive(file: &File) -> io::Result<bool> {
    match file.try_lock() {
        Ok(()) => Ok(true),
        Err(std::fs::TryLockError::WouldBlock) => Ok(false),
        Err(std::fs::TryLockError::Error(err)) => Err(err),
    }
}

/// Release a previously acquired exclusive lock.
pub fn unlock(file: &File) -> io::Result<()> {
    file.unlock()
}

/// True while `pid` names a live process.
pub fn process_alive(pid: i32) -> bool {
    #[cfg(unix)]
    {
        use rustix::process::Pid;
        (pid > 0)
            .then(|| Pid::from_raw(pid))
            .flatten()
            .is_some_and(|p| rustix::process::test_kill_process(p).is_ok())
    }
    #[cfg(windows)]
    {
        win::alive(pid)
    }
}

/// Ask a process to exit politely (Unix TERM) or terminate (Windows).
pub fn process_term(pid: i32) {
    #[cfg(unix)]
    {
        use rustix::process::{Pid, Signal};
        if let Some(p) = (pid > 0).then(|| Pid::from_raw(pid)).flatten() {
            let _ = rustix::process::kill_process(p, Signal::TERM);
        }
    }
    #[cfg(windows)]
    {
        win::terminate(pid);
    }
}

/// Force-kill a process (Unix KILL / Windows TerminateProcess).
pub fn process_kill(pid: i32) {
    #[cfg(unix)]
    {
        use rustix::process::{Pid, Signal};
        if let Some(p) = (pid > 0).then(|| Pid::from_raw(pid)).flatten() {
            let _ = rustix::process::kill_process(p, Signal::KILL);
        }
    }
    #[cfg(windows)]
    {
        win::terminate(pid);
    }
}

/// This process's parent pid, to notice being orphaned: it changes when the parent exits
/// and the kernel reparents us (to 1, or a subreaper). `None` on Windows, where a parent
/// pid is never updated and so says nothing about the parent still being there.
pub fn parent_pid() -> Option<i32> {
    #[cfg(unix)]
    {
        rustix::process::getppid().map(|p| p.as_raw_nonzero().get())
    }
    #[cfg(windows)]
    {
        None
    }
}

/// The parent pid of any process `pid`, read without spawning one (the hook client's budget
/// leaves no room for `ps`). `None` when it cannot be read, the process is gone, or the
/// platform has no reliable answer (Windows: a parent pid is never updated after the parent
/// exits, so it says nothing).
pub fn parent_of(pid: i32) -> Option<i32> {
    if pid <= 0 {
        return None;
    }
    #[cfg(target_os = "linux")]
    {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        // `comm` (field 2) may hold spaces and parentheses, so fields count from the last `)`.
        let rest = stat.get(stat.rfind(')')? + 1..)?;
        rest.split_whitespace().nth(1)?.parse().ok()
    }
    #[cfg(target_os = "macos")]
    {
        let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
        // SAFETY: `info` is a writable buffer of exactly `size` bytes, and the call fills it
        // completely or returns a length other than `size`, which is rejected below.
        let got = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                info.as_mut_ptr().cast(),
                size,
            )
        };
        // SAFETY: a full-size return means the kernel initialised the whole struct.
        (got == size).then(|| unsafe { info.assume_init() }.pbi_ppid as i32)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

/// Up to `max` ancestors of `pid`, nearest first, not counting `pid` itself. Stops at pid 1
/// (the init process says nothing about a host), at an unreadable link, and at a loop.
pub fn ancestors(pid: i32, max: usize) -> Vec<i32> {
    let mut chain = Vec::new();
    let mut at = pid;
    while chain.len() < max {
        match parent_of(at) {
            Some(p) if p > 1 && p != pid && !chain.contains(&p) => {
                chain.push(p);
                at = p;
            }
            _ => break,
        }
    }
    chain
}

/// Become a session leader so closing the terminal does not take the tree down.
/// No-op on Windows.
pub fn setsid() {
    #[cfg(unix)]
    {
        let _ = rustix::process::setsid();
    }
}

/// Windows only: stop the next spawned child from inheriting this process's own stdio
/// handles. `CreateProcess` inherits every already-inheritable handle the caller holds once a
/// child is given any redirected stdio — not just the handles that child was assigned — so a
/// supervisor spawned with `Stdio::null()` still keeps alive whatever piped our own stdout or
/// stderr (a test harness's `Command::output()`, or any other parent that captured us). Held by
/// a process meant to outlive us, that pipe's write end never closes, so the reader waiting on
/// EOF (`output()`/`wait_with_output()`) blocks long after both it and we have exited — the
/// hang behind T83.3. On Unix the equivalent fds are close-on-exec by default, so this is a
/// no-op there.
pub fn stop_inheriting_own_stdio() {
    #[cfg(windows)]
    {
        win::stop_inheriting_std_handles();
    }
}

#[cfg(windows)]
mod win {
    use windows_sys::Win32::Foundation::{
        CloseHandle, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, STILL_ACTIVE, SetHandleInformation,
    };
    use windows_sys::Win32::System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
        TerminateProcess,
    };

    pub fn stop_inheriting_std_handles() {
        for id in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            let handle = unsafe { GetStdHandle(id) };
            if handle.is_null() || handle == INVALID_HANDLE_VALUE {
                continue;
            }
            unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) };
        }
    }

    pub fn alive(pid: i32) -> bool {
        if pid <= 0 {
            return false;
        }
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid as u32) };
        if handle.is_null() {
            return false;
        }
        let mut code = 0u32;
        let ok = unsafe { GetExitCodeProcess(handle, &mut code) };
        unsafe { CloseHandle(handle) };
        ok != 0 && code == STILL_ACTIVE as u32
    }

    pub fn terminate(pid: i32) {
        if pid <= 0 {
            return;
        }
        let handle = unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid as u32) };
        if handle.is_null() {
            return;
        }
        let _ = unsafe { TerminateProcess(handle, 1) };
        unsafe { CloseHandle(handle) };
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests {
    use super::*;

    #[test]
    fn a_process_reads_its_own_parent_and_a_chain_starts_there() {
        let me = std::process::id() as i32;
        assert_eq!(parent_of(me), parent_pid());
        let chain = ancestors(me, 3);
        assert!(chain.len() <= 3 && !chain.contains(&me) && !chain.contains(&1));
        assert_eq!(chain.first().copied(), parent_pid().filter(|p| *p > 1));
    }

    #[test]
    fn a_missing_or_invalid_pid_has_no_parent() {
        assert_eq!(parent_of(0), None);
        assert_eq!(parent_of(-5), None);
        assert!(ancestors(0, 3).is_empty());
    }
}

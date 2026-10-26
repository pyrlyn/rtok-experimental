// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! A fake language server for T142's LSP-orphan tests (unix-only): a script named
//! `clangd`, first on `PATH`. It answers `--version` (the readiness probe in
//! `lsp::on_path`) with exit 0, and in server mode speaks just enough LSP over stdio
//! (Content-Length framing) to satisfy `lsp.rs`'s 40s `READY` poll on the first reply
//! instead of timing it out.
//!
//! Liveness, without touching real host processes: the creator has banned `ps`/`kill -0`/
//! `kill` on a pid in these tests (this host runs many live rtok processes and pids get
//! reused, so a pid check or a kill can hit the wrong process). Instead the fake takes an
//! exclusive `flock` (Perl's `flock`, the same `flock(2)` syscall `std::fs::File::lock`
//! uses on Unix, via [`rtok_sys::try_lock_exclusive`]) on a lock file inside the test's own
//! temp dir and holds it for its whole life. A test proves the fake is gone by acquiring
//! that same lock itself — the OS releases a `flock` the instant the holding process's file
//! descriptors are torn down, for *any* reason (normal exit, `exit()`, or being `SIGKILL`ed
//! by a correctly shut-down LSP session), so this is exact and needs no signal.
//!
//! Protocol: every request gets `{"result":null}` except `initialize` (`{"capabilities":{}}`)
//! and `workspace/symbol` (a one-item array naming the query, so `wait_def`/`symbol_text`
//! resolve immediately) — unless the query is `BoomTrigger`, which answers with a JSON-RPC
//! error so callers can exercise the error path after the server has already started.
//!
//! Exit conditions, weakest to strongest:
//! - an explicit `exit` notification (what a correctly shut-down session sends, right before
//!   it also `SIGKILL`s the child — see `Drop for Session` in `src/plugins/graph/lsp.rs`)
//!   ends it immediately;
//! - otherwise, stdin closing (EOF, which also happens when a *buggy* caller simply exits
//!   without shutting the session down — the pipe closes either way) starts a short grace
//!   sleep before it exits on its own. The grace is deliberately longer than the ~2s window
//!   the tests give a liveness check: that gap is T142's actual bug made observable — with
//!   the guard removed, nothing sends `exit` or kills the child, so within the 2s check the
//!   fake is still there holding its lock;
//! - a hard self-timeout from process start always wins regardless, via Perl's own `alarm`,
//!   so a red test can never leave a process running past it.

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Writes the fake `clangd` executable into `bin_dir` (created if needed) and returns its
/// path. Put `bin_dir` first on `PATH` so `lsp::pick`'s bare `clangd` resolves to it instead
/// of any real clangd on the machine.
pub fn write_fake_clangd(bin_dir: &Path) -> PathBuf {
    fs::create_dir_all(bin_dir).unwrap();
    let path = bin_dir.join("clangd");
    fs::write(&path, FAKE_CLANGD).unwrap();
    let mut perm = fs::metadata(&path).unwrap().permissions();
    perm.set_mode(0o755);
    fs::set_permissions(&path, perm).unwrap();
    path
}

/// The lock file a fake started with `FAKE_LSP_LOCK_FILE=<this path>` holds exclusively for
/// its whole life. Tests poll this file rather than the fake's pid.
pub fn lock_path(dir: &Path) -> PathBuf {
    dir.join("fake-lsp.lock")
}

/// Whether `path` is currently held by another process: opens (creating if needed) and
/// immediately tries a non-blocking exclusive lock, releasing it again on return if we
/// happened to acquire it — we only ever want to read the lock's state, not hold it.
fn locked_elsewhere(path: &Path) -> io::Result<bool> {
    let file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(path)?;
    Ok(!rtok_sys::try_lock_exclusive(&file)?)
}

/// Polls up to `timeout` for the fake holding `lock_path` to have released it (i.e. to have
/// exited, by any means). Returns whether it was gone by the deadline.
pub fn wait_for_fake_death(lock_path: &Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if matches!(locked_elsewhere(lock_path), Ok(false)) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

const FAKE_CLANGD: &str = r#"#!/usr/bin/env perl
use strict;
use warnings;
use Fcntl qw(:flock);

if (@ARGV && $ARGV[0] eq '--version') {
    print "fake clangd version 1.0.0\n";
    exit 0;
}

# Hard self-timeout: whatever else happens, this process is gone within ~15s. Perl's
# default SIGALRM disposition terminates the process, so no extra code is needed.
alarm(15);


# Declared here, at top level, and never let go out of scope before `exit`: a lexically
# scoped `open(my $fh, ...)` inside the `if` below would drop its only reference (and so
# close the handle and release the flock) the moment that block ended -- within
# microseconds of acquiring it, defeating the whole point.
our $lock_fh;
if (defined $ENV{FAKE_LSP_LOCK_FILE}) {
    open($lock_fh, '>', $ENV{FAKE_LSP_LOCK_FILE}) or die "lock file: $!";
    flock($lock_fh, LOCK_EX) or die "flock: $!";
    # Keep $lock_fh open (and so the lock held) for the rest of this process's life. It is
    # never explicitly unlocked here: the OS releases it the moment this process's file
    # descriptors go away, whether that is this script reaching `exit`, or `rtok` SIGKILLing
    # it from `Drop for Session` — exactly the event the tests wait on.
}

binmode(STDIN);
binmode(STDOUT);
$| = 1;

sub read_msg {
    my $len;
    while (1) {
        my $line = readline(STDIN);
        return undef unless defined $line;
        $line =~ s/\r?\n\z//;
        last if $line eq '';
        if ($line =~ /^Content-Length:\s*(\d+)/i) {
            $len = $1;
        }
    }
    return undef unless defined $len;
    my $body = '';
    my $got = read(STDIN, $body, $len);
    return undef unless defined $got && $got == $len;
    return $body;
}

sub write_msg {
    my ($body) = @_;
    print STDOUT "Content-Length: " . length($body) . "\r\n\r\n" . $body;
}

while (1) {
    my $body = read_msg();
    if (!defined $body) {
        # Stdin EOF: a correctly shut-down parent already sent `exit` and SIGKILLed us
        # before this point ever matters. A buggy parent just exited, which also closes
        # this pipe -- so react to plain EOF too, but only after a grace period longer
        # than the liveness check's ~2s deadline, so a missing shutdown still shows up as
        # "still alive" there. The alarm(15) above is the absolute backstop either way.
        sleep(5);
        last;
    }
    my ($method) = $body =~ /"method"\s*:\s*"([^"]*)"/;
    $method = '' unless defined $method;
    last if $method eq 'exit';
    my ($id) = $body =~ /"id"\s*:\s*(\d+)/;
    next unless defined $id;

    if ($method eq 'initialize') {
        write_msg(qq({"jsonrpc":"2.0","id":$id,"result":{"capabilities":{}}}));
    } elsif ($method eq 'workspace/symbol') {
        my ($query) = $body =~ /"query"\s*:\s*"([^"]*)"/;
        $query = '' unless defined $query;
        if ($query eq 'BoomTrigger') {
            write_msg(qq({"jsonrpc":"2.0","id":$id,"error":{"code":-32000,"message":"boom"}}));
        } else {
            my $uri = $ENV{FAKE_LSP_FILE_URI} // 'file:///tmp/main.c';
            my $result = qq([{"name":"$query","kind":12,"location":{"uri":"$uri",)
                . qq("range":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}}}}]);
            write_msg(qq({"jsonrpc":"2.0","id":$id,"result":$result}));
        }
    } else {
        write_msg(qq({"jsonrpc":"2.0","id":$id,"result":null}));
    }
}
exit 0;
"#;

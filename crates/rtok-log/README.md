# rtok-log

A small rotating text log other programs can depend on. No config crate, no database.
The caller owns the path and the level floor.

```toml
rtok-log = { path = "crates/rtok-log" }
```

From outside this repository, vendor the crate or depend on it by path. It is not published.

## What a line looks like

`<YYYY-MM-DD HH:MM:SS> <level> <source>/<name>: <message>`, UTC. A newline inside
the message becomes a space, so one event stays one line.

```rust
use rtok_log::{append, FileLog};

let path = std::env::temp_dir().join("app.log");
let log = FileLog {
    path: &path,
    max_bytes: 1_048_576,
    files: 5,
    level: "info",
};
append(&log, "info", "app", "boot", "started");
```

## What it does

- `level` is a floor: `error` and `warn` pass when the floor is `warn`; `info` does not.
  An unknown level is kept (it ranks as severe as `error`).
- When the next line would pass `max_bytes`, the live file becomes `app.log.1`, `.1`
  becomes `.2`, and a file past `files` is deleted. `files = 0` truncates the live file
  and keeps no history.
- Two processes appending the same path share a lock beside the log
  (`app.log.rotate.lock`), created owner-only and without following symlinks on Unix.
  The size is checked again under that lock, so a
  second process does not rotate a file the first one just replaced. The lock is not
  the log file itself: Windows refuses to rename a file this process still has locked.
- If the rotation lock is busy or rotation fails, the line is appended without rotating.
  The size limit is best effort in that case; a rotation failure must not discard a log line.
- [`write_line`] returns errors creating the directory or appending the line.
  [`append`] swallows them: a log line is not worth failing the caller.

## Errors

`error` lines are copied into `errors.log` next to the live file (`rtok.log` and
`errors.log` in the same directory). The bytes of that line match the general log.
`warn`, `info` and `debug` are not copied. A warning is a general-log line; an error
is both.

## What it does not do

Coloured stderr, a database row, or a viewer. Those stay in the program that embeds it.
rtok mirrors each line to the `log` facade and, when asked, inserts the same text into
its `logs` table before and after this crate runs.

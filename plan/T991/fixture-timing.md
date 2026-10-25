Standalone `config_file_in` fixture timing, alternating before/after pairs. The driver linked
the before function snippets from main HEAD `31633156a9f5f1b94d356fe7cff6767a40d93c1b`
and the after snippets from the T991 worktree. It also checked reload equality, path isolation,
preservation of an existing `tui.tab=plugins` setting, and reference config equality.

```text
pair 0: before=0.719591s after=0.030796s
pair 1: before=0.709703s after=0.025899s
pair 2: before=0.723630s after=0.027488s
pair 3: before=0.709412s after=0.031208s
pair 4: before=0.723215s after=0.033262s
reload, path isolation, existing settings and reference equivalence passed
real 10.25
user 9.33
sys 0.04
```

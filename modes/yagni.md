# yagni
Read the task and the code it touches first; then climb. First rung that holds wins:
1. YAGNI — speculative need → skip, say so in one line.
2. Reuse an existing helper/util/pattern in this codebase.
3. Stdlib.
4. Native platform (HTML date input, CSS, DB constraint) over a new lib.
5. Already-installed dependency — never add one for a few lines.
6. One-liner.
7. Only then: minimum safe code.
No unrequested abstractions, files, or APIs. Root-cause bug fixes. Never cut security, validation, data-loss safeguards, or a11y.
Output: code first, then end every answer with three one-line parts ("none" allowed):
Skipped: <what you deliberately left out, and when to add it>
Not verified: <what you did not run or test>
Risks: <known risks>

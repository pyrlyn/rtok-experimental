---
status: in progress
priority: P2
complexity: 3
readiness: 0%
agent: 92b1bcc9-6c0f-4ea7-ad76-d74268fcc6ac
---

# T989. Prevent transcript scans from hanging on unreadable inputs

Why: the Claude JSONL parser continues after persistent read errors, which can loop forever on a directory, while the shared transcript walker admits non-regular `*.jsonl` entries such as FIFOs that block when opened. Transcript cache compaction lookup also scans every compaction for every skill even though parser output is sorted.

Done means: make JSONL parsing terminate on persistent I/O errors; skip non-regular transcript files, including symlinks to non-regular targets; use binary search (`partition_point`) for the sorted compaction boundary lookup while preserving results; add focused regression tests and run `just check`.

Check: `just check`.

// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! One insta snapshot for a stable text rendering: `compress_prose` at Full intensity.
//! The fence body must survive byte-identical while the prose around it is stripped.

use rtok::modes::{CaveIntensity, compress_prose};

#[test]
fn full_compress_rendering_is_stable() {
    let input = "Sure! I'd be happy to help. The bug is just really in the auth middleware.\n```rs\nfn check(t: u64) { assert!(t <= MAX); }\n```\nFix the comparison.";
    insta::assert_snapshot!(compress_prose(input, CaveIntensity::Full));
}

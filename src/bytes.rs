// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Parsing of binary byte sizes (`10MB`, `1.5 GB`); printing them is `change_preview::human_bytes`.

/// `10MB`, `1.5 GB` or a bare `512` (bytes): the inverse of [`change_preview::human_bytes`], binary units.
pub fn parse_bytes(s: &str) -> Result<u64, String> {
    let t = s.trim().to_ascii_lowercase();
    let (num, unit) = t.split_at(
        t.find(|c: char| !c.is_ascii_digit() && c != '.')
            .unwrap_or(t.len()),
    );
    let n: f64 = num
        .parse()
        .map_err(|_| format!("not a size: {s} (try 10MB)"))?;
    let shift = match unit.trim() {
        "" | "b" => 0,
        "k" | "kb" => 1,
        "m" | "mb" => 2,
        "g" | "gb" => 3,
        "t" | "tb" => 4,
        _ => return Err(format!("unknown size unit in {s} (B, KB, MB, GB, TB)")),
    };
    Ok((n * 1024f64.powi(shift)) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use change_preview::human_bytes;

    #[test]
    fn sizes_parse_in_binary_units_and_bad_ones_are_named() {
        assert_eq!(parse_bytes("512"), Ok(512));
        assert_eq!(parse_bytes("10MB"), Ok(10 << 20));
        assert_eq!(parse_bytes("1.5 gb"), Ok(3 << 29));
        assert!(parse_bytes("ten").is_err());
        assert!(parse_bytes("5 parsecs").is_err());
        assert_eq!(parse_bytes(&human_bytes(2 << 20)), Ok(2 << 20));
    }
}

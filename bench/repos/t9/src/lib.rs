// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T9.1 bench fixture: a counter and a port parser. `Counter::count_to` has a seeded
//! off-by-one bug, so `counts_to_three` fails until the `fix-bug` task fixes it.

/// Counts from 1 upwards.
#[derive(Default)]
pub struct Counter {
    pub value: u32,
}

impl Counter {
    /// Every value from 1 to `n`, inclusive.
    pub fn count_to(&mut self, n: u32) -> Vec<u32> {
        let mut out = Vec::new();
        while self.value < n - 1 {
            self.value += 1;
            out.push(self.value);
        }
        out
    }
}

/// Parses a TCP port, rejecting 0 and anything above 65535.
pub fn parse_port(s: &str) -> Option<u16> {
    match s.trim().parse::<u16>() {
        Ok(0) | Err(_) => None,
        Ok(p) => Some(p),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_to_three() {
        assert_eq!(Counter::default().count_to(3), vec![1, 2, 3]);
    }
}

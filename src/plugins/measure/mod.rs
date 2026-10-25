// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `measure` — session-log ingest, proxy usage capture, `rtok stats` / `rtok bench` (plan P1, P9).
//!
//! Spec: the catalogue in `plan.md` §1 names the tools this replaces; none is a
//! dependency (D6) — the behaviour is re-implemented here.

use rtok_plugin_sdk::{DashboardPage, Manifest, Plugin, Surface};

pub struct MeasurePlugin;

impl Plugin for MeasurePlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "measure",
            surfaces: &[Surface::Cli, Surface::Proxy],
            default_on: true,
        }
    }

    fn dashboard_page(&self) -> DashboardPage {
        DashboardPage::new(
            "Measure",
            "Only Measurement rows count as savings. Stats from transcripts and proxy usage.",
            false,
        )
    }
}

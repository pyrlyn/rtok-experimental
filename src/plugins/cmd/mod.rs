// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `cmd` — archive, filter and measure every Bash output via `rtok run` (plan P3).
//!
//! Spec: the catalogue in `plan.md` §1 names the tools this replaces; none is a
//! dependency (D6) — the behaviour is re-implemented here.

use rtok_plugin_sdk::{Ctx, DashboardPage, Manifest, Plugin, PreToolDecision, PreToolUse, Surface};

pub mod bounded;
pub mod catfile;
pub mod compact;
pub mod diffs;
pub mod hook;
pub mod logfmt;

pub mod filter;
pub mod formatters;
pub mod gh;
pub mod grep;
pub mod loops;
pub mod mise;
pub mod python;
pub mod rules;
pub mod run;

pub struct CmdPlugin;

impl Plugin for CmdPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "cmd",
            surfaces: &[Surface::Hook, Surface::Cli],
            default_on: true,
        }
    }

    fn dashboard_page(&self) -> DashboardPage {
        DashboardPage::new(
            "Bash / cmd",
            "Archive and filter command output; expand the original by id.",
            true,
        )
    }

    fn pre_tool(&self, ev: &PreToolUse, cx: &Ctx) -> Option<PreToolDecision> {
        hook::pre_tool(ev, cx)
    }
}

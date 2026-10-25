// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! `proxy` — `ANTHROPIC_BASE_URL` passthrough with SSE streaming and usage capture (plan P5).
//!
//! Spec: the catalogue in `plan.md` §1 names the tools this replaces; none is a
//! dependency (D6) — the behaviour is re-implemented here.

use rtok_plugin_sdk::{DashboardPage, Manifest, Plugin, Surface};

pub struct ProxyPlugin;

impl Plugin for ProxyPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "proxy",
            surfaces: &[Surface::Proxy],
            default_on: true,
        }
    }

    fn dashboard_page(&self) -> DashboardPage {
        DashboardPage::new(
            "Proxy usage",
            "Record provider usage; compress mode runs plugin proxy_filter.",
            true,
        )
    }
}

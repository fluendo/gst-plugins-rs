// Copyright (C) 2025, Fluendo S.A.
//      Author: Diego Nieto <dnieto@fluendo.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v2.0.
// If a copy of the MPL was not distributed with this file, You can obtain one at
// <https://mozilla.org/MPL/2.0/>.
//
// SPDX-License-Identifier: MPL-2.0

use gst::glib;

mod comparemixer;
mod encoderstats;
mod videoencoderstats;
pub mod videoencoderstatsmeta;

fn plugin_init(plugin: &gst::Plugin) -> Result<(), glib::BoolError> {
    comparemixer::register(plugin)?;
    encoderstats::register(plugin)?;
    Ok(())
}

gst::plugin_define!(
    videostats,
    env!("CARGO_PKG_DESCRIPTION"),
    plugin_init,
    concat!(env!("CARGO_PKG_VERSION"), "-", "commit-id"),
    "MPL",
    env!("CARGO_PKG_NAME"),
    env!("CARGO_PKG_NAME"),
    env!("CARGO_PKG_REPOSITORY"),
    env!("BUILD_REL_DATE")
);

// Copyright (C) 2025, Fluendo S.A.
//      Author: Diego Nieto <dnieto@fluendo.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v2.0.
// If a copy of the MPL was not distributed with this file, You can obtain one at
// <https://mozilla.org/MPL/2.0/>.
//
// SPDX-License-Identifier: MPL-2.0

use gst::prelude::*;
use anyhow::Error;

fn main() -> Result<(), Error> {
    gst::init()?;

    let pipeline = gst::parse::launch("videotestsrc ! video/x-raw,width=640,height=480 ! videoconvert ! tee name=tee ! queue name=encq0 ! video-encoder-stats encoder=\"x264enc\" ! decodebin3 name=dec0 tee. ! queue name=encq1 ! video-encoder-stats encoder=\"x264enc bitrate=32\" ! decodebin3 name=dec1 video-compare-mixer name=mixer backend=CPU dec0. ! mixer.  dec1. ! mixer.  mixer. ! videoconvert ! autovideosink")?;
    pipeline.set_state(gst::State::Playing)?;

    let bus = pipeline.bus().unwrap();
    while let Some(msg) = bus.timed_pop(gst::ClockTime::NONE) {
        use gst::MessageView;
        match msg.view() {
            MessageView::Eos(..) => {
                break;
            }
            MessageView::Error(..) => unreachable!(),
            _ => (),
        }
    }

    pipeline.set_state(gst::State::Null)?;

    Ok(())
}

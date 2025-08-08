// Copyright (C) 2025, Fluendo S.A.
//      Author: Diego Nieto <dnieto@fluendo.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v2.0.
// If a copy of the MPL was not distributed with this file, You can obtain one at
// <https://mozilla.org/MPL/2.0/>.
//
// SPDX-License-Identifier: MPL-2.0

use anyhow::Error;
use gst::prelude::*;

fn main() -> Result<(), Error> {
    gst::init()?;

    let pipeline = gst::parse::launch("souphttpsrc location=\"https://ftp.nluug.nl/pub/graphics/blender/demo/movies/ToS/tears_of_steel_1080p.mov\" ! qtdemux name=demux demux.video_0 ! queue ! decodebin3 ! videoconvertscale ! capsfilter caps=\"video/x-raw,width=720,aspect-ratio=1/1\" ! tee name=tee ! queue name=encq0 ! video-encoder-stats encoder=\"x264enc\" parser=\"h264parse\" decoder=\"avdec_h264\" ! decodebin3 name=dec0 tee. ! queue name=encq1 ! video-encoder-stats encoder=\"x264enc bitrate=512\" ! decodebin3 name=dec1 video-compare-mixer split-screen=false backend=OpenGL name=mixer dec0. ! mixer.sink_0  dec1. ! mixer.sink_1  mixer. ! autovideosink")?;
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

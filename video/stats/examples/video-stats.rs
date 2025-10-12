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
use std::env;

fn print_usage() {
    println!("Usage: video-stats [PIPELINE_TYPE]");
    println!();
    println!("Pipeline types:");
    println!("  default        - Default pipeline with VMAF stats and decoder request pads");
    println!("  zero-latency   - Zero latency pipeline with ultrafast encoding, no VMAF");
    println!("  split-screen   - Split screen comparison using video-compare-mixer");
    println!("  no-vmaf        - Pipeline without VMAF statistics");
    println!();
    println!("If no argument is provided, 'default' pipeline will be used.");
}

fn get_pipeline_string(pipeline_type: &str) -> Result<&'static str, Error> {
    match pipeline_type {
        "default" => Ok("souphttpsrc location=\"https://ftp.nluug.nl/pub/graphics/blender/demo/movies/ToS/tears_of_steel_1080p.mov\" ! qtdemux name=demux demux.video_0 ! queue ! decodebin3 ! videoconvertscale ! capsfilter caps=\"video/x-raw,aspect-ratio=1/1\" ! tee name=tee ! video-encoder-stats encoder=\"x264enc bitrate=1024\" decoder=\"h264parse ! avdec_h264\" name=vs0 ! fakesink tee. ! video-encoder-stats encoder=\"x264enc bitrate=512\" name=vs1 ! fakesink video-compare-mixer split-screen=false backend=CPU name=mixer vs0.decoder_src ! mixer.sink_0  vs1.decoder_src ! mixer.sink_1  mixer. ! autovideosink"),

        "zero-latency" => Ok("souphttpsrc location=\"https://ftp.nluug.nl/pub/graphics/blender/demo/movies/ToS/tears_of_steel_1080p.mov\" ! qtdemux name=demux demux.video_0 ! queue ! decodebin3 ! videoconvertscale ! capsfilter caps=\"video/x-raw,aspect-ratio=1/1\" ! tee name=tee ! video-encoder-stats encoder=\"x264enc bitrate=1024 tune=zerolatency speed-preset=ultrafast threads=4\" name=vs0 vmaf-stats=false tee. ! video-encoder-stats encoder=\"x264enc bitrate=512 tune=zerolatency speed-preset=ultrafast\" name=vs1 video-compare-mixer split-screen=true backend=CPU name=mixer vs0.src ! h264parse ! avdec_h264 ! mixer.sink_0  vs1.src ! h264parse ! avdec_h264 ! mixer.sink_1  mixer. ! autovideosink"),

        "split-screen" => Ok("souphttpsrc location=\"https://ftp.nluug.nl/pub/graphics/blender/demo/movies/ToS/tears_of_steel_1080p.mov\" ! qtdemux name=demux demux.video_0 ! queue ! decodebin3 ! videoconvertscale ! capsfilter caps=\"video/x-raw,aspect-ratio=1/1\" ! tee name=tee ! video-encoder-stats encoder=\"x264enc\" decoder=\"h264parse ! avdec_h264\" name=vs0  tee. ! video-encoder-stats encoder=\"x264enc bitrate=512\" name=vs1  video-compare-mixer split-screen=true backend=CPU name=mixer vs0.src ! decodebin3 ! mixer.sink_0  vs1.src ! decodebin3 ! mixer.sink_1  mixer. ! autovideosink"),

        "no-vmaf" => Ok("souphttpsrc location=\"https://ftp.nluug.nl/pub/graphics/blender/demo/movies/ToS/tears_of_steel_1080p.mov\" ! qtdemux name=demux demux.video_0 ! queue ! decodebin3 ! videoconvertscale ! capsfilter caps=\"video/x-raw,aspect-ratio=1/1\" ! tee name=tee ! video-encoder-stats encoder=\"x264enc bitrate=1024\" name=vs0 vmaf-stats=false tee. ! video-encoder-stats encoder=\"x264enc bitrate=512\" name=vs1 video-compare-mixer split-screen=true backend=CPU name=mixer vs0.src ! h264parse ! avdec_h264 ! mixer.sink_0  vs1.src ! h264parse ! avdec_h264 ! mixer.sink_1  mixer. ! autovideosink"),

        _ => {
            println!("Unknown pipeline type: {}", pipeline_type);
            print_usage();
            Err(anyhow::anyhow!("Invalid pipeline type"))
        }
    }
}

fn main() -> Result<(), Error> {
    gst::init()?;

    gstvideostats::plugin_register_static().expect("Failed to register videostats plugin");

    // Parse command line arguments
    let args: Vec<String> = env::args().collect();
    let pipeline_type = if args.len() > 1 {
        if args[1] == "--help" || args[1] == "-h" {
            print_usage();
            return Ok(());
        }
        &args[1]
    } else {
        "default"
    };

    println!("Using pipeline type: {}", pipeline_type);

    let pipeline_string = get_pipeline_string(pipeline_type)?;
    let pipeline = gst::parse::launch(pipeline_string)?;

    // Alternative test pipelines (commented out for reference)
    // let pipeline = gst::parse::launch("videotestsrc is-live=true ! videoconvertscale ! capsfilter caps=\"video/x-raw,width=640,height=480,aspect-ratio=1/1,framerate=30/1\" ! tee name=tee ! video-encoder-stats encoder=\"x264enc bitrate=1024\" decoder=\"h264parse ! avdec_h264\" name=vs0 ! fakesink tee. ! video-encoder-stats encoder=\"x264enc bitrate=256\" name=vs1 ! fakesink video-compare-mixer split-screen=true backend=CPU name=mixer vs0.decoder_src ! mixer.sink_0  vs1.decoder_src ! mixer.sink_1  mixer. ! autovideosink")?;
    // let pipeline = gst::parse::launch("gltestsrc is-live=true pattern=13 ! gldownload ! videoconvert ! capsfilter caps=\"video/x-raw,width=640,height=480,framerate=30/1\" ! tee name=tee tee.src_0 ! video-encoder-stats encoder=\"x264enc bitrate=1024\" decoder=\"h264parse ! avdec_h264\" name=vs0 ! fakesink tee.src_1 ! video-encoder-stats encoder=\"x264enc bitrate=512\" name=vs1 ! fakesink video-compare-mixer split-screen=true backend=CPU name=mixer vs0.decoder_src ! mixer.sink_0  vs1.decoder_src ! mixer.sink_1  mixer. ! autovideosink")?;

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

// Copyright (C) 2025, Fluendo S.A.
//      Author: Diego Nieto <dnieto@fluendo.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v2.0.
// If a copy of the MPL was not distributed with this file, You can obtain one at
// <https://mozilla.org/MPL/2.0/>.
//
// SPDX-License-Identifier: MPL-2.0

use gst::prelude::*;

fn init() {
    use std::sync::Once;
    static INIT: Once = Once::new();

    INIT.call_once(|| {
        gst::init().unwrap();
        gstvideostats::plugin_register_static().unwrap();
    });
}

fn create_test_buffer(pts: gst::ClockTime) -> gst::Buffer {
    let width = 320i32;
    let height = 240i32;
    let fps_num = 30i32;
    let fps_den = 1i32;

    let info =
        gst_video::VideoInfo::builder(gst_video::VideoFormat::I420, width as u32, height as u32)
            .fps(gst::Fraction::new(fps_num, fps_den))
            .build()
            .unwrap();

    let mut buffer = gst::Buffer::with_size(info.size()).unwrap();
    {
        let buffer_mut = buffer.get_mut().unwrap();
        buffer_mut.set_pts(pts);
        buffer_mut.set_duration(gst::ClockTime::SECOND / fps_num as u64);
    }
    buffer
}

#[test]
fn test_videomixer_with_tee() {
    init();

    // Use tee element which exists and has multiple src pads
    let mut h = gst_check::Harness::with_padnames("tee", Some("sink"), None);

    // Set input caps
    let caps = gst_video::VideoCapsBuilder::new()
        .format(gst_video::VideoFormat::I420)
        .width(320)
        .height(240)
        .framerate(gst::Fraction::new(30, 1))
        .build();

    h.set_src_caps(caps);

    // Push a buffer
    let buf = create_test_buffer(0.nseconds());
    h.push(buf).unwrap();

    // Since tee doesn't have a default src pad, we need to request one
    let element = h.element().unwrap();
    let src_pad = element.request_pad_simple("src_%u");
    assert!(
        src_pad.is_some(),
        "Should be able to request src pad from tee"
    );
}

#[test]
fn test_compositor_two_pads() {
    init();

    // Test with compositor which should be available
    if gst::ElementFactory::find("compositor").is_none() {
        eprintln!("Skipping test: compositor not available");
        return;
    }

    let mut h = gst_check::Harness::with_padnames("compositor", None, Some("src"));

    // Create two sink pads using request pads
    let element = h.element().unwrap();
    let sink_0_pad = element.request_pad_simple("sink_%u");
    let sink_1_pad = element.request_pad_simple("sink_%u");

    assert!(
        sink_0_pad.is_some(),
        "Should be able to request first sink pad"
    );
    assert!(
        sink_1_pad.is_some(),
        "Should be able to request second sink pad"
    );

    // Create harnesses for the sink pads
    let mut sink_0 =
        gst_check::Harness::with_element(&element, Some(&sink_0_pad.unwrap().name()), None);
    let mut sink_1 =
        gst_check::Harness::with_element(&element, Some(&sink_1_pad.unwrap().name()), None);

    // Set input caps for both pads
    let caps = gst_video::VideoCapsBuilder::new()
        .format(gst_video::VideoFormat::Rgba)
        .width(320)
        .height(240)
        .framerate(gst::Fraction::new(30, 1))
        .build();

    sink_0.set_src_caps(caps.clone());
    sink_1.set_src_caps(caps);

    // Push buffers to both pads
    let buf_0 = create_test_buffer(0.nseconds());
    sink_0.push(buf_0).unwrap();

    let buf_1 = create_test_buffer(0.nseconds());
    sink_1.push(buf_1).unwrap();

    // Pull the output buffer
    let out = h.pull().unwrap();

    // Verify we get a buffer with some size
    assert!(out.size() > 0, "Should receive a non-empty buffer");
}

#[test]
fn test_element_creation_fallback() {
    init();

    // Test creating various video elements to see what's available
    let elements_to_test = ["compositor", "videomixer", "tee", "videoconvert"];

    for element_name in &elements_to_test {
        let element = gst::ElementFactory::make(element_name).build();
        if element.is_ok() {
            println!("Successfully created element: {}", element_name);
            let element = element.unwrap();

            // Test basic state transitions
            assert_eq!(
                element.set_state(gst::State::Ready),
                Ok(gst::StateChangeSuccess::Success)
            );
            assert_eq!(
                element.set_state(gst::State::Null),
                Ok(gst::StateChangeSuccess::Success)
            );
        } else {
            println!("Element not available: {}", element_name);
        }
    }
}

#[test]
fn test_video_encoder_stats_integration() {
    init();

    // Test that our video-encoder-stats element exists and can be created
    let element = gst::ElementFactory::make("video-encoder-stats").build();
    assert!(
        element.is_ok(),
        "Should be able to create video-encoder-stats element"
    );

    let element = element.unwrap();

    // Test state transitions
    assert_eq!(
        element.set_state(gst::State::Ready),
        Ok(gst::StateChangeSuccess::Success)
    );
    assert_eq!(element.current_state(), gst::State::Ready);

    // Clean up
    assert_eq!(
        element.set_state(gst::State::Null),
        Ok(gst::StateChangeSuccess::Success)
    );
}

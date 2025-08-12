// Copyright (C) 2025, Fluendo S.A.
//      Author: Diego Nieto <dnieto@fluendo.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v2.0.
// If a copy of the MPL was not distributed with this file, You can obtain one at
// <https://mozilla.org/MPL/2.0/>.
//
// SPDX-License-Identifier: MPL-2.0

use gst::prelude::*;
use gstvideostats::videoencoderstatsmeta::VideoEncoderStatsMeta;

fn init() {
    use std::sync::Once;
    static INIT: Once = Once::new();

    INIT.call_once(|| {
        gst::init().unwrap();
        gstvideostats::plugin_register_static().unwrap();
        // Register x264 plugin if available
        let registry = gst::Registry::get();
        let _ = registry.find_plugin("x264");
    });
}

fn create_test_buffer() -> gst::Buffer {
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
        buffer_mut.set_pts(0.nseconds());
        buffer_mut.set_duration(gst::ClockTime::SECOND / fps_num as u64);
    }
    buffer
}

#[test]
fn test_video_encoder_stats_single_buffer() {
    init();

    // Skip test if x264enc is not available
    if gst::ElementFactory::find("x264enc").is_none() {
        eprintln!("Skipping test: x264enc not available");
        return;
    }

    let mut h = gst_check::Harness::with_padnames("video-encoder-stats", Some("sink"), Some("src"));

    // Create and configure the encoder
    let encoder = gst::ElementFactory::make("x264enc")
        .property("bitrate", 256u32)
        .build()
        .unwrap();

    // Set the encoder property on the video-encoder-stats element
    h.element().unwrap().set_property("encoder", &encoder);

    // Set input caps (raw video)
    let caps = gst_video::VideoCapsBuilder::new()
        .format(gst_video::VideoFormat::I420)
        .width(320)
        .height(240)
        .framerate(gst::Fraction::new(30, 1))
        .build();
    h.set_src_caps(caps);

    // Create and push a test buffer
    let buf = create_test_buffer();
    h.push(buf).unwrap();

    // Pull the output buffer
    let out = h.pull().unwrap();

    // Verify we get a buffer with some size
    assert!(out.size() > 0, "Should receive a non-empty buffer");

    // Check that VideoEncoderStatsMeta is present
    let meta = out.meta::<VideoEncoderStatsMeta>();
    assert!(
        meta.is_some(),
        "VideoEncoderStatsMeta should be present on output buffer"
    );

    let meta = meta.unwrap();
    let stats = meta.stats();

    // Verify basic stats properties
    assert_eq!(stats.name, "x264enc", "Encoder name should match");
    assert!(
        stats.num_buffers > 0,
        "Buffer count should be greater than 0"
    );
    assert!(stats.num_bytes > 0, "Byte count should be greater than 0");
}

#[test]
fn test_video_encoder_stats_multiple_buffers() {
    init();

    // Skip test if x264enc is not available
    if gst::ElementFactory::find("x264enc").is_none() {
        eprintln!("Skipping test: x264enc not available");
        return;
    }

    let mut h = gst_check::Harness::with_padnames("video-encoder-stats", Some("sink"), Some("src"));

    // Create and configure the encoder
    let encoder = gst::ElementFactory::make("x264enc")
        .property("bitrate", 256u32)
        .build()
        .unwrap();

    h.element().unwrap().set_property("encoder", &encoder);

    // Set input caps
    let caps = gst_video::VideoCapsBuilder::new()
        .format(gst_video::VideoFormat::I420)
        .width(320)
        .height(240)
        .framerate(gst::Fraction::new(30, 1))
        .build();
    h.set_src_caps(caps);

    // Push multiple buffers
    for i in 0..5 {
        let mut buf = create_test_buffer();
        {
            let buf_mut = buf.get_mut().unwrap();
            buf_mut.set_pts((i as u64) * gst::ClockTime::SECOND / 30);
        }
        h.push(buf).unwrap();
    }

    // Pull and verify each output buffer
    for i in 0..5 {
        let out = h.pull().unwrap();
        assert!(out.size() > 0, "Should receive non-empty buffers");

        // Check that VideoEncoderStatsMeta is present
        let meta = out.meta::<VideoEncoderStatsMeta>();
        assert!(
            meta.is_some(),
            "VideoEncoderStatsMeta should be present on buffer {}",
            i
        );

        let meta = meta.unwrap();
        let stats = meta.stats();

        assert_eq!(stats.name, "x264enc");
        assert!(
            stats.num_buffers >= (i + 1) as u64,
            "Buffer count should increase with each buffer"
        );
        assert!(stats.num_bytes > 0);

        // Verify framerate is set from caps
        assert!(
            stats.framerate.is_some(),
            "Framerate should be set from caps"
        );
        if let Some(fps) = stats.framerate {
            assert_eq!(fps.numer(), 30);
            assert_eq!(fps.denom(), 1);
        }
    }
}

#[test]
fn test_video_encoder_stats_with_request_pad() {
    init();

    // Skip test if x264enc is not available
    if gst::ElementFactory::find("x264enc").is_none() {
        eprintln!("Skipping test: x264enc not available");
        return;
    }

    let element = gst::ElementFactory::make("video-encoder-stats")
        .build()
        .unwrap();

    // Request the decoder src pad before going to PAUSED
    let request_pad = element.request_pad_simple("decoder_src_%u");
    assert!(
        request_pad.is_some(),
        "Should be able to request decoder_src pad"
    );

    let mut h = gst_check::Harness::with_element(&element, Some("sink"), Some("src"));

    // Create and configure the encoder
    let encoder = gst::ElementFactory::make("x264enc")
        .property("bitrate", 256u32)
        .build()
        .unwrap();

    element.set_property("encoder", &encoder);

    // Set input caps
    let caps = gst_video::VideoCapsBuilder::new()
        .format(gst_video::VideoFormat::I420)
        .width(320)
        .height(240)
        .framerate(gst::Fraction::new(30, 1))
        .build();
    h.set_src_caps(caps);

    // Push a buffer
    let buf = create_test_buffer();
    h.push(buf).unwrap();

    // Pull the output buffer from main src pad
    let out = h.pull().unwrap();

    // Verify we get a buffer with some size
    assert!(out.size() > 0, "Should receive a non-empty buffer");

    // Check that VideoEncoderStatsMeta is present even with request pad
    let meta = out.meta::<VideoEncoderStatsMeta>();
    assert!(
        meta.is_some(),
        "VideoEncoderStatsMeta should be present with request pad"
    );

    let meta = meta.unwrap();
    let stats = meta.stats();
    assert_eq!(stats.name, "x264enc");
    assert!(stats.num_buffers > 0);
    assert!(stats.num_bytes > 0);
}

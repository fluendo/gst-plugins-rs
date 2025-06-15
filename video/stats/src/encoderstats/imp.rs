// Copyright (C) 2025, Fluendo S.A.
//      Author: Diego Nieto <dnieto@fluendo.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v2.0.
// If a copy of the MPL was not distributed with this file, You can obtain one at
// <https://mozilla.org/MPL/2.0/>.
//
// SPDX-License-Identifier: MPL-2.0

use gst::glib;
use gst::prelude::*;
use gst::subclass::prelude::*;

use crate::videoencoderstats::*;
use crate::videoencoderstatsmeta::VideoEncoderStatsMeta;

use std::sync::{LazyLock, Mutex};
use std::vec::Vec;
use std::sync::Arc;

static CAT: LazyLock<gst::DebugCategory> = LazyLock::new(|| {
    gst::DebugCategory::new(
        "video-encoder-stats",
        gst::DebugColorFlags::empty(),
        Some("GstVideoEncoderStats"),
    )
});

pub struct EncoderStats {
    srcpad: gst::GhostPad,
    sinkpad: gst::GhostPad,
    identity: gst::Element,
    stats: Arc<Mutex<VideoEncoderStats>>,
}

impl EncoderStats {
    fn add_identity_probe(
        &self,
    ) {
        let identity = self.obj().by_name("identity").expect("expected identity");
        let identity_src_pad = identity.static_pad("src").unwrap();
        let encoder = self.obj().by_name("enc").expect("expected encoder");
        let encoder_factory = encoder.factory().expect("encoder should have a factory");
        let encoder_name = encoder_factory.name();

        let stats = self.stats.clone();
        let obj_name = self.obj().name().to_string();
        identity_src_pad.add_probe(gst::PadProbeType::BUFFER, move |_pad, probe_info| {
            let Some(buffer) = probe_info.buffer_mut() else {
                return gst::PadProbeReturn::Ok;
            };

            let identity_stats = identity.property::<gst::Structure>("stats");
            let num_bytes = identity_stats.get::<u64>("num-bytes").unwrap();
            let num_buffers = identity_stats.get::<u64>("num-buffers").unwrap();

            let mut stats = stats.lock().unwrap();
            let fps_n: i32;
            if let Some(fps) = stats.framerate {
                fps_n = fps.numer();
            } else {
                return gst::PadProbeReturn::Ok;
            }

            if num_buffers % (fps_n as u64) != 0 {
                gst::log!(CAT, "Skipping probe for buffer {num_buffers} as it is not a multiple of framerate {fps_n}");
                return gst::PadProbeReturn::Ok;
            }

            // FIXME: integrates queues internally to calculate the CPU usage
            let thread_name = if obj_name.contains("0") {
                "encq0:src"
            } else {
                "encq1:src"
            };
            let (total_utime, total_stime) = get_cpu_usage(thread_name.to_string());

            stats.threads_utime = total_utime;
            stats.threads_stime = total_stime;
            stats.num_bytes = num_bytes;
            stats.num_buffers = num_buffers;
            stats.name = encoder_name.to_string();

            let buffer = buffer.make_mut();

            // Add the VideoEncoderStatsMeta to the buffer
            VideoEncoderStatsMeta::add(
                buffer,
                stats.clone(),
            );

            gst::PadProbeReturn::Ok
        });
    }

    fn add_encoder_probes(&self) {
        let encoder = self.obj().by_name("enc").expect("expected identity");
        let encoder_sink_pad = encoder.static_pad("sink").unwrap();
        let encoder_src_pad = encoder.static_pad("src").unwrap();

        let stats = self.stats.clone();
        encoder_sink_pad.add_probe(gst::PadProbeType::BUFFER, move |_, probe_info| {
            let Some(_) = probe_info.buffer() else {
                return gst::PadProbeReturn::Ok;
            };
            stats.lock().unwrap().buffer_in();
            gst::log!(CAT, "Buffer in encoder sink pad");
            gst::PadProbeReturn::Ok
        });

        let stats = self.stats.clone();
        encoder_src_pad.add_probe(gst::PadProbeType::BUFFER, move |_, probe_info| {
            let Some(_) = probe_info.buffer() else {
                return gst::PadProbeReturn::Ok;
            };
            stats.lock().unwrap().buffer_out();
            gst::log!(CAT, "Buffer out encoder src pad");
            gst::PadProbeReturn::Ok
        });
    }

    fn sink_event(&self, pad: &gst::Pad, event: gst::Event) -> bool {
        gst::log!(CAT, obj = pad, "Handling sink event {:?}", event);

        use gst::EventView::*;
        match event.view() {
            Caps(event) => {
                let caps = event.caps();
                let s = caps.structure(0).unwrap();
                let fps = s.get::<gst::Fraction>("framerate").ok();
                self.stats.lock().unwrap().framerate = fps;
                gst::info!(CAT, "Received caps {caps:?}");
            }
            _ => {
                gst::info!(CAT, "Other event");
            }
        }
        gst::Pad::event_default(pad, Some(&*self.obj()), event);
        true
    }
}

#[glib::object_subclass]
impl ObjectSubclass for EncoderStats {
    const NAME: &'static str = "GstEncoderStats";
    type Type = super::VideoEncoderStats;
    type ParentType = gst::Bin;

    fn with_class(klass: &Self::Class) -> Self {
        let templ = klass.pad_template("sink").unwrap();
        let sinkpad = gst::GhostPad::from_template(&templ);

        let templ = klass.pad_template("src").unwrap();
        let srcpad = gst::GhostPad::from_template(&templ);

        let identity = gst::ElementFactory::make("identity")
            .build()
            .expect("Failed to create identity element");
        identity.set_property("name", "identity");

        Self {
            srcpad,
            sinkpad,
            identity,
            stats: Arc::new(Mutex::new(VideoEncoderStats::default())),
        }
    }
}

impl ObjectImpl for EncoderStats {
    fn properties() -> &'static [glib::ParamSpec] {
        static PROPERTIES: LazyLock<Vec<glib::ParamSpec>> = LazyLock::new(|| {
            vec![
                glib::ParamSpecObject::builder::<gst::Element>("encoder")
                    .nick("The encoder stats")
                    .blurb("The encoder name to use")
                    .construct_only()
                    .build(),
            ]
        });

        PROPERTIES.as_ref()
    }

    fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
        match pspec.name() {
            "encoder" => {
                self.obj().by_name("enc").to_value()
            }
            _ => unimplemented!(),
        }
    }

    fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
        match pspec.name() {
            "encoder" => {
                if let Ok(Some(enc_obj)) = value.get::<Option<gst::Element>>() {
                    let factory = enc_obj
                        .factory()
                        .expect("Element should have a factory");

                    if !factory.has_type(gst::ElementFactoryType::VIDEO_ENCODER)
                    {
                        gst::error!(CAT, "The element is not a video encoder");
                        panic!("The element is not a video encoder");
                    }
                    enc_obj.set_property("name", "enc");

                    self.obj().add(&self.identity).unwrap();
                    self.srcpad
                        .set_target(Some(&self.identity.static_pad("src").unwrap()))
                        .unwrap();

                    self.obj().add(&enc_obj).expect("Failed to add encoder element");
                    self.sinkpad
                        .set_target(Some(&enc_obj.static_pad( "sink").unwrap()))
                        .expect("Failed to link sink pad to encoder element");
                    enc_obj.link(&self.obj().by_name("identity").expect("expected identity"))
                        .expect("Failed to link encoder to identity");

                    unsafe
                    {
                        self.sinkpad.set_event_full_function(|pad, parent, event| {
                            EncoderStats::catch_panic_pad_function(
                                parent,
                                || false,
                                |video_encoder_stats| video_encoder_stats.sink_event(&pad.clone().upcast::<gst::Pad>(), event),
                            );
                            Ok(gst::FlowSuccess::Ok)
                        });
                    }

                    self.add_identity_probe();
                    self.add_encoder_probes();
                }
            }
            _ => unimplemented!(),
        }
    }

    fn constructed(&self) {
        self.parent_constructed();
        let obj = self.obj();

        obj.add_pad(&self.sinkpad).unwrap();
        obj.add_pad(&self.srcpad).unwrap();
    }
}

impl GstObjectImpl for EncoderStats {}

impl ElementImpl for EncoderStats {
    fn metadata() -> Option<&'static gst::subclass::ElementMetadata> {
        static ELEMENT_METADATA: LazyLock<gst::subclass::ElementMetadata> = LazyLock::new(|| {
            gst::subclass::ElementMetadata::new(
                "EncoderStats",
                "Video/Encoder/Filter",
                "Video Encoder Stats Wrapper",
                "Diego Nieto <dnieto@fluendo.com>",
            )
        });

        Some(&*ELEMENT_METADATA)
    }

    fn pad_templates() -> &'static [gst::PadTemplate] {
        static PAD_TEMPLATES: LazyLock<Vec<gst::PadTemplate>> = LazyLock::new(|| {
            let sink_caps = gst_video::VideoCapsBuilder::new()
                    .build();
            let src_caps = gst::Caps::new_any();
            let video_src_pad_template = gst::PadTemplate::new(
                "src",
                gst::PadDirection::Src,
                gst::PadPresence::Always,
                &src_caps,
            )
            .unwrap();
            let video_sink_pad_template = gst::PadTemplate::new(
                "sink",
                gst::PadDirection::Sink,
                gst::PadPresence::Always,
                &sink_caps,
            )
            .unwrap();

            vec![video_src_pad_template, video_sink_pad_template]
        });

        PAD_TEMPLATES.as_ref()
    }
}

impl BinImpl for EncoderStats {}

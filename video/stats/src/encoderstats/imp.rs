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
    encoder: Mutex<Option<gst::Element>>,
    decoder: Mutex<Option<gst::Element>>,
    parser: Mutex<Option<gst::Element>>,
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

    fn prepare_pipeline(&self) -> Result<(), gst::ErrorMessage> {
        let encoder = {
            let encoder_guard = self.encoder.lock().unwrap();
            encoder_guard.clone().expect("Encoder must be set")
        };
        
        let decoder = {
            let decoder_guard = self.decoder.lock().unwrap();
            decoder_guard.clone()
        };

        let parser = {
            let parser_guard = self.parser.lock().unwrap();
            parser_guard.clone()
        };

        encoder.set_property("name", "enc");

        let originalbuffersave = gst::ElementFactory::make("originalbuffersave")
            .build()
            .expect("Failed to create originalbuffersave element");
        self.obj().add(&originalbuffersave).expect("Failed to add originalbuffersave element");

        self.obj().add(&self.identity).unwrap();

        let tee0 = gst::ElementFactory::make("tee")
            .name("tee0")
            .build()
            .expect("Failed to create tee0 element");
        self.obj().add(&tee0).unwrap();
        
        self.obj().add(&encoder).expect("Failed to add encoder element");
        originalbuffersave.link(&encoder).expect("Failed to link originalbuffersave to encoder");
        encoder.link(&self.identity).expect("Failed to link encoder to identity");
        self.identity.link(&tee0).expect("Failed to link identity to tee0");
        
        let tee0_src_0 = tee0.request_pad_simple("src_%u").expect("tee0 src pad");
        let queue0 = gst::ElementFactory::make("queue")
        .name("encq0")
        .build()
        .expect("Failed to create queue encq0");
        self.obj().add(&queue0).expect("Failed to add queue encq0");
        let queue0_sink_pad = queue0.static_pad("sink").unwrap();
        let queue0_src_pad = queue0.static_pad("src").unwrap();
        tee0_src_0.link(&queue0_sink_pad).expect("tee0.src_0 -> encq0.sink");
        self.srcpad.set_target(Some(&queue0_src_pad)).unwrap();

        self.sinkpad
            .set_target(Some(&originalbuffersave.static_pad("sink").unwrap()))
            .expect("Failed to link sink pad to originalbuffersave element");

        let tee0_src_1 = tee0.request_pad_simple("src_%u").expect("tee0 src_1");
        let queue1 = gst::ElementFactory::make("queue")
            .name("encq1")
            .build()
            .expect("Failed to create queue encq1");
        
        // Use custom decoder and parser if provided, otherwise use decodebin3
        let final_decoder = if let (Some(custom_decoder), Some(custom_parser)) = (decoder.clone(), parser.clone()) {
            custom_decoder.set_property("name", "dec");
            custom_parser.set_property("name", "parser");
            self.obj().add(&custom_parser).expect("Failed to add custom parser element");
            self.obj().add(&custom_decoder).expect("Failed to add custom decoder element");
            
            // Link parser -> decoder
            custom_parser.link(&custom_decoder).expect("Failed to link parser to decoder");
            custom_parser
        } else {
            let decodebin3 = gst::ElementFactory::make("decodebin3")
                .name("dec")
                .build()
                .expect("Failed to create decodebin3");
            self.obj().add(&decodebin3).expect("Failed to add decodebin3");
            decodebin3
        };

        // Add videoconvert after decoder and before capsfilter
        let videoconvert = gst::ElementFactory::make("videoconvert")
            .build()
            .expect("Failed to create videoconvert");
        let capsfilter = gst::ElementFactory::make("capsfilter")
            .build()
            .expect("Failed to create capsfilter");
        let caps = gst::Caps::builder("video/x-raw")
            .field("format", &"I420")
            .build();
        capsfilter.set_property("caps", &caps);
        let tee1 = gst::ElementFactory::make("tee")
            .name("tee1")
            .build()
            .expect("Failed to create tee1 element");
        let originalbufferstore = gst::ElementFactory::make("originalbufferrestore")
            .build()
            .expect("Failed to create originalbufferrestore");
        // Add queue before originalbufferrestore -> vmaf
        let queue_vmaf_0 = gst::ElementFactory::make("queue")
            .name("queue_vmaf_0")
            .build()
            .expect("Failed to create queue_vmaf_0");
        // Add queue before vmaf sink_1
        let queue_vmaf_1 = gst::ElementFactory::make("queue")
            .name("queue_vmaf_1")
            .build()
            .expect("Failed to create queue_vmaf_1");
        let vmaf = gst::ElementFactory::make("vmaf")
            .name("vmaf0")
            .build()
            .expect("Failed to create vmaf");
        vmaf.set_property("signal-scores", true);
        {
            let stats = self.stats.clone();
            vmaf.connect_closure(
                "score",
                false,
                glib::closure!(
                    move |_vmaf: &gst::Element, score: f64| {
                        let mut stats = stats.lock().unwrap();
                        stats.vmaf_score = score;
                }
                ),
            );
        }
        let fakesink = gst::ElementFactory::make("fakesink")
            .build()
            .expect("Failed to create fakesink");

        self.obj().add_many([
            &queue1, &videoconvert, &capsfilter, &tee1,
            &originalbufferstore, &queue_vmaf_0, &vmaf, &queue_vmaf_1, &fakesink,
        ].as_ref()).expect("Failed to add vmaf branch elements");

        tee0_src_1.link(&queue1.static_pad("sink").unwrap()).expect("tee0.src_1 -> queue1");
        queue1.static_pad("src").unwrap().link(&final_decoder.static_pad("sink").unwrap()).expect("queue1.src -> decoder.sink");

        let tee1_clone = tee1.clone();
        let originalbufferstore_clone = originalbufferstore.clone();
        let queue_vmaf_0_clone = queue_vmaf_0.clone();
        let vmaf_clone = vmaf.clone();
        let queue_vmaf_1_clone = queue_vmaf_1.clone();
        let fakesink_clone = fakesink.clone();
        let videoconvert_clone = videoconvert.clone();
        let capsfilter_clone = capsfilter.clone();

        // Handle linking based on whether we're using manual parser/decoder or decodebin3
        if let (Some(_), Some(_)) = (decoder, parser) {
            // Manual parser/decoder case: link decoder directly to videoconvert
            let actual_decoder = self.obj().by_name("dec").expect("expected decoder");
            let decoder_src_pad = actual_decoder.static_pad("src").expect("decoder should have src pad");
            let videoconvert_sink_pad = videoconvert.static_pad("sink").expect("videoconvert should have sink pad");
            decoder_src_pad.link(&videoconvert_sink_pad).expect("decoder.src -> videoconvert.sink");
            videoconvert.link(&capsfilter).expect("videoconvert -> capsfilter");
            capsfilter.link(&tee1).expect("capsfilter -> tee1");
            
            let tee1_src_0 = tee1.request_pad_simple("src_%u").expect("tee1 src_0");
            // Link: tee1.src_0 -> originalbufferstore -> queue_vmaf_0 -> vmaf -> fakesink
            tee1_src_0.link(&originalbufferstore.static_pad("sink").unwrap()).expect("tee1.src_0 -> originalbufferstore");
            originalbufferstore.link(&queue_vmaf_0).expect("originalbufferrestore -> queue_vmaf_0");
            queue_vmaf_0.link(&vmaf).expect("queue_vmaf_0 -> vmaf");
            vmaf.link(&fakesink).expect("vmaf -> fakesink");

            let tee1_src_1 = tee1.request_pad_simple("src_%u").expect("tee1 src_1");
            let vmaf_sink_1 = vmaf.request_pad_simple("sink_1").expect("vmaf sink_1");
            // Link: tee1.src_1 -> queue_vmaf_1 -> vmaf.sink_1
            tee1_src_1.link(&queue_vmaf_1.static_pad("sink").unwrap()).expect("tee1.src_1 -> queue_vmaf_1");
            queue_vmaf_1.static_pad("src").unwrap().link(&vmaf_sink_1).expect("queue_vmaf_1.src -> vmaf.sink_1");
        } else {
            // decodebin3 case: use connect_pad_added for dynamic linking
            final_decoder.connect_pad_added(move |_dbin, src_pad| {
                // Link decodebin3 src_pad -> videoconvert -> capsfilter -> tee1
                let videoconvert_sink = videoconvert_clone.static_pad("sink").unwrap();
                if src_pad.link(&videoconvert_sink).is_ok() {
                    let videoconvert_src = videoconvert_clone.static_pad("src").unwrap();
                    let capsfilter_sink = capsfilter_clone.static_pad("sink").unwrap();
                    if videoconvert_src.link(&capsfilter_sink).is_ok() {
                        let capsfilter_src = capsfilter_clone.static_pad("src").unwrap();
                        let tee1_sink = tee1_clone.static_pad("sink").unwrap();
                        if capsfilter_src.link(&tee1_sink).is_ok() {
                            let tee1_src_0 = tee1_clone.request_pad_simple("src_%u").expect("tee1 src_0");
                            // Link: tee1.src_0 -> originalbufferstore -> queue_vmaf_0 -> vmaf -> fakesink
                            tee1_src_0.link(&originalbufferstore_clone.static_pad("sink").unwrap()).expect("tee1.src_0 -> originalbufferstore");
                            originalbufferstore_clone.link(&queue_vmaf_0_clone).expect("originalbufferrestore -> queue_vmaf_0");
                            queue_vmaf_0_clone.link(&vmaf_clone).expect("queue_vmaf_0 -> vmaf");
                            vmaf_clone.link(&fakesink_clone).expect("vmaf -> fakesink");

                            let tee1_src_1 = tee1_clone.request_pad_simple("src_%u").expect("tee1 src_1");
                            let vmaf_sink_1 = vmaf_clone.request_pad_simple("sink_1").expect("vmaf sink_1");
                            // Link: tee1.src_1 -> queue_vmaf_1 -> vmaf.sink_1
                            tee1_src_1.link(&queue_vmaf_1_clone.static_pad("sink").unwrap()).expect("tee1.src_1 -> queue_vmaf_1");
                            queue_vmaf_1_clone.static_pad("src").unwrap().link(&vmaf_sink_1).expect("queue_vmaf_1.src -> vmaf.sink_1");
                        }
                    }
                }
            });
        }

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

        Ok(())
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
            encoder: Mutex::new(None),
            decoder: Mutex::new(None),
            parser: Mutex::new(None),
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
                glib::ParamSpecObject::builder::<gst::Element>("decoder")
                    .nick("The decoder element")
                    .blurb("The decoder element to use for VMAF calculation (default: decodebin3)")
                    .build(),
                glib::ParamSpecObject::builder::<gst::Element>("parser")
                    .nick("The parser element")
                    .blurb("The parser element to use before decoder (must be set together with decoder)")
                    .build(),
            ]
        });

        PROPERTIES.as_ref()
    }

    fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
        match pspec.name() {
            "encoder" => {
                let encoder_guard = self.encoder.lock().unwrap();
                encoder_guard.clone().to_value()
            }
            "decoder" => {
                let decoder_guard = self.decoder.lock().unwrap();
                decoder_guard.clone().to_value()
            }
            "parser" => {
                let parser_guard = self.parser.lock().unwrap();
                parser_guard.clone().to_value()
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
                    
                    let mut encoder_guard = self.encoder.lock().unwrap();
                    *encoder_guard = Some(enc_obj);
                }
            }
            "decoder" => {
                if let Ok(Some(dec_obj)) = value.get::<Option<gst::Element>>() {
                    let factory = dec_obj
                        .factory()
                        .expect("Element should have a factory");

                    if !factory.has_type(gst::ElementFactoryType::DECODER) {
                        gst::error!(CAT, "The element is not a decoder");
                        panic!("The element is not a decoder");
                    }
                    
                    let mut decoder_guard = self.decoder.lock().unwrap();
                    *decoder_guard = Some(dec_obj);
                }
            }
            "parser" => {
                if let Ok(Some(parser_obj)) = value.get::<Option<gst::Element>>() {
                    let factory = parser_obj
                        .factory()
                        .expect("Element should have a factory");

                    if !factory.has_type(gst::ElementFactoryType::PARSER) {
                        gst::error!(CAT, "The element is not a parser");
                        panic!("The element is not a parser");
                    }
                    
                    let mut parser_guard = self.parser.lock().unwrap();
                    *parser_guard = Some(parser_obj);
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

    fn change_state(
        &self,
        transition: gst::StateChange,
    ) -> Result<gst::StateChangeSuccess, gst::StateChangeError> {
        match transition {
            gst::StateChange::ReadyToPaused => {
                // Validate parser and decoder properties
                let decoder_guard = self.decoder.lock().unwrap();
                let parser_guard = self.parser.lock().unwrap();
                let has_decoder = decoder_guard.is_some();
                let has_parser = parser_guard.is_some();
                drop(decoder_guard);
                drop(parser_guard);

                if has_decoder != has_parser {
                    let error_msg = if has_decoder {
                        "Parser must be set when decoder is provided"
                    } else {
                        "Decoder must be set when parser is provided"
                    };
                    gst::error!(CAT, imp = self, "{}", error_msg);
                    return Err(gst::StateChangeError);
                }

                if let Err(err) = self.prepare_pipeline() {
                    gst::error!(CAT, imp = self, "Failed to prepare pipeline: {}", err);
                    return Err(gst::StateChangeError);
                }
                gst::info!(CAT, imp = self, "Pipeline prepared");
            }
            _ => {}
        }

        self.parent_change_state(transition)
    }
}

impl BinImpl for EncoderStats {}

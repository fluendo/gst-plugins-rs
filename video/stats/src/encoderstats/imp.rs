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
    request_pad: Mutex<Option<gst::GhostPad>>,
    vmaf_stats: Mutex<bool>,
    vmaf_available: bool,
    silent: Mutex<bool>,
    last_message: Arc<Mutex<Option<String>>>,
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
        identity_src_pad.add_probe(gst::PadProbeType::BUFFER, move |_pad, probe_info| {
            let Some(_) = probe_info.buffer() else {
                return gst::PadProbeReturn::Ok;
            };

            let identity_stats = identity.property::<gst::Structure>("stats");
            let num_bytes = identity_stats.get::<u64>("num-bytes").unwrap();
            let num_buffers = identity_stats.get::<u64>("num-buffers").unwrap();

            let mut stats = stats.lock().unwrap();

            stats.num_bytes = num_bytes;
            stats.num_buffers = num_buffers;
            stats.name = encoder_name.to_string();

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
            stats.lock().unwrap().pre_encode_time = *gst::ClockTime::from_nseconds(
                gst::SystemClock::obtain().upcast::<gst::Clock>().time().unwrap().nseconds()
            );
            gst::PadProbeReturn::Ok
        });

        let stats = self.stats.clone();
        encoder_src_pad.add_probe(gst::PadProbeType::BUFFER, move |_, probe_info| {
            let Some(_) = probe_info.buffer() else {
                return gst::PadProbeReturn::Ok;
            };
            stats.lock().unwrap().buffer_out();
            gst::log!(CAT, "Buffer out encoder src pad");
            stats.lock().unwrap().post_encode_time = *gst::ClockTime::from_nseconds(
                gst::SystemClock::obtain().upcast::<gst::Clock>().time().unwrap().nseconds()
            );
            gst::PadProbeReturn::Ok
        });
    }

    fn sink_event(&self, pad: &gst::Pad, event: gst::Event) -> bool {
        gst::log!(CAT, obj = pad, "Handling sink event {:?}", event);

        use gst::EventView::*;
        match event.view() {
            Caps(event) => {
                let caps = event.caps();
                gst::info!(CAT, "Received caps {caps:?}");
                let s = caps.structure(0).unwrap();
                let fps = s.get::<gst::Fraction>("framerate").ok();
                self.stats.lock().unwrap().framerate = fps;
                
                // Only set vmaf subsample if vmaf is available and enabled
                let vmaf_enabled = {
                    let vmaf_stats_guard = self.vmaf_stats.lock().unwrap();
                    *vmaf_stats_guard && self.vmaf_available
                };
                
                if vmaf_enabled {
                    if let Some(vmaf) = self.obj().by_name("vmaf0") {
                        vmaf.set_property("subsample", fps.unwrap().numer() as u32);
                    }
                }
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

        let vmaf_enabled = {
            let vmaf_stats_guard = self.vmaf_stats.lock().unwrap();
            *vmaf_stats_guard && self.vmaf_available
        };

        let has_request_pad = {
            let request_pad_guard = self.request_pad.lock().unwrap();
            request_pad_guard.is_some()
        };

        encoder.set_property("name", "enc");

        // Add internal queue at the beginning
        let obj_name = self.obj().name().to_string();
        let queue_name = if obj_name.contains("0") {
            "encq0"
        } else {
            "encq1"
        };

        let input_queue = gst::ElementFactory::make("queue")
            .name(queue_name)
            .build()
            .expect("Failed to create input queue");
        self.obj().add(&input_queue).expect("Failed to add input queue");

        // Add probe to input queue src pad to log buffer flow
        let input_queue_src_pad = input_queue.static_pad("src").unwrap();
        let queue_name_clone = queue_name.to_string();
        let stats_clone = self.stats.clone();
        let element_weak = self.obj().downgrade();
        let silent_arc = Arc::new(self.silent.lock().unwrap().clone());
        let last_message_arc = self.last_message.clone();
        input_queue_src_pad.add_probe(gst::PadProbeType::BUFFER, move |_pad, probe_info| {
            let Some(buffer) = probe_info.buffer_mut() else {
                return gst::PadProbeReturn::Ok;
            };
            gst::info!(CAT, "Buffer received in {} src pad, PTS: {:?}, DTS: {:?}, size: {}", 
                queue_name_clone, buffer.pts(), buffer.dts(), buffer.size());

            let mut stats = stats_clone.lock().unwrap();

            // Only update stats at framerate intervals
            let fps_n: i32;
            if let Some(fps) = stats.framerate {
                fps_n = fps.numer();
            } else {
                return gst::PadProbeReturn::Ok;
            }

            let num_buffers = stats.num_buffers;

            if num_buffers % (fps_n as u64) != 0 {
                gst::log!(CAT, "Skipping probe for buffer {num_buffers} as it is not a multiple of framerate {fps_n}");
                return gst::PadProbeReturn::Ok;
            }

            let queue_name = if obj_name.contains("0") { "encq0:src" } else { "encq1:src" };
            let thread_patterns = match stats.name.as_str() {
                "flulcevch264enc" => vec![queue_name, "lcevc", "pool."],
                "lcevch264enc" => vec![queue_name, "pool."],
                _ => vec![queue_name],
            };

            let (total_utime, total_stime) = thread_patterns.iter()
                .map(|pattern| {
                    let (utime, stime) = get_cpu_usage(pattern.to_string());
                    gst::log!(CAT, "Thread pattern '{}' - utime: {}, stime: {}", pattern, utime, stime);
                    (utime, stime)
                })
                .fold((0u64, 0u64), |(acc_utime, acc_stime), (utime, stime)| {
                    (acc_utime + utime, acc_stime + stime)
                });

            stats.threads_utime = total_utime;
            stats.threads_stime = total_stime;

            stats.input_time = *gst::ClockTime::from_nseconds(
                gst::SystemClock::obtain().upcast::<gst::Clock>().time().unwrap().nseconds()
            );

            if let Some(element) = element_weak.upgrade() {
                let stats_message = format!("{}", stats.clone());
                
                let structure = gst::Structure::builder("encoder-stats")
                    .field("message", &stats_message)
                    .build();
                
                let message = gst::message::Application::new(structure);
                let _ = element.post_message(message);

                let silent = *silent_arc;
                
                if !silent {
                    {
                        let mut last_message_guard = last_message_arc.lock().unwrap();
                        *last_message_guard = Some(stats_message);
                    }
                    element.notify("last-message");
                }
            }

            let buffer = buffer.make_mut();

            // Add the VideoEncoderStatsMeta to the buffer
            VideoEncoderStatsMeta::add(
                buffer,
                stats.clone(),
            );

            gst::PadProbeReturn::Ok
        });

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

        // Link: input_queue -> originalbuffersave -> encoder -> identity -> tee0
        input_queue.link(&originalbuffersave).expect("Failed to link input queue to originalbuffersave");
        originalbuffersave.link(&encoder).expect("Failed to link originalbuffersave to encoder");
        encoder.link(&self.identity).expect("Failed to link encoder to identity");
        self.identity.link(&tee0).expect("Failed to link identity to tee0");
        
        let tee0_src_0 = tee0.request_pad_simple("src_%u").expect("tee0 src pad");
        let queue0 = gst::ElementFactory::make("queue")
        .name("encintq0")
        .build()
        .expect("Failed to create queue encintq0");
        self.obj().add(&queue0).expect("Failed to add queue encintq0");
        let queue0_sink_pad = queue0.static_pad("sink").unwrap();
        let queue0_src_pad = queue0.static_pad("src").unwrap();
        tee0_src_0.link(&queue0_sink_pad).expect("tee0.src_0 -> encintq0.sink");
        self.srcpad.set_target(Some(&queue0_src_pad)).unwrap();

        // Connect sink ghostpad to input queue
        self.sinkpad
            .set_target(Some(&input_queue.static_pad("sink").unwrap()))
            .expect("Failed to link sink pad to input queue");

        // Only create decoder branch if VMAF is enabled
        if vmaf_enabled {
            let decoder = {
                let decoder_guard = self.decoder.lock().unwrap();
                decoder_guard.clone()
            };

            let tee0_src_1 = tee0.request_pad_simple("src_%u").expect("tee0 src_1");
            let queue1 = gst::ElementFactory::make("queue")
                .name("encintq1")
                .build()
                .expect("Failed to create queue encintq1");
            
            // Use custom decoder if provided, otherwise use decodebin3
            let final_decoder = if let Some(custom_decoder) = decoder.clone() {
                custom_decoder.set_property("name", "dec");
                self.obj().add(&custom_decoder).expect("Failed to add custom decoder element");
                custom_decoder
            } else {
                let decodebin3 = gst::ElementFactory::make("decodebin3")
                    .name("dec")
                    .build()
                    .expect("Failed to create decodebin3");
                self.obj().add(&decodebin3).expect("Failed to add decodebin3");
                decodebin3
            };

            self.obj().add(&queue1).expect("Failed to add queue1");
            tee0_src_1.link(&queue1.static_pad("sink").unwrap()).expect("tee0.src_1 -> queue1");
            queue1.static_pad("src").unwrap().link(&final_decoder.static_pad("sink").unwrap()).expect("queue1.src -> decoder.sink");

            // Conditionally add tee after decoder if request pad exists
            if has_request_pad {
                let decoder_tee = gst::ElementFactory::make("tee")
                    .name("decoder_tee")
                    .build()
                    .expect("Failed to create decoder_tee");
                self.obj().add(&decoder_tee).expect("Failed to add decoder_tee");

                // Set up decoder -> decoder_tee connection
                self.setup_decoder_to_tee_connection(final_decoder.clone(), decoder_tee.clone(), decoder.is_some());

                // Connect decoder_tee src_0 to VMAF pipeline
                let decoder_tee_src_0 = decoder_tee.request_pad_simple("src_%u").expect("decoder_tee src_0");
                self.setup_vmaf_pipeline(decoder_tee_src_0);

                // Connect decoder_tee src_1 to request pad
                let decoder_tee_src_1 = decoder_tee.request_pad_simple("src_%u").expect("decoder_tee src_1");
                let request_pad_guard = self.request_pad.lock().unwrap();
                if let Some(ref request_pad) = *request_pad_guard {
                    request_pad.set_target(Some(&decoder_tee_src_1)).unwrap();
                }
            } else {
                // No request pad - direct connection to VMAF pipeline
                self.setup_decoder_to_vmaf_direct(final_decoder.clone(), decoder.is_some());
            }
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

    fn setup_decoder_to_tee_connection(&self, final_decoder: gst::Element, decoder_tee: gst::Element, is_manual_decoder: bool) {
        if is_manual_decoder {
            // Manual decoder case: direct link
            final_decoder.link(&decoder_tee).expect("decoder -> decoder_tee");
        } else {
            // decodebin3 case: use connect_pad_added
            let decoder_tee_clone = decoder_tee.clone();
            final_decoder.connect_pad_added(move |_dbin, src_pad| {
                let decoder_tee_sink = decoder_tee_clone.static_pad("sink").unwrap();
                src_pad.link(&decoder_tee_sink).expect("decodebin3.src -> decoder_tee.sink");
            });
        }
    }

    fn create_vmaf_pipeline_elements(&self) -> (gst::Element, gst::Element, gst::Element, gst::Element, gst::Element, gst::Element, gst::Element, gst::Element) {
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
        let queue_vmaf_0 = gst::ElementFactory::make("queue")
            .name("queue_vmaf_0")
            .build()
            .expect("Failed to create queue_vmaf_0");
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
                        stats.vmaf_score = Some(score);
                        println!("VMAF score: {:.3}", score);
                }
                ),
            );
        }
        let fakesink = gst::ElementFactory::make("fakesink")
            .build()
            .expect("Failed to create fakesink");

        (videoconvert, capsfilter, tee1, originalbufferstore, queue_vmaf_0, queue_vmaf_1, vmaf, fakesink)
    }

    fn setup_vmaf_pipeline(&self, input_pad: gst::Pad) {
        let (videoconvert, capsfilter, tee1, originalbufferstore, queue_vmaf_0, queue_vmaf_1, vmaf, fakesink) = 
            self.create_vmaf_pipeline_elements();

        self.obj().add_many([
            &videoconvert, &capsfilter, &tee1,
            &originalbufferstore, &queue_vmaf_0, &vmaf, &queue_vmaf_1, &fakesink,
        ].as_ref()).expect("Failed to add vmaf branch elements");

        // Link input_pad -> videoconvert -> capsfilter -> tee1
        let videoconvert_sink = videoconvert.static_pad("sink").unwrap();
        input_pad.link(&videoconvert_sink).expect("input -> videoconvert");
        videoconvert.link(&capsfilter).expect("videoconvert -> capsfilter");
        capsfilter.link(&tee1).expect("capsfilter -> tee1");
        
        let tee1_src_0 = tee1.request_pad_simple("src_%u").expect("tee1 src_0");
        tee1_src_0.link(&originalbufferstore.static_pad("sink").unwrap()).expect("tee1.src_0 -> originalbufferstore");
        originalbufferstore.link(&queue_vmaf_0).expect("originalbufferrestore -> queue_vmaf_0");
        queue_vmaf_0.link(&vmaf).expect("queue_vmaf_0 -> vmaf");
        vmaf.link(&fakesink).expect("vmaf -> fakesink");

        let tee1_src_1 = tee1.request_pad_simple("src_%u").expect("tee1 src_1");
        let vmaf_sink_1 = vmaf.request_pad_simple("sink_1").expect("vmaf sink_1");
        tee1_src_1.link(&queue_vmaf_1.static_pad("sink").unwrap()).expect("tee1.src_1 -> queue_vmaf_1");
        queue_vmaf_1.static_pad("src").unwrap().link(&vmaf_sink_1).expect("queue_vmaf_1.src -> vmaf.sink_1");
    }

    fn setup_decoder_to_vmaf_direct(&self, final_decoder: gst::Element, is_manual_decoder: bool) {
        let (videoconvert, capsfilter, tee1, originalbufferstore, queue_vmaf_0, queue_vmaf_1, vmaf, fakesink) = 
            self.create_vmaf_pipeline_elements();

        self.obj().add_many([
            &videoconvert, &capsfilter, &tee1,
            &originalbufferstore, &queue_vmaf_0, &vmaf, &queue_vmaf_1, &fakesink,
        ].as_ref()).expect("Failed to add vmaf branch elements");

        if is_manual_decoder {
            // Manual decoder case: link decoder directly to videoconvert
            final_decoder.link(&videoconvert).expect("decoder -> videoconvert");
            videoconvert.link(&capsfilter).expect("videoconvert -> capsfilter");
            capsfilter.link(&tee1).expect("capsfilter -> tee1");
            
            let tee1_src_0 = tee1.request_pad_simple("src_%u").expect("tee1 src_0");
            tee1_src_0.link(&originalbufferstore.static_pad("sink").unwrap()).expect("tee1.src_0 -> originalbufferstore");
            originalbufferstore.link(&queue_vmaf_0).expect("originalbufferrestore -> queue_vmaf_0");
            queue_vmaf_0.link(&vmaf).expect("queue_vmaf_0 -> vmaf");
            vmaf.link(&fakesink).expect("vmaf -> fakesink");

            let tee1_src_1 = tee1.request_pad_simple("src_%u").expect("tee1 src_1");
            let vmaf_sink_1 = vmaf.request_pad_simple("sink_1").expect("vmaf sink_1");
            tee1_src_1.link(&queue_vmaf_1.static_pad("sink").unwrap()).expect("tee1.src_1 -> queue_vmaf_1");
            queue_vmaf_1.static_pad("src").unwrap().link(&vmaf_sink_1).expect("queue_vmaf_1.src -> vmaf.sink_1");
        } else {
            // decodebin3 case: use connect_pad_added for dynamic linking
            let tee1_clone = tee1.clone();
            let originalbufferstore_clone = originalbufferstore.clone();
            let queue_vmaf_0_clone = queue_vmaf_0.clone();
            let vmaf_clone = vmaf.clone();
            let queue_vmaf_1_clone = queue_vmaf_1.clone();
            let fakesink_clone = fakesink.clone();
            let videoconvert_clone = videoconvert.clone();
            let capsfilter_clone = capsfilter.clone();

            final_decoder.connect_pad_added(move |_dbin, src_pad| {
                let videoconvert_sink = videoconvert_clone.static_pad("sink").unwrap();
                if src_pad.link(&videoconvert_sink).is_ok() {
                    let videoconvert_src = videoconvert_clone.static_pad("src").unwrap();
                    let capsfilter_sink = capsfilter_clone.static_pad("sink").unwrap();
                    if videoconvert_src.link(&capsfilter_sink).is_ok() {
                        let capsfilter_src = capsfilter_clone.static_pad("src").unwrap();
                        let tee1_sink = tee1_clone.static_pad("sink").unwrap();
                        if capsfilter_src.link(&tee1_sink).is_ok() {
                            let tee1_src_0 = tee1_clone.request_pad_simple("src_%u").expect("tee1 src_0");
                            tee1_src_0.link(&originalbufferstore_clone.static_pad("sink").unwrap()).expect("tee1.src_0 -> originalbufferstore");
                            originalbufferstore_clone.link(&queue_vmaf_0_clone).expect("originalbufferrestore -> queue_vmaf_0");
                            queue_vmaf_0_clone.link(&vmaf_clone).expect("queue_vmaf_0 -> vmaf");
                            vmaf_clone.link(&fakesink_clone).expect("vmaf -> fakesink");

                            let tee1_src_1 = tee1_clone.request_pad_simple("src_%u").expect("tee1 src_1");
                            let vmaf_sink_1 = vmaf_clone.request_pad_simple("sink_1").expect("vmaf sink_1");
                            tee1_src_1.link(&queue_vmaf_1_clone.static_pad("sink").unwrap()).expect("tee1.src_1 -> queue_vmaf_1");
                            queue_vmaf_1_clone.static_pad("src").unwrap().link(&vmaf_sink_1).expect("queue_vmaf_1.src -> vmaf.sink_1");
                        }
                    }
                }
            });
        }
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

        // Check if vmaf element is available
        let vmaf_available = if gst::ElementFactory::find("vmaf").is_none() {
            gst::warning!(CAT, "VMAF element not found, VMAF stats will be disabled");
            false
        } else {
            true
        };

        Self {
            srcpad,
            sinkpad,
            identity,
            stats: Arc::new(Mutex::new(VideoEncoderStats::default())),
            encoder: Mutex::new(None),
            decoder: Mutex::new(None),
            request_pad: Mutex::new(None),
            vmaf_stats: Mutex::new(true), // Default enabled
            vmaf_available,
            silent: Mutex::new(false), // Default: not silent
            last_message: Arc::new(Mutex::new(None)),
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
                glib::ParamSpecBoolean::builder("vmaf-stats")
                    .nick("Enable VMAF stats")
                    .blurb("Enable VMAF statistics calculation (requires vmaf element)")
                    .default_value(true)
                    .build(),
                glib::ParamSpecBoolean::builder("silent")
                    .nick("Silent")
                    .blurb("Enable silent mode (disable stdout output of stats)")
                    .default_value(false)
                    .build(),
                glib::ParamSpecString::builder("last-message")
                    .nick("Last Message")
                    .blurb("The message describing current encoder statistics")
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
            "vmaf-stats" => {
                let vmaf_stats_guard = self.vmaf_stats.lock().unwrap();
                (*vmaf_stats_guard && self.vmaf_available).to_value()
            }
            "silent" => {
                let silent_guard = self.silent.lock().unwrap();
                (*silent_guard).to_value()
            }
            "last-message" => {
                let last_message_guard = self.last_message.lock().unwrap();
                last_message_guard.clone().to_value()
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
                    let mut decoder_guard = self.decoder.lock().unwrap();
                    *decoder_guard = Some(dec_obj);
                }
            }
            "vmaf-stats" => {
                if let Ok(vmaf_stats) = value.get::<bool>() {
                    if vmaf_stats && !self.vmaf_available {
                        gst::warning!(CAT, imp = self, "Cannot enable VMAF stats: vmaf element not available");
                        return;
                    }
                    let mut vmaf_stats_guard = self.vmaf_stats.lock().unwrap();
                    *vmaf_stats_guard = vmaf_stats;
                }
            }
            "silent" => {
                if let Ok(silent) = value.get::<bool>() {
                    let mut silent_guard = self.silent.lock().unwrap();
                    *silent_guard = silent;
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
            let request_src_pad_template = gst::PadTemplate::new(
                "decoder_src",
                gst::PadDirection::Src,
                gst::PadPresence::Request,
                &src_caps,
            )
            .unwrap();

            vec![video_src_pad_template, video_sink_pad_template, request_src_pad_template]
        });

        PAD_TEMPLATES.as_ref()
    }

    fn request_new_pad(
        &self,
        templ: &gst::PadTemplate,
        name: Option<&str>,
        _caps: Option<&gst::Caps>,
    ) -> Option<gst::Pad> {
        // Only allow request pads before ReadyToPaused transition
        if self.obj().current_state() >= gst::State::Paused {
            gst::warning!(CAT, imp = self, "Cannot request pad after ReadyToPaused transition");
            return None;
        }

        // Only allow request pads if VMAF is enabled (since that's when we have decoder)
        let vmaf_enabled = {
            let vmaf_stats_guard = self.vmaf_stats.lock().unwrap();
            *vmaf_stats_guard && self.vmaf_available
        };

        if !vmaf_enabled {
            gst::warning!(CAT, imp = self, "Cannot request decoder pad when VMAF stats are disabled");
            return None;
        }

        if templ.name() == "decoder_src" {
            let mut request_pad_guard = self.request_pad.lock().unwrap();
            if request_pad_guard.is_some() {
                gst::warning!(CAT, imp = self, "Request pad already exists");
                return None;
            }

            let pad_name = if let Some(name) = name {
                name.to_string()
            } else {
                "decoder_src".to_string()
            };

            let request_pad = gst::GhostPad::from_template(templ);
            request_pad.set_property("name", &pad_name);
            self.obj().add_pad(&request_pad).unwrap();
            *request_pad_guard = Some(request_pad.clone());
            
            gst::info!(CAT, imp = self, "Created request pad: {}", pad_name);
            Some(request_pad.upcast())
        } else {
            None
        }
    }

    fn change_state(
        &self,
        transition: gst::StateChange,
    ) -> Result<gst::StateChangeSuccess, gst::StateChangeError> {
        match transition {
            gst::StateChange::ReadyToPaused => {
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

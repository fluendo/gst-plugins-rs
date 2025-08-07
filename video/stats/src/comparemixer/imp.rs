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

use crate::videoencoderstatsmeta::VideoEncoderStatsMeta;

use std::sync::{LazyLock, Mutex};
use std::vec::Vec;

static CAT: LazyLock<gst::DebugCategory> = LazyLock::new(|| {
    gst::DebugCategory::new(
        "video-compare-mixer",
        gst::DebugColorFlags::empty(),
        Some("GstVideoCompareMixer"),
    )
});

#[derive(Default, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Clone, Copy, glib::Enum)]
#[enum_type(name = "GstVideoCompareMixerBackend")]
#[repr(u32)]
#[non_exhaustive]
pub enum Backend {
    #[enum_value(name = "OpenGL", nick = "OpenGL")]
    GL,
    #[enum_value(name = "VAAPI", nick = "VAAPI")]
    #[cfg(target_os = "linux")]
    VAAPI,
    #[default]
    #[enum_value(name = "CPU", nick = "CPU")]
    CPU,
    #[enum_value(name = "D3D12", nick = "D3D12")]
    #[cfg(target_os = "windows")]
    D3D12,
}

struct Settings {
    backend: Backend,
    split_screen: bool,
}

pub struct VideoCompareMixer {
    srcpad: gst::GhostPad,
    sinkpad0: gst::GhostPad,
    sinkpad1: gst::GhostPad,
    queue0: gst::Element,
    queue1: gst::Element,
    overlay0: gst::Element,
    overlay1: gst::Element,
    settings: Mutex<Settings>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            backend: Backend::default(),
            split_screen: false,
        }
    }
}

impl VideoCompareMixer {
    fn get_pipeline_compositor(&self, backend: Backend) -> &str {
        match backend {
            Backend::GL => "glvideomixer",
            #[cfg(target_os = "linux")]
            Backend::VAAPI => "vacompositor",
            Backend::CPU => "compositor",
            #[cfg(target_os = "windows")]
            Backend::D3D12 => "d3d12compositor",
        }
    }

    fn prepare_pipeline(&self) -> Result<(), gst::ErrorMessage> {
        let settings = self.settings.lock().unwrap();
        let split_screen = settings.split_screen;
        let backend = settings.backend;
        drop(settings);

        let compositor = gst::ElementFactory::make(self.get_pipeline_compositor(backend))
            .build()
            .expect("Failed to create compositor element");
        compositor.set_property("name", "compositor");

        if split_screen && backend != Backend::GL {
            let crop0 = gst::ElementFactory::make("videocrop")
                .build()
                .expect("Failed to create crop0");
            crop0.set_property("name", "crop0");

            let crop1 = gst::ElementFactory::make("videocrop")
                .build()
                .expect("Failed to create crop1");
            crop1.set_property("name", "crop1");

            self.obj().add(&crop0).expect("Failed to add crop0 element");
            self.obj().add(&crop1).expect("Failed to add crop1 element");
        }

        self.link_elements(&compositor, split_screen, backend)?;

        self.add_overlay_probe(&self.overlay0);
        self.add_overlay_probe(&self.overlay1);

        unsafe {
            self.sinkpad0.set_event_full_function(|pad, parent, event| {
                VideoCompareMixer::catch_panic_pad_function(
                    parent,
                    || false,
                    |video_compare_mixer| video_compare_mixer.sink_event(&pad.clone().upcast::<gst::Pad>(), event),
                );
                Ok(gst::FlowSuccess::Ok)
            });
        }

        Ok(())
    }

    fn add_overlay_probe(&self, overlay: &gst::Element) {
        let overlay_src_pad = overlay.static_pad("video_sink").unwrap();
        let overlay_clone = overlay.clone();
        overlay_src_pad.add_probe(gst::PadProbeType::BUFFER, move |_: &gst::Pad, probe_info| {
            let Some(buffer) = probe_info.buffer_mut() else {
                return gst::PadProbeReturn::Ok;
            };

            if let Some(statsmeta) = buffer.meta::<VideoEncoderStatsMeta>() {
                let stats = statsmeta.stats();
                let stats_string = format!("{stats}");
                overlay_clone.set_property("text", stats_string);
            }

            gst::PadProbeReturn::Ok
        });
    }

    fn link_elements(
        &self,
        compositor: &gst::Element,
        split_screen: bool,
        backend: Backend,
    ) -> Result<(), gst::ErrorMessage> {
        self.overlay0.set_property_from_str("line-alignment", "left");
        self.overlay0.set_property_from_str("halignment", "left");
        self.overlay0.set_property_from_str("valignment", "top");
        self.overlay1.set_property_from_str("line-alignment", "right");
        self.overlay1.set_property_from_str("halignment", "right");
        self.overlay1.set_property_from_str("valignment", "top");

        let compositor_pad0 = compositor
            .request_pad_simple("sink_0")
            .expect("Failed to request pad sink_0");
        let compositor_pad1 = compositor
            .request_pad_simple("sink_1")
            .expect("Failed to request pad sink_1");

        self.obj()
            .add(compositor)
            .expect("Failed to add compositor element");
        self.obj()
            .add(&self.queue0)
            .expect("Failed to add queue0 element");
        self.obj()
            .add(&self.queue1)
            .expect("Failed to add queue1 element");
        self.obj()
            .add(&self.overlay0)
            .expect("Failed to add overlay0 element");
        self.obj()
            .add(&self.overlay1)
            .expect("Failed to add overlay1 element");

        self.sinkpad0
            .set_target(Some(&self.queue0.static_pad("sink").unwrap()))
            .expect("Failed to link sinkpad0 to queue0");
        self.sinkpad1
            .set_target(Some(&self.queue1.static_pad("sink").unwrap()))
            .expect("Failed to link sinkpad1 to queue1");

        self.srcpad
            .set_target(Some(&compositor.static_pad("src").unwrap()))
            .expect("Failed to link srcpad to compositor");

        if split_screen && backend != Backend::GL {
            // Get crop elements by name since we can't store them in struct easily
            let crop0 = self.obj().by_name("crop0").expect("crop0 should exist");
            let crop1 = self.obj().by_name("crop1").expect("crop1 should exist");

            self.queue0
                .static_pad("src")
                .unwrap()
                .link(&self.overlay0.static_pad("video_sink").unwrap())
                .expect("Failed to link queue0 to overlay0");
            self.overlay0
                .static_pad("src")
                .unwrap()
                .link(&crop0.static_pad("sink").unwrap())
                .expect("Failed to link overlay0 to crop0");
            crop0
                .static_pad("src")
                .unwrap()
                .link(&compositor_pad0)
                .expect("Failed to link crop0 to queue2");
            self.queue1
                .static_pad("src")
                .unwrap()
                .link(&self.overlay1.static_pad("video_sink").unwrap())
                .expect("Failed to link queue1 to overlay1");
            self.overlay1
                .static_pad("src")
                .unwrap()
                .link(&crop1.static_pad("sink").unwrap())
                .expect("Failed to link overlay1 to crop1");
            crop1
                .static_pad("src")
                .unwrap()
                .link(&compositor_pad1)
                .expect("Failed to link crop1 to queue3");
        } else {
            // Direct connection without crops - overlay mode
            self.queue0
                .static_pad("src")
                .unwrap()
                .link(&self.overlay0.static_pad("video_sink").unwrap())
                .expect("Failed to link queue0 to overlay0");
            self.overlay0
                .static_pad("src")
                .unwrap()
                .link(&compositor_pad0)
                .expect("Failed to link overlay0 to queue2");
            self.queue1
                .static_pad("src")
                .unwrap()
                .link(&self.overlay1.static_pad("video_sink").unwrap())
                .expect("Failed to link queue1 to overlay1");
            self.overlay1
                .static_pad("src")
                .unwrap()
                .link(&compositor_pad1)
                .expect("Failed to link overlay1 to queue3");
        }

        self.queue0.sync_state_with_parent().unwrap();
        self.queue1.sync_state_with_parent().unwrap();
        self.overlay0.sync_state_with_parent().unwrap();
        self.overlay1.sync_state_with_parent().unwrap();
        self.obj().by_name("compositor").unwrap().sync_state_with_parent().unwrap();
        Ok(())
    }

    fn sink_event(&self, pad: &gst::Pad, event: gst::Event) -> bool {
        gst::log!(CAT, obj = pad, "Handling sink event {:?}", event);

        use gst::EventView::*;
        match event.view() {
            Caps(c) => {
                let caps = c.caps();
                let s = caps.structure(0).unwrap();
                let width = s.get::<i32>("width").unwrap();
                let half_width = width / 2;

                let settings = self.settings.lock().unwrap();
                let split_screen = settings.split_screen;
                let backend = settings.backend;
                drop(settings);

                let compositor_sink1_pad = self.obj().by_name("compositor").unwrap().static_pad("sink_1").unwrap();
                if split_screen {
                    if backend != Backend::GL {
                        // Set crop properties for both crops
                        if let Some(crop0) = self.obj().by_name("crop0") {
                            crop0.set_property("right", half_width);
                        }
                        if let Some(crop1) = self.obj().by_name("crop1") {
                            crop1.set_property("left", half_width);
                        }
                    } else {
                        let compositor_sink0_pad = self.obj().by_name("compositor").unwrap().static_pad("sink_0").unwrap();
                        compositor_sink0_pad.set_property("crop-right", half_width);
                        compositor_sink1_pad.set_property("crop-left", half_width);
                    }
                    compositor_sink1_pad.set_property("xpos", half_width);
                } else {
                    compositor_sink1_pad.set_property("xpos", width);
                }
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
impl ObjectSubclass for VideoCompareMixer {
    const NAME: &'static str = "GstVideoCompareMixer";
    type Type = super::VideoCompareMixer;
    type ParentType = gst::Bin;

    fn with_class(klass: &Self::Class) -> Self {
        let templ = klass.pad_template("sink_0").unwrap();
        let sinkpad0 = gst::GhostPad::from_template(&templ);

        let templ = klass.pad_template("sink_1").unwrap();
        let sinkpad1 = gst::GhostPad::from_template(&templ);

        let templ = klass.pad_template("src").unwrap();
        let srcpad = gst::GhostPad::from_template(&templ);

        let queue0 = gst::ElementFactory::make("queue")
            .build()
            .expect("Failed to create queue0");
        queue0.set_property("name", "queue0");

        let queue1 = gst::ElementFactory::make("queue")
            .build()
            .expect("Failed to create queue1");
        queue1.set_property("name", "queue1");

        let overlay0 = gst::ElementFactory::make("textoverlay")
            .build()
            .expect("Failed to create overlay0");
        overlay0.set_property("name", "overlay0");

        let overlay1 = gst::ElementFactory::make("textoverlay")
            .build()
            .expect("Failed to create overlay1");
        overlay1.set_property("name", "overlay1");

        Self {
            srcpad,
            sinkpad0,
            sinkpad1,
            queue0,
            queue1,
            overlay0,
            overlay1,
            settings: Mutex::new(Settings::default()),
        }
    }
}

impl ObjectImpl for VideoCompareMixer {
    // TODO
    // navigation-evets = default true

    fn properties() -> &'static [glib::ParamSpec] {
        static PROPERTIES: LazyLock<Vec<glib::ParamSpec>> = LazyLock::new(|| {
            vec![
                glib::ParamSpecEnum::builder_with_default("backend", Backend::default())
                    .nick("The backend to use for mixing the video")
                    .blurb("The backend to use for mixing the video")
                    .mutable_ready()
                    .build(),
                glib::ParamSpecBoolean::builder("split-screen")
                    .nick("Split Screen Mode")
                    .blurb("Enable split-screen mode with cropping")
                    .default_value(false)
                    .mutable_ready()
                    .build(),
            ]
        });

        PROPERTIES.as_ref()
    }

    fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
        let mut settings = self.settings.lock().unwrap();
        match pspec.name() {
            "backend" => {
                settings.backend = value.get().expect("type checked upstream");

                gst::info!(
                    CAT,
                    imp = self,
                    "Set backend to {:?}",
                    settings.backend
                );
            }
            "split-screen" => {
                settings.split_screen = value.get().expect("type checked upstream");

                gst::info!(
                    CAT,
                    imp = self,
                    "Set split-screen to {:?}",
                    settings.split_screen
                );
            }
            _ => unimplemented!(),
        }
    }

    fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
        let settings = self.settings.lock().unwrap();
        match pspec.name() {
            "backend" => settings.backend.to_value(),
            "split-screen" => settings.split_screen.to_value(),
            _ => unimplemented!(),
        }
    }

    fn constructed(&self) {
        gst::info!(CAT, "Constructing VideoCompareMixer");
        self.parent_constructed();

        let obj = self.obj();
        obj.add_pad(&self.sinkpad0).unwrap();
        obj.add_pad(&self.sinkpad1).unwrap();
        obj.add_pad(&self.srcpad).unwrap();
    }
}

impl GstObjectImpl for VideoCompareMixer {}

impl ElementImpl for VideoCompareMixer {
    fn metadata() -> Option<&'static gst::subclass::ElementMetadata> {
        static ELEMENT_METADATA: LazyLock<gst::subclass::ElementMetadata> = LazyLock::new(|| {
            gst::subclass::ElementMetadata::new(
                "VideoCompareMixer",
                "Video/Mixer/Filter",
                "Video Compare Mixer Wrapper",
                "Diego Nieto <dnieto@fluendo.com>",
            )
        });

        Some(&*ELEMENT_METADATA)
    }

    fn pad_templates() -> &'static [gst::PadTemplate] {
        static PAD_TEMPLATES: LazyLock<Vec<gst::PadTemplate>> = LazyLock::new(|| {
            let caps = gst_video::VideoCapsBuilder::new().build();

            let video_src_pad_template = gst::PadTemplate::new(
                "src",
                gst::PadDirection::Src,
                gst::PadPresence::Always,
                &caps,
            )
            .unwrap();

            let video_sink_0_pad_template = gst::PadTemplate::new(
                "sink_0",
                gst::PadDirection::Sink,
                gst::PadPresence::Always,
                &caps,
            )
            .unwrap();

            let video_sink_1_pad_template = gst::PadTemplate::new(
                "sink_1",
                gst::PadDirection::Sink,
                gst::PadPresence::Always,
                &caps,
            )
            .unwrap();

            vec![
                video_src_pad_template,
                video_sink_0_pad_template,
                video_sink_1_pad_template,
            ]
        });

        PAD_TEMPLATES.as_ref()
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

impl BinImpl for VideoCompareMixer {
}

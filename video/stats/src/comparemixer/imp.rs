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
use gst_video::NavigationEvent;

use crate::comparemixer::compositor::Compositor;
use crate::comparemixer::compositor::Mode;
use crate::comparemixer::compositor::Position;
use crate::videoencoderstatsmeta::VideoEncoderStatsMeta;

use std::fmt;
use std::sync::{Arc, LazyLock, Mutex};
use std::vec::Vec;

static CAT: LazyLock<gst::DebugCategory> = LazyLock::new(|| {
    gst::DebugCategory::new(
        "video-compare-mixer",
        gst::DebugColorFlags::empty(),
        Some("GstVideoCompareMixer"),
    )
});

#[derive(Default)]
pub struct MouseState {
    clicked: bool,
    clicked_x: f64,
    clicked_y: f64,
    clicked_xpos: i32,
    clicked_ypos: i32,
}

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
    navigation_events: bool,
    overlay_stats: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            backend: Backend::default(),
            split_screen: false,
            navigation_events: true,
            overlay_stats: true,
        }
    }
}

pub struct VideoCompareMixer {
    srcpad: gst::GhostPad,
    sinkpad0: gst::GhostPad,
    sinkpad1: gst::GhostPad,
    queue0: gst::Element,
    queue1: gst::Element,
    overlay: gst::Element,
    settings: Mutex<Settings>,
    mouse_state: Arc<Mutex<MouseState>>,
    compositor_helper: Arc<Mutex<Option<Compositor>>>,
    queue_stats: Arc<Mutex<QueueStats>>,
}

#[derive(Default)]
pub struct QueueStats {
    stats0: Option<String>,
    stats1: Option<String>,
    framerate: Option<gst::Fraction>,
}

impl fmt::Display for QueueStats {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let stats0_str = self.stats0.as_deref().unwrap_or("No stats available");
        let stats1_str = self.stats1.as_deref().unwrap_or("No stats available");

        // Parse stats from both sides
        let (encoder0, size0, bitrate0, proc_time0, cpu0, vmaf0, latency0) =
            parse_stats_string(stats0_str);
        let (encoder1, size1, bitrate1, proc_time1, cpu1, vmaf1, latency1) =
            parse_stats_string(stats1_str);

        let total_width: usize = 60;
        let encoder0_full = format!("Encoder: {}", encoder0);
        let encoder1_full = format!("Encoder: {}", encoder1);
        let encoder_spacing = total_width.saturating_sub(encoder0_full.len() + encoder1_full.len());
        writeln!(
            f,
            "{}{:>width$}{}",
            encoder0_full,
            "",
            encoder1_full,
            width = encoder_spacing
        )?;

        let size0_full = format!("Output size: {}", size0);
        let size1_full = format!("Output size: {}", size1);
        let size_spacing = total_width.saturating_sub(size0_full.len() + size1_full.len());
        writeln!(
            f,
            "{}{:>width$}{}",
            size0_full,
            "",
            size1_full,
            width = size_spacing
        )?;

        let bitrate0_full = format!("Bitrate: {}", bitrate0);
        let bitrate1_full = format!("Bitrate: {}", bitrate1);
        let bitrate_spacing = total_width.saturating_sub(bitrate0_full.len() + bitrate1_full.len());
        writeln!(
            f,
            "{}{:>width$}{}",
            bitrate0_full,
            "",
            bitrate1_full,
            width = bitrate_spacing
        )?;

        let proc_time0_full = format!("Processing time: {}", proc_time0);
        let proc_time1_full = format!("Processing time: {}", proc_time1);
        let proc_time_spacing =
            total_width.saturating_sub(proc_time0_full.len() + proc_time1_full.len());
        writeln!(
            f,
            "{}{:>width$}{}",
            proc_time0_full,
            "",
            proc_time1_full,
            width = proc_time_spacing
        )?;

        let cpu0_full = format!("CPU: {}", cpu0);
        let cpu1_full = format!("CPU: {}", cpu1);
        let cpu_spacing = total_width.saturating_sub(cpu0_full.len() + cpu1_full.len());
        writeln!(
            f,
            "{}{:>width$}{}",
            cpu0_full,
            "",
            cpu1_full,
            width = cpu_spacing
        )?;

        let vmaf0_full = format!("VMAF: {}", vmaf0);
        let vmaf1_full = format!("VMAF: {}", vmaf1);
        let vmaf_spacing = total_width.saturating_sub(vmaf0_full.len() + vmaf1_full.len());
        writeln!(
            f,
            "{}{:>width$}{}",
            vmaf0_full,
            "",
            vmaf1_full,
            width = vmaf_spacing
        )?;

        let latency0_full = format!("Encoding latency: {}", latency0);
        let latency1_full = format!("Encoding latency: {}", latency1);
        let latency_spacing = total_width.saturating_sub(latency0_full.len() + latency1_full.len());
        writeln!(
            f,
            "{}{:>width$}{}",
            latency0_full,
            "",
            latency1_full,
            width = latency_spacing
        )
    }
}

fn parse_stats_string(stats_str: &str) -> (String, String, String, String, String, String, String) {
    let mut encoder = "Unknown".to_string();
    let mut size = "0 KB".to_string();
    let mut bitrate = "0.000 kbps".to_string();
    let mut proc_time = "0.00 ms".to_string();
    let mut cpu = "0 s".to_string();
    let mut vmaf = "N/A".to_string();
    let mut latency = "0.000 ms".to_string();

    for line in stats_str.lines() {
        if line.starts_with("Encoder: ") {
            encoder = line.replace("Encoder: ", "");
        } else if line.starts_with("Output size: ") {
            size = line.replace("Output size: ", "");
        } else if line.starts_with("Bitrate: ") {
            bitrate = line.replace("Bitrate: ", "");
        } else if line.starts_with("Processing time: ") {
            proc_time = line.replace("Processing time: ", "");
        } else if line.starts_with("CPU: ") {
            cpu = line.replace("CPU: ", "");
        } else if line.starts_with("VMAF: ") {
            vmaf = line.replace("VMAF: ", "");
        } else if line.starts_with("Encode latency: ") {
            latency = line.replace("Encode latency: ", "");
        }
    }

    (encoder, size, bitrate, proc_time, cpu, vmaf, latency)
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

    pub fn add_navigation_events_probe(&self) {
        let compositor_supports_crop: bool = self.settings.lock().unwrap().backend == Backend::GL;

        let mixer = self.obj().by_name("compositor").unwrap();
        let crop0 = self.obj().by_name("crop0").unwrap();
        let crop1 = self.obj().by_name("crop1").unwrap();
        let mixer_src_pad = mixer.static_pad("src").unwrap();
        let mixer_sink_0_pad = mixer.static_pad("sink_0").unwrap();
        let mixer_sink_1_pad = mixer.static_pad("sink_1").unwrap();

        self.update_mixer(
            &(*self.compositor_helper.lock().unwrap()).unwrap(),
            &mixer_sink_0_pad,
            &mixer_sink_1_pad,
            &crop0,
            &crop1,
            compositor_supports_crop,
        );

        let compositor_helper_clone = self.compositor_helper.clone();
        let mouse_state = self.mouse_state.clone();
        let imp_weak = self.downgrade();
        // Probe added in the sink pad to get direct navigation events w/o transformation done by the zoom_mixer
        mixer_src_pad.add_probe(gst::PadProbeType::EVENT_UPSTREAM, move |_, probe_info| {
            let Some(ev) = probe_info.event() else {
                return gst::PadProbeReturn::Ok;
            };

            if ev.type_() != gst::EventType::Navigation {
                return gst::PadProbeReturn::Ok;
            };

            let Ok(nav_event) = NavigationEvent::parse(ev) else {
                return gst::PadProbeReturn::Ok;
            };

            let compositor = &mut (*compositor_helper_clone.lock().unwrap()).unwrap();
            let original_compositor = *compositor;
            let nav_event_clone = nav_event.clone();

            match nav_event {
                NavigationEvent::KeyPress { key, .. } => match key.as_str() {
                    "Left" | "Left arrow" => {
                        compositor.move_pos(-10, 0);
                    }
                    "Right" | "Right arrow" => {
                        compositor.move_pos(10, 0);
                    }
                    "Up" | "Up arrow" => {
                        compositor.move_pos(0, -10);
                    }
                    "Down" | "Down arrow" => {
                        compositor.move_pos(0, 10);
                    }
                    "plus" | "+" => {
                        compositor.zoom_in();
                    }
                    "minus" | "-" => {
                        compositor.zoom_out();
                    }
                    "r" => {
                        compositor.reset_position();
                    }
                    "Shift_R" | "R" => {
                        compositor.reset();
                    }
                    "1" => {
                        compositor.split_mode();
                        let w = compositor.width;
                        compositor.move_border_to(w);
                    }
                    "2" => {
                        compositor.split_mode();
                        compositor.move_border_to(0);
                    }
                    "3" => {
                        compositor.split_mode();
                        compositor.reset_border();
                    }
                    "4" => {
                        compositor.side_by_side_mode();
                    }
                    "5" => {
                        compositor.split_mode();
                        compositor.move_border(-10);
                    }
                    "6" => {
                        compositor.split_mode();
                        compositor.move_border(10);
                    }
                    _ => {
                        gst::info!(CAT, "Unhandled key: {}", key);
                    }
                },
                NavigationEvent::MouseMove { x, y, .. } => {
                    let state = mouse_state.lock().unwrap();
                    if state.clicked {
                        let new_xpos = (x - state.clicked_x) as i32 + state.clicked_xpos;
                        let new_ypos = (y - state.clicked_y) as i32 + state.clicked_ypos;

                        compositor.move_pos_to(new_xpos, new_ypos);
                    }
                }
                NavigationEvent::MouseButtonPress { button, x, y, .. } => {
                    if button == 1 || button == 272 {
                        let mut state = mouse_state.lock().unwrap();
                        state.clicked = true;
                        state.clicked_x = x;
                        state.clicked_y = y;
                        state.clicked_xpos = compositor.offset_x;
                        state.clicked_ypos = compositor.offset_y;

                        if y >= 600.0 {
                            compositor.move_border_to(x as i32);
                        }
                    } else if button == 2 || button == 3 || button == 274 || button == 273 {
                        compositor.reset();
                    } else if button == 4 {
                        compositor.zoom_in_center_at(x as i32, y as i32);
                    } else if button == 5 {
                        compositor.zoom_out_center_at(x as i32, y as i32);
                    }
                }
                NavigationEvent::MouseButtonRelease { button, .. } => {
                    if button == 1 || button == 272 {
                        let mut state = mouse_state.lock().unwrap();
                        state.clicked = false;
                    }
                }
                // FIXME add support for scroll events
                // NavigationEvent::MouseScroll { x, y, delta_x, delta_y, ..} => {
                //     if delta_y > 0.0 {
                //         compositor.zoom_in_center_at(x as i32, y as i32);
                //     } else if delta_y < 0.0 {
                //         compositor.zoom_out_center_at(x as i32, y as i32);
                //     }
                // }
                _ => (),
            }

            if original_compositor != *compositor {
                gst::log!(CAT, "Compositor changed: {compositor:?}");
                let Some(imp) = imp_weak.upgrade() else {
                    return gst::PadProbeReturn::Ok;
                };
                imp.update_mixer(
                    compositor,
                    &mixer_sink_0_pad,
                    &mixer_sink_1_pad,
                    &crop0,
                    &crop1,
                    compositor_supports_crop,
                );
                *imp.compositor_helper.lock().unwrap() = Some(*compositor);
            }

            gst::log!(CAT, "Navigation event: {nav_event_clone:?}");

            gst::PadProbeReturn::Ok
        });
    }

    fn fix_pos(pos: &mut Position, width: i32, compositor_supports_crop: bool) {
        // workaround to handle gst issue when width==0 with any video mixers
        // see `glvideomixer sink_0::width=0` in README.md
        if pos.width == 0 {
            pos.width = width;
            pos.xpos = width;
        }

        // workaround to handle gst issue when crop==total_width with compositor and vacompositor
        // see `compositor and vacompositor video out of the box` in README.md
        if !compositor_supports_crop {
            if pos.crop_right == width {
                pos.crop_right = width - 10;
            }

            if pos.crop_left == width {
                pos.crop_left = width - 10;
            }
        }
    }

    fn update_mixer(
        &self,
        compositor_helper: &Compositor,
        mixer_sink_0_pad: &gst::Pad,
        mixer_sink_1_pad: &gst::Pad,
        crop0: &gst::Element,
        crop1: &gst::Element,
        compositor_supports_crop: bool,
    ) {
        let (mut pos0, mut pos1) = compositor_helper.get_positions();

        Self::fix_pos(&mut pos0, compositor_helper.width, compositor_supports_crop);
        Self::fix_pos(&mut pos1, compositor_helper.width, compositor_supports_crop);

        gst::log!(
            CAT,
            "Position 0: {}x{}+{}+{}, crop_right: {}",
            pos0.width,
            pos0.height,
            pos0.xpos,
            pos0.ypos,
            pos0.crop_right
        );
        gst::log!(
            CAT,
            "Position 1: {}x{}+{}+{}, crop_left: {}",
            pos1.width,
            pos1.height,
            pos1.xpos,
            pos1.ypos,
            pos1.crop_left
        );

        //TODO refactor avoid copy and paste
        if compositor_supports_crop {
            mixer_sink_0_pad.set_properties(&[
                ("width", &pos0.width),
                ("height", &pos0.height),
                ("xpos", &pos0.xpos),
                ("ypos", &pos0.ypos),
                ("crop-right", &pos0.crop_right),
            ]);

            mixer_sink_1_pad.set_properties(&[
                ("width", &pos1.width),
                ("height", &pos1.height),
                ("xpos", &pos1.xpos),
                ("ypos", &pos1.ypos),
                ("crop-left", &pos1.crop_left),
            ]);
        } else {
            mixer_sink_0_pad.set_properties(&[
                ("width", &pos0.width),
                ("height", &pos0.height),
                ("xpos", &pos0.xpos),
                ("ypos", &pos0.ypos),
            ]);

            mixer_sink_1_pad.set_properties(&[
                ("width", &pos1.width),
                ("height", &pos1.height),
                ("xpos", &pos1.xpos),
                ("ypos", &pos1.ypos),
            ]);

            gst::log!(
                CAT,
                "right crop: {}, left crop: {}",
                pos0.crop_right,
                pos1.crop_left
            );
            crop0.set_property("right", pos0.crop_right);
            crop1.set_property("left", pos1.crop_left);
        }
    }

    fn prepare_pipeline(&self) -> Result<(), gst::ErrorMessage> {
        let settings = self.settings.lock().unwrap();
        let backend = settings.backend;
        drop(settings);

        let compositor = gst::ElementFactory::make(self.get_pipeline_compositor(backend))
            .build()
            .expect("Failed to create compositor element");
        compositor.set_property("name", "compositor");

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

        self.link_elements(&compositor, backend)?;

        self.add_queue_probe(&self.queue0, 0);
        self.add_queue_probe(&self.queue1, 1);

        unsafe {
            self.sinkpad0.set_event_full_function(|pad, parent, event| {
                VideoCompareMixer::catch_panic_pad_function(
                    parent,
                    || false,
                    |video_compare_mixer| {
                        video_compare_mixer.sink_event(&pad.clone().upcast::<gst::Pad>(), event)
                    },
                );
                Ok(gst::FlowSuccess::Ok)
            });
        }

        Ok(())
    }

    fn add_queue_probe(&self, queue: &gst::Element, queue_index: u8) {
        let queue_src_pad = queue.static_pad("src").unwrap();
        let queue_stats = self.queue_stats.clone();
        queue_src_pad.add_probe(
            gst::PadProbeType::BUFFER,
            move |_: &gst::Pad, probe_info| {
                let Some(buffer) = probe_info.buffer_mut() else {
                    return gst::PadProbeReturn::Ok;
                };

                let mut stats = queue_stats.lock().unwrap();

                // Only update at framerate intervals, similar to encoderstats
                let fps_n: i32;
                if let Some(fps) = stats.framerate {
                    fps_n = fps.numer();
                } else {
                    return gst::PadProbeReturn::Ok;
                }

                if let Some(statsmeta) = buffer.meta::<VideoEncoderStatsMeta>() {
                    let num_buffers = statsmeta.stats().num_buffers;

                    // Only update once per second based on framerate
                    if num_buffers % (fps_n as u64) == 0 {
                        let stats_string = format!("{}", statsmeta.stats());
                        match queue_index {
                            0 => stats.stats0 = Some(stats_string),
                            1 => stats.stats1 = Some(stats_string),
                            _ => {}
                        }
                    }
                }

                gst::PadProbeReturn::Ok
            },
        );
    }

    fn add_compositor_src_probe(&self, compositor: &gst::Element) {
        let compositor_src_pad = compositor.static_pad("src").unwrap();
        let overlay = self.overlay.clone();
        let queue_stats = self.queue_stats.clone();
        let imp_weak = self.downgrade();

        compositor_src_pad.add_probe(
            gst::PadProbeType::BUFFER,
            move |_: &gst::Pad, probe_info| {
                let Some(_) = probe_info.buffer() else {
                    return gst::PadProbeReturn::Ok;
                };

                let Some(imp) = imp_weak.upgrade() else {
                    return gst::PadProbeReturn::Ok;
                };

                if imp.settings.lock().unwrap().overlay_stats {
                    let stats = queue_stats.lock().unwrap();
                    let stats_text = format!("{}", *stats);
                    overlay.set_property("text", stats_text);
                };
                gst::PadProbeReturn::Ok
            },
        );
    }

    fn link_elements(
        &self,
        compositor: &gst::Element,
        backend: Backend,
    ) -> Result<(), gst::ErrorMessage> {
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
            .add(&self.overlay)
            .expect("Failed to add overlay element");

        let caps_filter = gst::ElementFactory::make("capsfilter")
            .name("capsfilter0")
            .build()
            .expect("Failed to create capsfilter0");
        self.obj()
            .add(&caps_filter)
            .expect("Failed to add capsfilter0 element");

        self.sinkpad0
            .set_target(Some(&self.queue0.static_pad("sink").unwrap()))
            .expect("Failed to link sinkpad0 to queue0");
        self.sinkpad1
            .set_target(Some(&self.queue1.static_pad("sink").unwrap()))
            .expect("Failed to link sinkpad1 to queue1");

        compositor
            .link(&self.overlay)
            .expect("Failed to link compositor to overlay");
        self.overlay
            .link(&caps_filter)
            .expect("Failed to link overlay to capsfilter");

        self.srcpad
            .set_target(Some(&caps_filter.static_pad("src").unwrap()))
            .expect("Failed to link srcpad to capsfilter");

        if backend != Backend::GL {
            // Get crop elements by name since we can't store them in struct easily
            let crop0 = self.obj().by_name("crop0").expect("crop0 should exist");
            let crop1 = self.obj().by_name("crop1").expect("crop1 should exist");

            self.queue0
                .static_pad("src")
                .unwrap()
                .link(&crop0.static_pad("sink").unwrap())
                .expect("Failed to link queue0 to crop0");
            crop0
                .static_pad("src")
                .unwrap()
                .link(&compositor_pad0)
                .expect("Failed to link crop0 to compositor");
            self.queue1
                .static_pad("src")
                .unwrap()
                .link(&crop1.static_pad("sink").unwrap())
                .expect("Failed to link queue1 to crop1");
            crop1
                .static_pad("src")
                .unwrap()
                .link(&compositor_pad1)
                .expect("Failed to link crop1 to compositor");
        } else {
            // Direct connection without crops
            self.queue0
                .static_pad("src")
                .unwrap()
                .link(&compositor_pad0)
                .expect("Failed to link queue0 to compositor");
            self.queue1
                .static_pad("src")
                .unwrap()
                .link(&compositor_pad1)
                .expect("Failed to link queue1 to compositor");
        }

        self.add_compositor_src_probe(compositor);

        self.queue0.sync_state_with_parent().unwrap();
        self.queue1.sync_state_with_parent().unwrap();
        self.overlay.sync_state_with_parent().unwrap();
        self.obj()
            .by_name("compositor")
            .unwrap()
            .sync_state_with_parent()
            .unwrap();
        Ok(())
    }

    fn sink_event(&self, pad: &gst::Pad, event: gst::Event) -> bool {
        gst::log!(CAT, obj = pad, "Handling sink event {:?}", event);

        use gst::EventView::*;
        match event.view() {
            Caps(c) => {
                let caps = c.caps();
                gst::info!(CAT, "Received caps {caps:?}");
                let s = caps.structure(0).unwrap();
                let width = s.get::<i32>("width").unwrap();
                let height = s.get::<i32>("height").unwrap();
                let fps = s.get::<gst::Fraction>("framerate").ok();

                // Update framerate in queue_stats
                {
                    let mut stats = self.queue_stats.lock().unwrap();
                    stats.framerate = fps;
                }

                let settings = self.settings.lock().unwrap();
                let split_screen = settings.split_screen;
                let navigation_events = settings.navigation_events;
                drop(settings);

                let caps = format!("video/x-raw,width={},height={}", width, height);
                self.obj()
                    .by_name("capsfilter0")
                    .unwrap()
                    .set_property_from_str("caps", &caps.as_str());

                let compositor_mode = if split_screen {
                    Mode::Split
                } else {
                    Mode::SideBySide
                };

                let compositor_helper = Compositor::new(compositor_mode, width, height);
                *self.compositor_helper.lock().unwrap() = Some(compositor_helper);

                if navigation_events {
                    self.add_navigation_events_probe();
                }
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

        let overlay = gst::ElementFactory::make("textoverlay")
            .build()
            .expect("Failed to create overlay");
        overlay.set_property("name", "overlay");
        overlay.set_property_from_str("font-desc", "Consolas 10");

        Self {
            srcpad,
            sinkpad0,
            sinkpad1,
            queue0,
            queue1,
            overlay,
            settings: Mutex::new(Settings::default()),
            mouse_state: Arc::new(Mutex::new(MouseState::default())),
            compositor_helper: Default::default(),
            queue_stats: Arc::new(Mutex::new(QueueStats::default())),
        }
    }
}

impl ObjectImpl for VideoCompareMixer {
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
                glib::ParamSpecBoolean::builder("navigation-events")
                    .nick("Navigation Events")
                    .blurb("Enable handling of navigation events for controlling the mixer")
                    .default_value(true)
                    .mutable_ready()
                    .build(),
                glib::ParamSpecBoolean::builder("overlay-stats")
                    .nick("Overlay Stats")
                    .blurb("Enable/disable the text overlay displaying encoder statistics")
                    .default_value(true)
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

                gst::info!(CAT, imp = self, "Set backend to {:?}", settings.backend);
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
            "navigation-events" => {
                settings.navigation_events = value.get().expect("type checked upstream");

                gst::info!(
                    CAT,
                    imp = self,
                    "Set navigation-events to {:?}",
                    settings.navigation_events
                );
            }
            "overlay-stats" => {
                settings.overlay_stats = value.get().expect("type checked upstream");

                gst::info!(
                    CAT,
                    imp = self,
                    "Set overlay-stats to {:?}",
                    settings.overlay_stats
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
            "navigation-events" => settings.navigation_events.to_value(),
            "overlay-stats" => settings.overlay_stats.to_value(),
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

impl BinImpl for VideoCompareMixer {}

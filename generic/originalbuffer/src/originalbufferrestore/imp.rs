// Copyright (C) 2024 Collabora Ltd
//   @author: Olivier Crête <olivier.crete@collabora.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v2.0.
// If a copy of the MPL was not distributed with this file, You can obtain one at
// <https://mozilla.org/MPL/2.0/>.
//
// SPDX-License-Identifier: MPL-2.0

use gst::glib;
use gst::subclass::prelude::*;
use gst_video::prelude::*;

use atomic_refcell::AtomicRefCell;

use crate::originalbuffermeta;
use crate::originalbuffermeta::OriginalBufferMeta;

struct CapsState {
    caps: gst::Caps,
    vinfo: Option<gst_video::VideoInfo>,
}

impl Default for CapsState {
    fn default() -> Self {
        CapsState {
            caps: gst::Caps::new_empty(),
            vinfo: None,
        }
    }
}

#[derive(Default)]
struct State {
    sinkpad_caps: CapsState,
    meta_caps: CapsState,
    sinkpad_segment: Option<gst::Event>,
    modified_src_pad_requested: bool,
}

pub struct OriginalBufferRestore {
    state: AtomicRefCell<State>,
    src_pad: gst::Pad,
    sink_pad: gst::Pad,
}

use std::sync::LazyLock;
#[allow(dead_code)]
static CAT: LazyLock<gst::DebugCategory> = LazyLock::new(|| {
    gst::DebugCategory::new(
        "originalbufferrestore",
        gst::DebugColorFlags::empty(),
        Some("Restore Original buffer as meta"),
    )
});

#[glib::object_subclass]
impl ObjectSubclass for OriginalBufferRestore {
    const NAME: &'static str = "GstOriginalBufferRestore";
    type Type = super::OriginalBufferRestore;
    type ParentType = gst::Element;
    type Interfaces = (gst::ChildProxy,);

    fn with_class(klass: &Self::Class) -> Self {
        let sink_templ = klass.pad_template("sink").unwrap();
        let src_templ = klass.pad_template("src").unwrap();

        let sink_pad = gst::Pad::builder_from_template(&sink_templ)
            .chain_function(|pad, parent, buffer| {
                OriginalBufferRestore::catch_panic_pad_function(
                    parent,
                    || Err(gst::FlowError::Error),
                    |obj| obj.sink_chain(pad, buffer),
                )
            })
            .event_function(|pad, parent, event| {
                OriginalBufferRestore::catch_panic_pad_function(
                    parent,
                    || false,
                    |obj| obj.sink_event(pad, parent, event),
                )
            })
            .query_function(|pad, parent, query| {
                OriginalBufferRestore::catch_panic_pad_function(
                    parent,
                    || false,
                    |obj| obj.sink_query(pad, parent, query),
                )
            })
            .build();

        let src_pad = gst::Pad::builder_from_template(&src_templ)
            .event_function(|pad, parent, event| {
                OriginalBufferRestore::catch_panic_pad_function(
                    parent,
                    || false,
                    |obj| obj.src_event(pad, parent, event),
                )
            })
            .build();

        Self {
            src_pad,
            sink_pad,
            state: Default::default(),
        }
    }
}

impl ChildProxyImpl for OriginalBufferRestore {
    fn children_count(&self) -> u32 {
        let object = self.obj();
        object.num_pads() as u32
    }

    fn child_by_name(&self, name: &str) -> Option<glib::Object> {
        let object = self.obj();
        object
            .pads()
            .into_iter()
            .find(|p| p.name() == name)
            .map(|p| p.upcast())
    }

    fn child_by_index(&self, index: u32) -> Option<glib::Object> {
        let object = self.obj();
        object
            .pads()
            .into_iter()
            .nth(index as usize)
            .map(|p| p.upcast())
    }
}

impl ObjectImpl for OriginalBufferRestore {
    fn constructed(&self) {
        self.parent_constructed();

        let obj = self.obj();
        obj.add_pad(&self.sink_pad).unwrap();
        obj.add_pad(&self.src_pad).unwrap();
    }
}

impl GstObjectImpl for OriginalBufferRestore {}

impl ElementImpl for OriginalBufferRestore {
    fn metadata() -> Option<&'static gst::subclass::ElementMetadata> {
        static ELEMENT_METADATA: LazyLock<gst::subclass::ElementMetadata> = LazyLock::new(|| {
            gst::subclass::ElementMetadata::new(
                "Original Buffer Restore",
                "Generic",
                "Restores a reference to the buffer in a meta",
                "Olivier Crête <olivier.crete@collabora.com>",
            )
        });

        Some(&*ELEMENT_METADATA)
    }

    fn pad_templates() -> &'static [gst::PadTemplate] {
        static PAD_TEMPLATES: LazyLock<Vec<gst::PadTemplate>> = LazyLock::new(|| {
            let caps = gst::Caps::new_any();
            let src_pad_template = gst::PadTemplate::new(
                "src",
                gst::PadDirection::Src,
                gst::PadPresence::Always,
                &caps,
            )
            .unwrap();
            let src_modified_pad_template = gst::PadTemplate::new(
                "modified_src",
                gst::PadDirection::Src,
                gst::PadPresence::Request,
                &caps,
            )
            .unwrap();
            let sink_pad_template = gst::PadTemplate::new(
                "sink",
                gst::PadDirection::Sink,
                gst::PadPresence::Always,
                &caps,
            )
            .unwrap();

            vec![src_pad_template, src_modified_pad_template, sink_pad_template]
        });

        PAD_TEMPLATES.as_ref()
    }

    fn request_new_pad(
        &self,
        templ: &gst::PadTemplate,
        name: Option<&str>,
        _caps: Option<&gst::Caps>,
    ) -> Option<gst::Pad> {
        match templ.name_template() {
            "modified_src" => {
                gst::error!(CAT, imp = self, "Requesting modified src pad");

                let mut state = self.state.borrow_mut();

                if state.modified_src_pad_requested {
                    gst::warning!(CAT, imp = self, "modified_pad has already been requested");

                    return None;
                }
                state.modified_src_pad_requested = true;
                drop(state);

                let modified_src_pad = gst::Pad::builder_from_template(templ)
                    .name("modified_src_pad")
                    .flags(gst::PadFlags::FIXED_CAPS)
                    .build();

                let stream_start_evt = gst::event::StreamStart::builder("originalbufferrestore")
                    .group_id(gst::GroupId::next())
                    .build();

                modified_src_pad.set_active(true).unwrap();
                modified_src_pad.push_event(stream_start_evt);

                self.obj()
                    .add_pad(&modified_src_pad)
                    .expect("Failed to add modified pad");

                let _ = self
                    .obj()
                    .post_message(gst::message::Latency::builder().src(&*self.obj()).build());


                self.obj().child_added(&modified_src_pad, &modified_src_pad.name());

                Some(modified_src_pad.upcast())
            }
            _ => None,
        }
    }

    fn release_pad(&self, pad: &gst::Pad) {
        gst::error!(CAT, imp = self, "Releasing pad: {}", pad.name());

        self.parent_release_pad(pad);

        self.obj().child_removed(pad, &pad.name());
    }

    fn change_state(
        &self,
        transition: gst::StateChange,
    ) -> Result<gst::StateChangeSuccess, gst::StateChangeError> {
        let ret = self.parent_change_state(transition)?;
        if transition == gst::StateChange::PausedToReady {
            let mut state = self.state.borrow_mut();
            *state = State::default();
        }

        Ok(ret)
    }
}

impl OriginalBufferRestore {
    fn sink_event(
        &self,
        pad: &gst::Pad,
        parent: Option<&impl IsA<gst::Object>>,
        event: gst::Event,
    ) -> bool {
        gst::error!(
            CAT,
            imp = self,
            "Sink pad event: {:?}, type: {:?}",
            event,
            event.type_()
        );
        let mut state = self.state.borrow_mut();
        match event.view() {
            gst::EventView::Caps(e) => {
                let caps = e.caps_owned();
                let vinfo = gst_video::VideoInfo::from_caps(&caps).ok();
                state.sinkpad_caps = CapsState { caps, vinfo };
                gst::error!(
                    CAT,
                    imp = self,
                    "Sink pad caps {:?}, VideoInfo {:?}",
                    state.sinkpad_caps.caps,
                    state.sinkpad_caps.vinfo
                );
                if state.modified_src_pad_requested {
                    if let Some(modified_src_pad) = self.obj().child_by_name("modified_src_pad") {
                        let modified_src_pad = modified_src_pad.downcast::<gst::Pad>().unwrap();
                        gst::Pad::push_event(
                            &modified_src_pad,
                            event.clone(),
                        );
                    }
                }
                true
            }
            gst::EventView::Segment(_) => {
                state.sinkpad_segment = Some(event.clone());
                if state.modified_src_pad_requested {
                    if let Some(modified_src_pad) = self.obj().child_by_name("modified_src_pad") {
                        let modified_src_pad = modified_src_pad.downcast::<gst::Pad>().unwrap();
                        gst::Pad::push_event(
                            &modified_src_pad,
                            event.clone(),
                        );
                    }
                }
                true
            }
            _ => gst::Pad::event_default(pad, parent, event),
        }
    }

    fn src_event(
        &self,
        pad: &gst::Pad,
        parent: Option<&impl IsA<gst::Object>>,
        event: gst::Event,
    ) -> bool {
        if event.type_() == gst::EventType::Reconfigure
            || event.has_name("gst-original-buffer-forward-upstream-event")
        {
            let s = gst::Structure::builder("gst-original-buffer-forward-upstream-event")
                .field("event", event)
                .build();
            let event = gst::event::CustomUpstream::new(s);
            self.sink_pad.push_event(event)
        } else {
            let res = gst::Pad::event_default(pad, parent, event);
            gst::error!(
                CAT,
                imp = self,
                "Src pad event default result: {:?}",
                res,
            );
            res
        }
    }

    fn sink_query(
        &self,
        pad: &gst::Pad,
        parent: Option<&impl IsA<gst::Object>>,
        query: &mut gst::QueryRef,
    ) -> bool {
        gst::error!(
            CAT,
            imp = self,
            "Sink pad query: {:?}",
            query,
        );
        if let gst::QueryViewMut::Custom(_) = query.view_mut() {
            let s = query.structure_mut();
            if s.has_name("gst-original-buffer-forward-query") {
                if let Ok(mut q) = s.get::<gst::Query>("query") {
                    s.remove_field("query");
                    assert!(q.is_writable());
                    let res = self.src_pad.peer_query(q.get_mut().unwrap());

                    s.set("query", q);
                    s.set("result", res);

                    return true;
                }
            }
        }

        gst::Pad::query_default(pad, parent, query)
    }

    fn sink_chain(
        &self,
        pad: &gst::Pad,
        inbuf: gst::Buffer,
    ) -> Result<gst::FlowSuccess, gst::FlowError> {
        gst::error!(
            CAT,
            imp = self,
            "Sink pad chain: {:?}, buffer: {:?}",
            pad.name(),
            inbuf
        );
        let Some(ometa) = inbuf.meta::<OriginalBufferMeta>() else {
            //gst::element_warning!(self, gst::StreamError::Failed, ["Buffer {} is missing the GstOriginalBufferMeta, put originalbuffersave upstream in your pipeline", buffer]);
            return Ok(gst::FlowSuccess::Ok);
        };
        let mut state = self.state.borrow_mut();
        let meta_caps = &mut state.meta_caps;
        if &meta_caps.caps != ometa.caps() {
            if !self.src_pad.push_event(gst::event::Caps::new(ometa.caps())) {
                return Err(gst::FlowError::NotNegotiated);
            }
            meta_caps.caps = ometa.caps().clone();
            meta_caps.vinfo = gst_video::VideoInfo::from_caps(&meta_caps.caps).ok();
        }

        let mut outbuf = ometa.original().copy();

        inbuf
            .copy_into(
                outbuf.make_mut(),
                gst::BufferCopyFlags::TIMESTAMPS | gst::BufferCopyFlags::FLAGS,
                ..,
            )
            .unwrap();

        for meta in inbuf.iter_meta::<gst::Meta>() {
            if meta.api() == originalbuffermeta::OriginalBufferMeta::meta_api() {
                continue;
            }

            if meta.has_tag::<gst::meta::tags::Memory>()
                || meta.has_tag::<gst::meta::tags::MemoryReference>()
            {
                continue;
            }

            if meta.has_tag::<gst_video::video_meta::tags::Size>() {
                if let (Some(ref meta_vinfo), Some(ref sink_vinfo)) =
                    (&state.meta_caps.vinfo, &state.sinkpad_caps.vinfo)
                {
                    if (meta_vinfo.width() != sink_vinfo.width()
                        || meta_vinfo.height() != sink_vinfo.height())
                        && meta
                            .transform(
                                outbuf.make_mut(),
                                &gst_video::video_meta::VideoMetaTransformScale::new(
                                    sink_vinfo, meta_vinfo,
                                ),
                            )
                            .is_ok()
                    {
                        continue;
                    }
                }
            }

            let _ = meta.transform(
                outbuf.make_mut(),
                &gst::meta::MetaTransformCopy::new(false, ..),
            );
        }

        if let Some(event) = state.sinkpad_segment.take() {
            if !self.src_pad.push_event(event) {
                return Err(gst::FlowError::Error);
            }
        }

        if state.modified_src_pad_requested {
            gst::error!(
                CAT,
                imp = self,
                "Modified src pad requested, but not implemented yet"
            );
            if let Some(modified_src_pad) = self.obj().child_by_name("modified_src_pad") {
                let modified_src_pad = modified_src_pad.downcast::<gst::Pad>().unwrap();
                let _ = modified_src_pad.push(inbuf.clone());
            }
        }

        self.src_pad.push(outbuf)
    }
}

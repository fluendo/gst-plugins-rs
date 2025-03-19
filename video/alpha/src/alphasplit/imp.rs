// Copyright (C) 2025 Diego Nieto <dnieto@fluendo.com>
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.
//
// SPDX-License-Identifier: MIT OR Apache-2.0

use gst::glib;
use gst_video::VideoFormat;
use gst::prelude::*;
use gst::subclass::prelude::*;

use std::sync::LazyLock;

static CAT: LazyLock<gst::DebugCategory> = LazyLock::new(|| {
    gst::DebugCategory::new(
        "alphasplit",
        gst::DebugColorFlags::empty(),
        Some("Split alpha channel"),
    )
});

pub struct Alphasplit {
    srcpad: gst::Pad,
    alphapad: gst::Pad,
    sinkpad: gst::Pad,
}

impl Alphasplit {
    fn sink_chain(
        &self,
        pad: &gst::Pad,
        buffer: gst::Buffer,
    ) -> Result<gst::FlowSuccess, gst::FlowError> {
        gst::log!(CAT, obj = pad, "Handling buffer {:?}", buffer);
        self.srcpad.push(buffer)
    }

    fn sink_event(&self, pad: &gst::Pad, event: gst::Event) -> bool {
        gst::log!(CAT, obj = pad, "Handling event {:?}", event);
        self.srcpad.push_event(event)
    }

    fn sink_query(&self, pad: &gst::Pad, query: &mut gst::QueryRef) -> bool {
        gst::log!(CAT, obj = pad, "Handling query {:?}", query);
        self.srcpad.peer_query(query)
    }

    fn src_event(&self, pad: &gst::Pad, event: gst::Event) -> bool {
        gst::log!(CAT, obj = pad, "Handling event {:?}", event);
        self.sinkpad.push_event(event)
    }

    fn src_query(&self, pad: &gst::Pad, query: &mut gst::QueryRef) -> bool {
        gst::log!(CAT, obj = pad, "Handling query {:?}", query);
        self.sinkpad.peer_query(query)
    }
}

#[glib::object_subclass]
impl ObjectSubclass for Alphasplit {
    const NAME: &'static str = "GstAlphasplit";
    type Type = super::Alphasplit;
    type ParentType = gst::Element;

    fn with_class(klass: &Self::Class) -> Self {
        let templ = klass.pad_template("sink").unwrap();
        let sinkpad = gst::Pad::builder_from_template(&templ)
            .chain_function(|pad, parent, buffer| {
                Alphasplit::catch_panic_pad_function(
                    parent,
                    || Err(gst::FlowError::Error),
                    |alphasplit| alphasplit.sink_chain(pad, buffer),
                )
            })
            .event_function(|pad, parent, event| {
                Alphasplit::catch_panic_pad_function(
                    parent,
                    || false,
                    |alphasplit| alphasplit.sink_event(pad, event),
                )
            })
            .query_function(|pad, parent, query| {
                Alphasplit::catch_panic_pad_function(
                    parent,
                    || false,
                    |alphasplit| alphasplit.sink_query(pad, query),
                )
            })
            .build();

        let templ = klass.pad_template("src").unwrap();
        let srcpad = gst::Pad::builder_from_template(&templ)
            .event_function(|pad, parent, event| {
                Alphasplit::catch_panic_pad_function(
                    parent,
                    || false,
                    |alphasplit| alphasplit.src_event(pad, event),
                )
            })
            .query_function(|pad, parent, query| {
                Alphasplit::catch_panic_pad_function(
                    parent,
                    || false,
                    |alphasplit| alphasplit.src_query(pad, query),
                )
            })
            .build();

        let templ = klass.pad_template("alpha").unwrap();
        let alphapad = gst::Pad::builder_from_template(&templ)
            .event_function(|pad, parent, event| {
                Alphasplit::catch_panic_pad_function(
                    parent,
                    || false,
                    |alphasplit| alphasplit.src_event(pad, event),
                )
            })
            .query_function(|pad, parent, query| {
                Alphasplit::catch_panic_pad_function(
                    parent,
                    || false,
                    |alphasplit| alphasplit.src_query(pad, query),
                )
            })
            .build();

        Self { srcpad, alphapad, sinkpad }
    }
}

impl ObjectImpl for Alphasplit {
    fn constructed(&self) {
        self.parent_constructed();

        let obj = self.obj();
        obj.add_pad(&self.sinkpad).unwrap();
        obj.add_pad(&self.srcpad).unwrap();
        obj.add_pad(&self.alphapad).unwrap();
    }
}

impl GstObjectImpl for Alphasplit {}

impl ElementImpl for Alphasplit {
    fn metadata() -> Option<&'static gst::subclass::ElementMetadata> {
        static ELEMENT_METADATA: LazyLock<gst::subclass::ElementMetadata> = LazyLock::new(|| {
            gst::subclass::ElementMetadata::new(
                "Alphasplit",
                "Video",
                "Split alpha and color channels into separate streams",
                "Diego Nieto <dnieto@fluendo.com>",
            )
        });

        Some(&*ELEMENT_METADATA)
    }

    fn pad_templates() -> &'static [gst::PadTemplate] {
        static PAD_TEMPLATES: LazyLock<Vec<gst::PadTemplate>> = LazyLock::new(|| {
            let src_caps = gst_video::VideoCapsBuilder::new()
                .format(VideoFormat::I42010le)
                .build();
            let src_pad_template = gst::PadTemplate::new(
                "src",
                gst::PadDirection::Src,
                gst::PadPresence::Always,
                &src_caps,
            )
            .unwrap();

            let alpha_caps = gst_video::VideoCapsBuilder::new()
                .format(VideoFormat::Gray10Le32)
                .build();
            let alpha_pad_template = gst::PadTemplate::new(
                "alpha",
                gst::PadDirection::Src,
                gst::PadPresence::Always,
                &alpha_caps,
            )
            .unwrap();

            let sink_caps = gst_video::VideoCapsBuilder::new()
                .format(VideoFormat::A42010le)
                .build();
            let sink_pad_template = gst::PadTemplate::new(
                "sink",
                gst::PadDirection::Sink,
                gst::PadPresence::Always,
                &sink_caps,
            )
            .unwrap();

            vec![src_pad_template, alpha_pad_template, sink_pad_template]
        });

        PAD_TEMPLATES.as_ref()
    }

    fn change_state(
        &self,
        transition: gst::StateChange,
    ) -> Result<gst::StateChangeSuccess, gst::StateChangeError> {
        gst::trace!(CAT, imp = self, "Changing state {:?}", transition);

        self.parent_change_state(transition)
    }
}

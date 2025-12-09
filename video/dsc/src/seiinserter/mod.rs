use gst::glib;
use gst::prelude::*;

mod dsc_meta;
mod ffi;
mod imp;

glib::wrapper! {
    pub struct DscSeiInserter(ObjectSubclass<imp::DscSeiInserter>) @extends gst_base::BaseTransform, gst::Element, gst::Object;
}

pub fn register(plugin: &gst::Plugin) -> Result<(), glib::BoolError> {
    gst::Element::register(
        Some(plugin),
        "dscseiinserter",
        gst::Rank::NONE,
        DscSeiInserter::static_type(),
    )
}

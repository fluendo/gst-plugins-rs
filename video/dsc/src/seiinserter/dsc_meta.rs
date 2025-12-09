use gst::ffi::*;
use glib::ffi::*;

#[repr(C)]
pub struct GstVideoDigitalSignedContentInitializationMeta {
    pub meta: GstMeta,
    pub hash_method_type: u8,
    pub key_source_uri: *mut std::os::raw::c_char,
    pub num_verification_substreams: u32,
    pub key_retrieval_mode_idc: u32,
    pub use_key_register_idx_flag: gboolean,
    pub key_register_idx: u32,
    pub content_uuid_present_flag: gboolean,
    pub content_uuid: [u8; 16],
}

#[repr(C)]
pub struct GstVideoDigitalSignedContentSelectionMeta {
    pub meta: GstMeta,
    pub verification_substream_id: u32,
}

#[repr(C)]
pub struct GstVideoDigitalSignedContentVerificationMeta {
    pub meta: GstMeta,
    pub verification_substream_id: u32,
    pub signature_length_in_octets: u32,
    pub signature: *mut GArray,
}

extern "C" {
    pub fn gst_video_digital_signed_content_initialization_meta_api_get_type() -> GType;
    pub fn gst_video_digital_signed_content_selection_meta_api_get_type() -> GType;
    pub fn gst_video_digital_signed_content_verification_meta_api_get_type() -> GType;
}

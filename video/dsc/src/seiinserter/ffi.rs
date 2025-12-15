use gst::ffi::*;
use std::mem::ManuallyDrop;

// H.264 SEI types
pub const GST_H264_SEI_DIGITALLY_SIGNED_CONTENT_INITIALIZATION: u32 = 220;
pub const GST_H264_SEI_DIGITALLY_SIGNED_CONTENT_SELECTION: u32 = 221;
pub const GST_H264_SEI_DIGITALLY_SIGNED_CONTENT_VERIFICATION: u32 = 222;

// H.265 SEI types
pub const GST_H265_SEI_DIGITALLY_SIGNED_CONTENT_INITIALIZATION: u32 = 220;
pub const GST_H265_SEI_DIGITALLY_SIGNED_CONTENT_SELECTION: u32 = 221;
pub const GST_H265_SEI_DIGITALLY_SIGNED_CONTENT_VERIFICATION: u32 = 222;

#[repr(C)]
pub struct GstH264DigitallySignedContentInitialization {
    pub hash_method_type: u8,
    pub key_source_uri: *mut std::os::raw::c_char,
    pub num_verification_substreams_minus1: u32,
    pub key_retrieval_mode_idc: u32,
    pub use_key_register_idx_flag: u8,
    pub key_register_idx: u32,
    pub content_uuid_present_flag: u8,
    pub content_uuid: [u8; 16],
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct GstH264DigitallySignedContentSelection {
    pub verification_substream_id: u32,
}

#[repr(C)]
pub struct GstH264DigitallySignedContentVerification {
    pub verification_substream_id: u32,
    pub signature_length_in_octets_minus1: u32,
    pub signature: *mut u8,
}

#[repr(C)]
pub union GstH264SEIPayload {
    pub digitally_signed_content_initialization: ManuallyDrop<GstH264DigitallySignedContentInitialization>,
    pub digitally_signed_content_selection: GstH264DigitallySignedContentSelection,
    pub digitally_signed_content_verification: ManuallyDrop<GstH264DigitallySignedContentVerification>,
}

#[repr(C)]
pub struct GstH264SEIMessage {
    pub payload_type: u32,
    pub payload: GstH264SEIPayload,
}

// Similar structures for H.265
#[repr(C)]
pub struct GstH265DigitallySignedContentInitialization {
    pub hash_method_type: u8,
    pub key_source_uri: *mut std::os::raw::c_char,
    pub num_verification_substreams_minus1: u32,
    pub key_retrieval_mode_idc: u32,
    pub use_key_register_idx_flag: u8,
    pub key_register_idx: u32,
    pub content_uuid_present_flag: u8,
    pub content_uuid: [u8; 16],
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct GstH265DigitallySignedContentSelection {
    pub verification_substream_id: u32,
}

#[repr(C)]
pub struct GstH265DigitallySignedContentVerification {
    pub verification_substream_id: u32,
    pub signature_length_in_octets_minus1: u32,
    pub signature: *mut u8,
}

#[repr(C)]
pub union GstH265SEIPayload {
    pub digitally_signed_content_initialization: ManuallyDrop<GstH265DigitallySignedContentInitialization>,
    pub digitally_signed_content_selection: GstH265DigitallySignedContentSelection,
    pub digitally_signed_content_verification: ManuallyDrop<GstH265DigitallySignedContentVerification>,
}

#[repr(C)]
pub struct GstH265SEIMessage {
    pub payload_type: u32,
    pub payload: GstH265SEIPayload,
}

// Add parser structures and functions
#[repr(C)]
pub struct GstH264NalParser {
    _private: [u8; 0],
}

#[repr(C)]
pub struct GstH265NalParser {
    _private: [u8; 0],
}

extern "C" {
    // H.264 parser functions
    pub fn gst_h264_nal_parser_new() -> *mut GstH264NalParser;
    pub fn gst_h264_nal_parser_free(parser: *mut GstH264NalParser);

    // H.264 functions
    pub fn gst_h264_create_sei_memory(
        start_code_prefix_length: u8,
        messages: *mut glib::ffi::GArray,
    ) -> *mut GstMemory;

    pub fn gst_h264_parser_insert_sei(
        parser: *mut std::ffi::c_void,
        au: *mut GstBuffer,
        sei: *mut GstMemory,
    ) -> *mut GstBuffer;

    // H.265 parser functions
    pub fn gst_h265_parser_new() -> *mut GstH265NalParser;
    pub fn gst_h265_parser_free(parser: *mut GstH265NalParser);

    // H.265 functions
    pub fn gst_h265_create_sei_memory(
        layer_id: u8,
        temporal_id_plus1: u8,
        start_code_prefix_length: u8,
        messages: *mut glib::ffi::GArray,
    ) -> *mut GstMemory;

    pub fn gst_h265_parser_insert_sei(
        parser: *mut std::ffi::c_void,
        au: *mut GstBuffer,
        sei: *mut GstMemory,
    ) -> *mut GstBuffer;
}

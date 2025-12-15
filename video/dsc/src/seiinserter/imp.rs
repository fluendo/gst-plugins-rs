use gst::glib;
use gst::subclass::prelude::*;
use gst_base::subclass::prelude::*;

use std::ffi::CString;
use std::sync::{LazyLock, Mutex};
use std::mem::ManuallyDrop;

use crate::seiinserter::ffi;
use crate::ffidscmeta as dsc_meta;

static CAT: LazyLock<gst::DebugCategory> = LazyLock::new(|| {
    gst::DebugCategory::new(
        "dscseiinserter",
        gst::DebugColorFlags::empty(),
        Some("GstDscSeiInserter"),
    )
});

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodecType {
    H264,
    H265,
    H266,
}

// Wrapper to make raw pointers Send + Sync
struct ParserPtr(*mut std::ffi::c_void);
unsafe impl Send for ParserPtr {}
unsafe impl Sync for ParserPtr {}

#[derive(Default)]
pub struct DscSeiInserter {
    codec_type: Mutex<Option<CodecType>>,
    h264_parser: Mutex<Option<ParserPtr>>,
    h265_parser: Mutex<Option<ParserPtr>>,
}

#[glib::object_subclass]
impl ObjectSubclass for DscSeiInserter {
    const NAME: &'static str = "DscSeiInserter";
    type Type = super::DscSeiInserter;
    type ParentType = gst_base::BaseTransform;
}

impl ObjectImpl for DscSeiInserter {
    fn constructed(&self) {
        self.parent_constructed();
    }
}

impl GstObjectImpl for DscSeiInserter {}

impl ElementImpl for DscSeiInserter {
    fn metadata() -> Option<&'static gst::subclass::ElementMetadata> {
        static ELEMENT_METADATA: LazyLock<gst::subclass::ElementMetadata> = LazyLock::new(|| {
            gst::subclass::ElementMetadata::new(
                "DSC SEI Inserter",
                "Generic",
                "Converts DSC signature metadata to SEI messages in H.264/H.265/H.266 bitstreams",
                "Diego Nieto <dnieto@fluendo.com>",
            )
        });

        Some(&*ELEMENT_METADATA)
    }

    fn pad_templates() -> &'static [gst::PadTemplate] {
        static TEMPLATES: LazyLock<Vec<gst::PadTemplate>> = LazyLock::new(|| {
            let caps = gst::Caps::builder_full()
                .structure(
                    gst::Structure::builder("video/x-h264")
                        .field("stream-format", "byte-stream")
                        .field("alignment", "au")
                        .build(),
                )
                .structure(
                    gst::Structure::builder("video/x-h265")
                        .field("stream-format", "byte-stream")
                        .field("alignment", "au")
                        .build(),
                )
                .structure(
                    gst::Structure::builder("video/x-h266")
                        .field("stream-format", "byte-stream")
                        .field("alignment", "au")
                        .build(),
                )
                .build();
            vec![
                gst::PadTemplate::new(
                    "sink",
                    gst::PadDirection::Sink,
                    gst::PadPresence::Always,
                    &caps,
                ).unwrap(),
                gst::PadTemplate::new(
                    "src",
                    gst::PadDirection::Src,
                    gst::PadPresence::Always,
                    &caps,
                ).unwrap(),
            ]
        });
        TEMPLATES.as_ref()
    }
}

impl BaseTransformImpl for DscSeiInserter {
    const MODE: gst_base::subclass::BaseTransformMode = gst_base::subclass::BaseTransformMode::NeverInPlace;
    const PASSTHROUGH_ON_SAME_CAPS: bool = false;
    const TRANSFORM_IP_ON_PASSTHROUGH: bool = false;

    fn set_caps(&self, incaps: &gst::Caps, _outcaps: &gst::Caps) -> Result<(), gst::LoggableError> {
        let structure = incaps.structure(0).unwrap();
        let codec_type = match structure.name().as_str() {
            "video/x-h264" => {
                // Initialize H.264 parser if needed
                let parser = unsafe { ffi::gst_h264_nal_parser_new() };
                if parser.is_null() {
                    return Err(gst::loggable_error!(
                        CAT,
                        "Failed to create H.264 parser"
                    ));
                }
                *self.h264_parser.lock().unwrap() = Some(ParserPtr(parser as *mut std::ffi::c_void));
                CodecType::H264
            },
            "video/x-h265" => {
                // Initialize H.265 parser if needed
                let parser = unsafe { ffi::gst_h265_parser_new() };
                if parser.is_null() {
                    return Err(gst::loggable_error!(
                        CAT,
                        "Failed to create H.265 parser"
                    ));
                }
                *self.h265_parser.lock().unwrap() = Some(ParserPtr(parser as *mut std::ffi::c_void));
                CodecType::H265
            },
            "video/x-h266" => CodecType::H266,
            _ => {
                return Err(gst::loggable_error!(
                    CAT,
                    "Unsupported caps: {}",
                    incaps
                ));
            }
        };

        *self.codec_type.lock().unwrap() = Some(codec_type);

        if codec_type == CodecType::H266 {
            gst::warning!(CAT, imp = self,
                "H.266 DSC SEI insertion not yet supported, passing through");
        }

        Ok(())
    }

    fn transform(
        &self,
        inbuf: &gst::Buffer,
        outbuf: &mut gst::BufferRef,
    ) -> Result<gst::FlowSuccess, gst::FlowError> {
        let codec_type = self.codec_type.lock().unwrap().ok_or(gst::FlowError::NotNegotiated)?;

        // Check for DSC metadata using our custom bindings
        let init_meta = unsafe {
            gst::ffi::gst_buffer_get_meta(
                inbuf.as_mut_ptr(),
                dsc_meta::gst_video_digital_signed_content_initialization_meta_api_get_type(),
            ) as *mut dsc_meta::GstVideoDigitalSignedContentInitializationMeta
        };

        let selection_meta = unsafe {
            gst::ffi::gst_buffer_get_meta(
                inbuf.as_mut_ptr(),
                dsc_meta::gst_video_digital_signed_content_selection_meta_api_get_type(),
            ) as *mut dsc_meta::GstVideoDigitalSignedContentSelectionMeta
        };

        let verification_meta = unsafe {
            gst::ffi::gst_buffer_get_meta(
                inbuf.as_mut_ptr(),
                dsc_meta::gst_video_digital_signed_content_verification_meta_api_get_type(),
            ) as *mut dsc_meta::GstVideoDigitalSignedContentVerificationMeta
        };

        let has_dsc_meta = !init_meta.is_null() || !selection_meta.is_null() || !verification_meta.is_null();

        if !has_dsc_meta {
            // No DSC metadata, just copy buffer
            let inmap = inbuf.map_readable().map_err(|_| gst::FlowError::Error)?;
            outbuf.set_size(inmap.len());
            let mut outmap = outbuf.map_writable().map_err(|_| gst::FlowError::Error)?;
            outmap.copy_from_slice(&inmap);
            return Ok(gst::FlowSuccess::Ok);
        }

        // H.266 not supported yet
        if codec_type == CodecType::H266 {
            gst::debug!(CAT, imp = self, "H.266 passthrough - API not available yet");
            let inmap = inbuf.map_readable().map_err(|_| gst::FlowError::Error)?;
            outbuf.set_size(inmap.len());
            let mut outmap = outbuf.map_writable().map_err(|_| gst::FlowError::Error)?;
            outmap.copy_from_slice(&inmap);
            return Ok(gst::FlowSuccess::Ok);
        }

        gst::debug!(CAT, imp = self, "Found DSC metadata, creating SEI messages");

        // Create SEI messages array
        let messages = unsafe {
            glib::ffi::g_array_new(
                0,
                0,
                match codec_type {
                    CodecType::H264 => std::mem::size_of::<ffi::GstH264SEIMessage>(),
                    CodecType::H265 => std::mem::size_of::<ffi::GstH265SEIMessage>(),
                    _ => unreachable!(),
                } as u32,
            )
        };

        if messages.is_null() {
            gst::error!(CAT, imp = self, "Failed to create SEI messages array");
            return Err(gst::FlowError::Error);
        }

        // Add DSC SEI messages based on metadata
        match codec_type {
            CodecType::H264 => {
                self.add_h264_dsc_sei_messages(messages, init_meta, selection_meta, verification_meta)?;
            }
            CodecType::H265 => {
                self.add_h265_dsc_sei_messages(messages, init_meta, selection_meta, verification_meta)?;
            }
            _ => unreachable!(),
        }

        // Create SEI memory
        let sei_memory = match codec_type {
            CodecType::H264 => unsafe {
                ffi::gst_h264_create_sei_memory(4, messages)
            },
            CodecType::H265 => unsafe {
                ffi::gst_h265_create_sei_memory(0, 1, 4, messages)
            },
            _ => unreachable!(),
        };

        // Cleanup messages array
        unsafe {
            glib::ffi::g_array_free(messages, 1);
        }

        if sei_memory.is_null() {
            gst::error!(CAT, imp = self, "Failed to create SEI memory");
            return Err(gst::FlowError::Error);
        }

        // Use proper parser insertion instead of concatenation
        let sei_mem = unsafe { gst::Memory::from_glib_full(sei_memory) };
        let input_buffer = inbuf.copy();

        let output_buffer = match codec_type {
            CodecType::H264 => {
                let parser = self.h264_parser.lock().unwrap();
                if let Some(parser_ptr) = parser.as_ref() {
                    unsafe {
                        let result_buf = ffi::gst_h264_parser_insert_sei(
                            parser_ptr.0,
                            input_buffer.as_mut_ptr(),
                            sei_mem.as_ptr() as *mut gst::ffi::GstMemory,
                        );
                        if result_buf.is_null() {
                            gst::error!(CAT, imp = self, "Failed to insert H.264 SEI");
                            return Err(gst::FlowError::Error);
                        }
                        gst::Buffer::from_glib_full(result_buf)
                    }
                } else {
                    gst::error!(CAT, imp = self, "H.264 parser not initialized");
                    return Err(gst::FlowError::Error);
                }
            },
            CodecType::H265 => {
                let parser = self.h265_parser.lock().unwrap();
                if let Some(parser_ptr) = parser.as_ref() {
                    unsafe {
                        let result_buf = ffi::gst_h265_parser_insert_sei(
                            parser_ptr.0,
                            input_buffer.as_mut_ptr(),
                            sei_mem.as_ptr() as *mut gst::ffi::GstMemory,
                        );
                        if result_buf.is_null() {
                            gst::error!(CAT, imp = self, "Failed to insert H.265 SEI");
                            return Err(gst::FlowError::Error);
                        }
                        gst::Buffer::from_glib_full(result_buf)
                    }
                } else {
                    gst::error!(CAT, imp = self, "H.265 parser not initialized");
                    return Err(gst::FlowError::Error);
                }
            },
            _ => unreachable!(),
        };

        // Copy the result to output buffer
        let outmap_result = output_buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
        outbuf.set_size(outmap_result.len());
        let mut outmap = outbuf.map_writable().map_err(|_| gst::FlowError::Error)?;
        outmap.copy_from_slice(&outmap_result);

        Ok(gst::FlowSuccess::Ok)
    }

    fn stop(&self) -> Result<(), gst::ErrorMessage> {
        // Cleanup parsers
        unsafe {
            let h264_parser = self.h264_parser.lock().unwrap();
            if let Some(parser_ptr) = h264_parser.as_ref() {
                ffi::gst_h264_nal_parser_free(parser_ptr.0 as *mut ffi::GstH264NalParser);
            }

            let h265_parser = self.h265_parser.lock().unwrap();
            if let Some(parser_ptr) = h265_parser.as_ref() {
                ffi::gst_h265_parser_free(parser_ptr.0 as *mut ffi::GstH265NalParser);
            }
        }

        self.parent_stop()
    }
}

impl DscSeiInserter {
    fn add_h264_dsc_sei_messages(
        &self,
        messages: *mut glib::ffi::GArray,
        init_meta: *mut dsc_meta::GstVideoDigitalSignedContentInitializationMeta,
        selection_meta: *mut dsc_meta::GstVideoDigitalSignedContentSelectionMeta,
        verification_meta: *mut dsc_meta::GstVideoDigitalSignedContentVerificationMeta,
    ) -> Result<(), gst::FlowError> {
        unsafe {
            // Add initialization SEI if present
            if !init_meta.is_null() {
                let meta = &*init_meta;
                let mut sei_msg: ffi::GstH264SEIMessage = std::mem::zeroed();
                sei_msg.payload_type = ffi::GST_H264_SEI_DIGITALLY_SIGNED_CONTENT_INITIALIZATION;

                let key_uri = CString::new(
                    std::ffi::CStr::from_ptr(meta.key_source_uri).to_bytes()
                ).unwrap();

                sei_msg.payload.digitally_signed_content_initialization = ManuallyDrop::new(
                    ffi::GstH264DigitallySignedContentInitialization {
                        hash_method_type: meta.hash_method_type,
                        key_source_uri: key_uri.into_raw(),
                        num_verification_substreams_minus1: meta.num_verification_substreams - 1,
                        key_retrieval_mode_idc: meta.key_retrieval_mode_idc,
                        use_key_register_idx_flag: if meta.use_key_register_idx_flag != 0 { 1 } else { 0 },
                        key_register_idx: meta.key_register_idx,
                        content_uuid_present_flag: if meta.content_uuid_present_flag != 0 { 1 } else { 0 },
                        content_uuid: meta.content_uuid,
                    }
                );

                glib::ffi::g_array_append_vals(messages, &sei_msg as *const _ as *const _, 1);
            }

            // Add selection SEI if present
            if !selection_meta.is_null() {
                let meta = &*selection_meta;
                let mut sei_msg: ffi::GstH264SEIMessage = std::mem::zeroed();
                sei_msg.payload_type = ffi::GST_H264_SEI_DIGITALLY_SIGNED_CONTENT_SELECTION;

                sei_msg.payload.digitally_signed_content_selection = ffi::GstH264DigitallySignedContentSelection {
                    verification_substream_id: meta.verification_substream_id,
                };

                glib::ffi::g_array_append_vals(messages, &sei_msg as *const _ as *const _, 1);
            }

            // Add verification SEI if present
            if !verification_meta.is_null() {
                let meta = &*verification_meta;
                let mut sei_msg: ffi::GstH264SEIMessage = std::mem::zeroed();
                sei_msg.payload_type = ffi::GST_H264_SEI_DIGITALLY_SIGNED_CONTENT_VERIFICATION;

                let sig_len = meta.signature_length_in_octets;
                let signature = glib::ffi::g_malloc(sig_len as usize) as *mut u8;
                std::ptr::copy_nonoverlapping(
                    (*meta.signature).data as *const u8,
                    signature,
                    sig_len as usize
                );

                sei_msg.payload.digitally_signed_content_verification = ManuallyDrop::new(
                    ffi::GstH264DigitallySignedContentVerification {
                        verification_substream_id: meta.verification_substream_id,
                        signature_length_in_octets_minus1: sig_len - 1,
                        signature,
                    }
                );

                glib::ffi::g_array_append_vals(messages, &sei_msg as *const _ as *const _, 1);
            }
        }

        Ok(())
    }

    fn add_h265_dsc_sei_messages(
        &self,
        messages: *mut glib::ffi::GArray,
        init_meta: *mut dsc_meta::GstVideoDigitalSignedContentInitializationMeta,
        selection_meta: *mut dsc_meta::GstVideoDigitalSignedContentSelectionMeta,
        verification_meta: *mut dsc_meta::GstVideoDigitalSignedContentVerificationMeta,
    ) -> Result<(), gst::FlowError> {
        // Similar implementation to H.264 but using GstH265SEIMessage
        unsafe {
            // Add initialization SEI if present
            if !init_meta.is_null() {
                let meta = &*init_meta;
                let mut sei_msg: ffi::GstH265SEIMessage = std::mem::zeroed();
                sei_msg.payload_type = ffi::GST_H265_SEI_DIGITALLY_SIGNED_CONTENT_INITIALIZATION;

                let key_uri = CString::new(
                    std::ffi::CStr::from_ptr(meta.key_source_uri).to_bytes()
                ).unwrap();

                sei_msg.payload.digitally_signed_content_initialization = ManuallyDrop::new(
                    ffi::GstH265DigitallySignedContentInitialization {
                        hash_method_type: meta.hash_method_type,
                        key_source_uri: key_uri.into_raw(),
                        num_verification_substreams_minus1: meta.num_verification_substreams - 1,
                        key_retrieval_mode_idc: meta.key_retrieval_mode_idc,
                        use_key_register_idx_flag: if meta.use_key_register_idx_flag != 0 { 1 } else { 0 },
                        key_register_idx: meta.key_register_idx,
                        content_uuid_present_flag: if meta.content_uuid_present_flag != 0 { 1 } else { 0 },
                        content_uuid: meta.content_uuid,
                    }
                );

                glib::ffi::g_array_append_vals(messages, &sei_msg as *const _ as *const _, 1);
            }

            // Add selection SEI if present
            if !selection_meta.is_null() {
                let meta = &*selection_meta;
                let mut sei_msg: ffi::GstH265SEIMessage = std::mem::zeroed();
                sei_msg.payload_type = ffi::GST_H265_SEI_DIGITALLY_SIGNED_CONTENT_SELECTION;

                sei_msg.payload.digitally_signed_content_selection = ffi::GstH265DigitallySignedContentSelection {
                    verification_substream_id: meta.verification_substream_id,
                };

                glib::ffi::g_array_append_vals(messages, &sei_msg as *const _ as *const _, 1);
            }

            // Add verification SEI if present
            if !verification_meta.is_null() {
                let meta = &*verification_meta;
                let mut sei_msg: ffi::GstH265SEIMessage = std::mem::zeroed();
                sei_msg.payload_type = ffi::GST_H265_SEI_DIGITALLY_SIGNED_CONTENT_VERIFICATION;

                let sig_len = meta.signature_length_in_octets;
                let signature = glib::ffi::g_malloc(sig_len as usize) as *mut u8;
                std::ptr::copy_nonoverlapping(
                    (*meta.signature).data as *const u8,
                    signature,
                    sig_len as usize
                );

                sei_msg.payload.digitally_signed_content_verification = ManuallyDrop::new(
                    ffi::GstH265DigitallySignedContentVerification {
                        verification_substream_id: meta.verification_substream_id,
                        signature_length_in_octets_minus1: sig_len - 1,
                        signature,
                    }
                );

                glib::ffi::g_array_append_vals(messages, &sei_msg as *const _ as *const _, 1);
            }
        }

        Ok(())
    }
}

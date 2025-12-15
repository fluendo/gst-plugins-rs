// Copyright (C) 2025, Fluendo S.A.
//      Author: Diego Nieto <dnieto@fluendo.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v2.0.
// If a copy of the MPL was not distributed with this file, You can obtain one at
// <https://mozilla.org/MPL/2.0/>.
//
// SPDX-License-Identifier: MPL-2.0

use gst::prelude::*;

fn init() {
    use std::sync::Once;
    static INIT: Once = Once::new();

    INIT.call_once(|| {
        gst::init().unwrap();
        gstdsc::plugin_register_static().expect("dsc test");
    });
}

#[test]
fn test_add_initialization_meta() {
    init();

    let mut buffer = gst::Buffer::new();
    let buffer_ref = buffer.get_mut().unwrap();

    let content_uuid = [0u8; 16];
    let key_uri = std::ffi::CString::new("http://example.com/key.pem").unwrap();

    unsafe {
        let meta = gstdsc::ffidscmeta::gst_buffer_add_video_digital_signed_content_initialization_meta(
            buffer_ref.as_mut_ptr(),
            2, // SHA-256
            key_uri.as_ptr(),
            1, // num_verification_substreams
            0, // key_retrieval_mode_idc
            0, // use_key_register_idx_flag
            0, // key_register_idx
            0, // content_uuid_present_flag
            content_uuid.as_ptr(),
        );

        assert!(!meta.is_null(), "Failed to add initialization meta");

        let meta_ref = &*meta;
        assert_eq!(meta_ref.hash_method_type, 2);
        assert_eq!(meta_ref.num_verification_substreams, 1);
    }
}

#[test]
fn test_add_selection_meta() {
    init();

    let mut buffer = gst::Buffer::new();
    let buffer_ref = buffer.get_mut().unwrap();

    unsafe {
        let meta = gstdsc::ffidscmeta::gst_buffer_add_video_digital_signed_content_selection_meta(
            buffer_ref.as_mut_ptr(),
            0, // verification_substream_id
        );

        assert!(!meta.is_null(), "Failed to add selection meta");

        let meta_ref = &*meta;
        assert_eq!(meta_ref.verification_substream_id, 0);
    }
}

#[test]
fn test_add_verification_meta() {
    init();

    let mut buffer = gst::Buffer::new();
    let buffer_ref = buffer.get_mut().unwrap();

    let signature = vec![0xAB; 256]; // Dummy 256-byte signature

    unsafe {
        let meta = gstdsc::ffidscmeta::gst_buffer_add_video_digital_signed_content_verification_meta(
            buffer_ref.as_mut_ptr(),
            0, // verification_substream_id
            signature.as_ptr(),
            signature.len() as u32,
        );

        assert!(!meta.is_null(), "Failed to add verification meta");

        let meta_ref = &*meta;
        assert_eq!(meta_ref.verification_substream_id, 0);
        assert_eq!(meta_ref.signature_length_in_octets, 256);
        assert!(!meta_ref.signature.is_null());
    }
}

#[test]
fn test_add_all_three_metas() {
    init();

    let mut buffer = gst::Buffer::new();
    let buffer_ref = buffer.get_mut().unwrap();

    let content_uuid = [0u8; 16];
    let key_uri = std::ffi::CString::new("http://example.com/key.pem").unwrap();
    let signature = vec![0xCD; 512];

    unsafe {
        // Add initialization meta
        let init_meta = gstdsc::ffidscmeta::gst_buffer_add_video_digital_signed_content_initialization_meta(
            buffer_ref.as_mut_ptr(),
            2,
            key_uri.as_ptr(),
            1,
            0,
            0,
            0,
            0,
            content_uuid.as_ptr(),
        );
        assert!(!init_meta.is_null(), "Failed to add initialization meta");

        // Add selection meta
        let selection_meta = gstdsc::ffidscmeta::gst_buffer_add_video_digital_signed_content_selection_meta(
            buffer_ref.as_mut_ptr(),
            0,
        );
        assert!(!selection_meta.is_null(), "Failed to add selection meta");

        // Add verification meta
        let verification_meta = gstdsc::ffidscmeta::gst_buffer_add_video_digital_signed_content_verification_meta(
            buffer_ref.as_mut_ptr(),
            0,
            signature.as_ptr(),
            signature.len() as u32,
        );
        assert!(!verification_meta.is_null(), "Failed to add verification meta");

        // Verify all metas are present
        let init_found = gst::ffi::gst_buffer_get_meta(
            buffer_ref.as_mut_ptr(),
            gstdsc::ffidscmeta::gst_video_digital_signed_content_initialization_meta_api_get_type(),
        );
        assert!(!init_found.is_null(), "Initialization meta not found after adding");

        let selection_found = gst::ffi::gst_buffer_get_meta(
            buffer_ref.as_mut_ptr(),
            gstdsc::ffidscmeta::gst_video_digital_signed_content_selection_meta_api_get_type(),
        );
        assert!(!selection_found.is_null(), "Selection meta not found after adding");

        let verification_found = gst::ffi::gst_buffer_get_meta(
            buffer_ref.as_mut_ptr(),
            gstdsc::ffidscmeta::gst_video_digital_signed_content_verification_meta_api_get_type(),
        );
        assert!(!verification_found.is_null(), "Verification meta not found after adding");
    }
}

#[test]
fn test_meta_survives_buffer_copy() {
    init();

    let mut buffer = gst::Buffer::new();
    let buffer_ref = buffer.get_mut().unwrap();

    let signature = vec![0xEF; 128];

    unsafe {
        gstdsc::ffidscmeta::gst_buffer_add_video_digital_signed_content_verification_meta(
            buffer_ref.as_mut_ptr(),
            0,
            signature.as_ptr(),
            signature.len() as u32,
        );
    }

    // Copy the buffer
    let copied_buffer = buffer.copy();

    // Verify meta is in the copied buffer
    unsafe {
        let meta = gst::ffi::gst_buffer_get_meta(
            copied_buffer.as_ptr() as *mut _,
            gstdsc::ffidscmeta::gst_video_digital_signed_content_verification_meta_api_get_type(),
        ) as *mut gstdsc::ffidscmeta::GstVideoDigitalSignedContentVerificationMeta;

        assert!(!meta.is_null(), "Meta not found in copied buffer");
        assert_eq!((*meta).signature_length_in_octets, 128);
    }
}

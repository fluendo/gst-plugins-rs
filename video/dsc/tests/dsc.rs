// Copyright (C) 2025, Fluendo S.A.
//      Author: Diego Nieto <dnieto@fluendo.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v2.0.
// If a copy of the MPL was not distributed with this file, You can obtain one at
// <https://mozilla.org/MPL/2.0/>.
//
// SPDX-License-Identifier: MPL-2.0

use gst::prelude::*;
use openssl::pkey::PKey;
use openssl::rsa::Rsa;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static KEY_COUNTER: AtomicU64 = AtomicU64::new(0);

fn init() {
    use std::sync::Once;
    static INIT: Once = Once::new();

    INIT.call_once(|| {
        gst::init().unwrap();
        gstdsc::plugin_register_static().expect("dsc test");
    });
}

fn create_test_keys() -> (PathBuf, PathBuf) {
    let temp_dir = std::env::temp_dir();
    let unique_id = KEY_COUNTER.fetch_add(1, Ordering::SeqCst);
    let private_key_path = temp_dir.join(format!("test_private_key_{}.pem", unique_id));
    let public_key_path = temp_dir.join(format!("test_public_key_{}.pem", unique_id));

    // Generate RSA key pair
    let rsa = Rsa::generate(2048).unwrap();
    let pkey = PKey::from_rsa(rsa).unwrap();

    // Save private key
    let private_pem = pkey.private_key_to_pem_pkcs8().unwrap();
    fs::write(&private_key_path, private_pem).unwrap();

    // Save public key
    let public_pem = pkey.public_key_to_pem().unwrap();
    fs::write(&public_key_path, public_pem).unwrap();

    (private_key_path, public_key_path)
}

fn cleanup_test_keys(private_key_path: &PathBuf, public_key_path: &PathBuf) {
    let _ = fs::remove_file(private_key_path);
    let _ = fs::remove_file(public_key_path);
}

#[test]
fn test_signer_verifier_sha256() {
    test_signer_verifier_with_hash("sha256");
}

#[test]
fn test_signer_verifier_sha384() {
    test_signer_verifier_with_hash("sha384");
}

#[test]
fn test_signer_verifier_sha512() {
    test_signer_verifier_with_hash("sha512");
}

#[test]
fn test_signer_verifier_sha1() {
    test_signer_verifier_with_hash("sha1");
}

#[test]
fn test_signer_verifier_sha224() {
    test_signer_verifier_with_hash("sha224");
}

fn test_signer_verifier_with_hash(hash_method: &str) {
    init();

    let (private_key_path, public_key_path) = create_test_keys();
    let key_store_path = public_key_path.parent().unwrap().to_str().unwrap();
    let public_key_filename = public_key_path.file_name().unwrap().to_str().unwrap();

    // Create signer harness
    let mut h_signer = gst_check::Harness::new("dscsigner");
    h_signer.play();

    // Configure signer
    let signer = h_signer.element().unwrap();
    signer.set_property("hash-method", hash_method);
    signer.set_property(
        "private-key-path",
        private_key_path.to_str().unwrap(),
    );
    signer.set_property("public-key-uri", public_key_filename);

    let caps = gst::Caps::builder("video/x-h264")
        .field("stream-format", "byte-stream")
        .field("alignment", "au")
        .build();
    h_signer.set_src_caps(caps.clone());

    // Create test buffers with H.264-like NAL units
    // We need at least 2 I-frames to get a signature (GOP-based signing)
    let num_buffers = 10;

    for i in 0..num_buffers {
        let is_i_frame = i % 5 == 0; // I-frame every 5 frames
        let buffer = create_test_h264_buffer(i, is_i_frame);
        h_signer.push(buffer).unwrap();
    }

    h_signer.push_event(gst::event::Eos::new());

    // Create verifier harness
    let mut h_verifier = gst_check::Harness::new("dscverifier");
    h_verifier.play();

    let verifier = h_verifier.element().unwrap();
    verifier.set_property("key-store-path", key_store_path);

    h_verifier.set_src_caps(caps);

    let mut signature_found = false;
    for _ in 0..num_buffers {
        let signed_buffer = h_signer.pull().unwrap();
        
        // Check if this buffer has signature meta
        if signed_buffer.meta::<gstdsc::signaturemeta::SignatureMeta>().is_some() {
            signature_found = true;
        }

        h_verifier.push(signed_buffer).unwrap();
    }

    h_verifier.push_event(gst::event::Eos::new());

    // Pull verified buffers
    for _ in 0..num_buffers {
        let _verified_buffer = h_verifier.pull().unwrap();
    }

    // With GOP-based signing, we should have at least one signature
    // (attached to the second I-frame for the first GOP)
    assert!(signature_found, "Expected at least one buffer with signature meta");

    cleanup_test_keys(&private_key_path, &public_key_path);
}

fn create_test_h264_buffer(index: usize, is_i_frame: bool) -> gst::Buffer {
    // Create a simple H.264 NAL unit with start code
    let nal_type = if is_i_frame { 0x65 } else { 0x41 }; // IDR slice or non-IDR slice
    
    let mut data = vec![
        0x00, 0x00, 0x00, 0x01, // Start code
        nal_type,               // NAL header
    ];
    
    // Add some payload data
    for j in 0..100 {
        data.push(((index * 100 + j) % 256) as u8);
    }

    let mut buffer = gst::Buffer::from_slice(data);
    {
        let buffer_ref = buffer.get_mut().unwrap();
        if !is_i_frame {
            buffer_ref.set_flags(gst::BufferFlags::DELTA_UNIT);
        }
    }
    buffer
}

#[test]
fn test_signer_without_key() {
    init();

    let mut h = gst_check::Harness::new("dscsigner");
    h.play();

    let caps = gst::Caps::builder("video/x-h264")
        .field("stream-format", "byte-stream")
        .field("alignment", "au")
        .build();
    h.set_src_caps(caps);

    // First I-frame starts GOP 1 - this succeeds because no signature is created yet
    let buffer1 = create_test_h264_buffer(0, true);
    let result1 = h.push(buffer1);
    // First I-frame might pass (no signature to create), or fail early if key check is done upfront
    
    // Add some P-frames
    for i in 1..5 {
        let buffer = create_test_h264_buffer(i, false);
        let _ = h.push(buffer); // These might pass or fail
    }
    
    // Second I-frame triggers signature for GOP 1 - this MUST fail without private key
    let buffer2 = create_test_h264_buffer(5, true);
    let result2 = h.push(buffer2);
    
    // At least one of these should fail - the second I-frame definitely should
    // because it tries to create a signature without a private key
    assert!(
        result1.is_err() || result2.is_err(),
        "Expected signer to fail without private key when creating signature"
    );
}

#[test]
fn test_verifier_without_signature_meta() {
    init();

    let (_, public_key_path) = create_test_keys();
    let key_store_path = public_key_path.parent().unwrap().to_str().unwrap();

    let mut h = gst_check::Harness::new("dscverifier");
    h.play();

    let verifier = h.element().unwrap();
    verifier.set_property("key-store-path", key_store_path);

    let caps = gst::Caps::builder("video/x-h264")
        .field("stream-format", "byte-stream")
        .field("alignment", "au")
        .build();
    h.set_src_caps(caps);

    // Buffer without signature meta - verifier should handle gracefully for first GOP
    let buffer = create_test_h264_buffer(0, true);
    
    // First I-frame without signature should pass (first GOP)
    let result = h.push(buffer);
    assert!(result.is_ok(), "First I-frame without signature should pass");

    cleanup_test_keys(&PathBuf::new(), &public_key_path);
}

#[test]
fn test_signature_meta_preservation() {
    init();

    let (private_key_path, public_key_path) = create_test_keys();
    let public_key_filename = public_key_path.file_name().unwrap().to_str().unwrap();

    let mut h = gst_check::Harness::new("dscsigner");
    h.play();

    let signer = h.element().unwrap();
    signer.set_property("hash-method", "sha256");
    signer.set_property(
        "private-key-path",
        private_key_path.to_str().unwrap(),
    );
    signer.set_property("public-key-uri", public_key_filename);

    let caps = gst::Caps::builder("video/x-h264")
        .field("stream-format", "byte-stream")
        .field("alignment", "au")
        .build();
    h.set_src_caps(caps);

    // Push multiple I-frames to trigger signature generation
    // First I-frame starts GOP 1
    let buffer1 = create_test_h264_buffer(0, true);
    h.push(buffer1).unwrap();
    
    // Some P-frames in GOP 1
    for i in 1..5 {
        let buffer = create_test_h264_buffer(i, false);
        h.push(buffer).unwrap();
    }
    
    // Second I-frame triggers signature for GOP 1
    let buffer2 = create_test_h264_buffer(5, true);
    h.push(buffer2).unwrap();

    h.push_event(gst::event::Eos::new());

    // Pull all buffers
    let mut signed_buffer_with_meta = None;
    for _ in 0..6 {
        let signed_buffer = h.pull().unwrap();
        if signed_buffer.meta::<gstdsc::signaturemeta::SignatureMeta>().is_some() {
            signed_buffer_with_meta = Some(signed_buffer);
            break;
        }
    }

    // Verify signature meta exists on at least one buffer
    let signed_buffer = signed_buffer_with_meta.expect("Should have at least one buffer with signature meta");
    let meta = signed_buffer.meta::<gstdsc::signaturemeta::SignatureMeta>().unwrap();
    assert!(!meta.signature().is_empty());

    // Test that signature meta is preserved when copying
    let copied_buffer = signed_buffer.copy_deep().unwrap();
    let copied_meta = copied_buffer.meta::<gstdsc::signaturemeta::SignatureMeta>().unwrap();
    assert_eq!(meta.signature(), copied_meta.signature());

    cleanup_test_keys(&private_key_path, &public_key_path);
}

#[test]
fn test_multiple_hash_methods_sequential() {
    init();

    let (private_key_path, public_key_path) = create_test_keys();
    let key_store_path = public_key_path.parent().unwrap().to_str().unwrap();
    let public_key_filename = public_key_path.file_name().unwrap().to_str().unwrap();
    let hash_methods = ["sha1", "sha224", "sha256", "sha384", "sha512"];

    for hash_method in &hash_methods {
        let mut h_signer = gst_check::Harness::new("dscsigner");
        h_signer.play();

        let signer = h_signer.element().unwrap();
        signer.set_property("hash-method", *hash_method);
        signer.set_property(
            "private-key-path",
            private_key_path.to_str().unwrap(),
        );
        signer.set_property("public-key-uri", public_key_filename);

        let caps = gst::Caps::builder("video/x-h264")
            .field("stream-format", "byte-stream")
            .field("alignment", "au")
            .build();
        h_signer.set_src_caps(caps.clone());

        // Push two I-frames to get a signature
        let buffer1 = create_test_h264_buffer(0, true);
        h_signer.push(buffer1).unwrap();
        
        for i in 1..5 {
            let buffer = create_test_h264_buffer(i, false);
            h_signer.push(buffer).unwrap();
        }
        
        let buffer2 = create_test_h264_buffer(5, true);
        h_signer.push(buffer2).unwrap();
        
        h_signer.push_event(gst::event::Eos::new());

        // Verify with matching hash method (read from metadata)
        let mut h_verifier = gst_check::Harness::new("dscverifier");
        h_verifier.play();

        let verifier = h_verifier.element().unwrap();
        verifier.set_property("key-store-path", key_store_path);

        h_verifier.set_src_caps(caps);

        for _ in 0..6 {
            let signed_buffer = h_signer.pull().unwrap();
            h_verifier.push(signed_buffer).unwrap();
        }
        
        h_verifier.push_event(gst::event::Eos::new());

        for _ in 0..6 {
            let _verified_buffer = h_verifier.pull().unwrap();
        }
    }

    cleanup_test_keys(&private_key_path, &public_key_path);
}

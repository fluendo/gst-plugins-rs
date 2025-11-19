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
    let private_key_path = temp_dir.join("test_private_key.pem");
    let public_key_path = temp_dir.join("test_public_key.pem");

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
    signer.set_property("enable-signing", true);

    // Set caps
    let caps = gst::Caps::builder("video/x-h264")
        .field("width", 320)
        .field("height", 240)
        .field("framerate", gst::Fraction::new(30, 1))
        .build();
    h_signer.set_src_caps(caps.clone());

    // Push multiple buffers through signer
    let num_buffers = 10;
    let buffer_size = 1024;

    for i in 0..num_buffers {
        let mut buffer = gst::Buffer::with_size(buffer_size).unwrap();
        {
            let buffer_ref = buffer.get_mut().unwrap();
            let mut map = buffer_ref.map_writable().unwrap();
            // Fill with test pattern
            for (idx, byte) in map.iter_mut().enumerate() {
                *byte = ((i * buffer_size + idx) % 256) as u8;
            }
        }
        h_signer.push(buffer).unwrap();
    }

    h_signer.push_event(gst::event::Eos::new());

    // Create verifier harness
    let mut h_verifier = gst_check::Harness::new("dscverifier");
    h_verifier.play();

    // Configure verifier
    let verifier = h_verifier.element().unwrap();
    verifier.set_property("hash-method", hash_method);
    verifier.set_property(
        "public-key-path",
        public_key_path.to_str().unwrap(),
    );
    verifier.set_property("enable-verification", true);

    h_verifier.set_src_caps(caps);

    // Pull signed buffers from signer and push to verifier
    for _ in 0..num_buffers {
        let signed_buffer = h_signer.pull().unwrap();
        
        // Verify that signature meta exists
        assert!(
            signed_buffer.meta::<gstdsc::signaturemeta::SignatureMeta>().is_some(),
            "Signed buffer should have signature meta"
        );

        h_verifier.push(signed_buffer).unwrap();
    }

    h_verifier.push_event(gst::event::Eos::new());

    // Pull verified buffers
    for _ in 0..num_buffers {
        let verified_buffer = h_verifier.pull().unwrap();
        assert!(verified_buffer.size() == buffer_size);
    }

    cleanup_test_keys(&private_key_path, &public_key_path);
}

#[test]
fn test_signer_without_key() {
    init();

    let mut h = gst_check::Harness::new("dscsigner");
    h.play();

    let caps = gst::Caps::builder("video/x-h264").build();
    h.set_src_caps(caps);

    let buffer = gst::Buffer::with_size(1024).unwrap();
    
    // Should fail because no private key is set
    assert!(h.push(buffer).is_err());
}

#[test]
fn test_verifier_without_signature_meta() {
    init();

    let (_, public_key_path) = create_test_keys();

    let mut h = gst_check::Harness::new("dscverifier");
    h.play();

    let verifier = h.element().unwrap();
    verifier.set_property(
        "public-key-path",
        public_key_path.to_str().unwrap(),
    );

    let caps = gst::Caps::builder("video/x-h264").build();
    h.set_src_caps(caps);

    let buffer = gst::Buffer::with_size(1024).unwrap();
    
    // Should fail because buffer has no signature meta
    assert!(h.push(buffer).is_err());

    cleanup_test_keys(&PathBuf::new(), &public_key_path);
}

#[test]
fn test_signature_meta_preservation() {
    init();

    let (private_key_path, _) = create_test_keys();

    let mut h = gst_check::Harness::new("dscsigner");
    h.play();

    let signer = h.element().unwrap();
    signer.set_property("hash-method", "sha256");
    signer.set_property(
        "private-key-path",
        private_key_path.to_str().unwrap(),
    );

    let caps = gst::Caps::builder("video/x-h264").build();
    h.set_src_caps(caps);

    let buffer = gst::Buffer::with_size(1024).unwrap();
    h.push(buffer).unwrap();

    let signed_buffer = h.pull().unwrap();
    
    // Verify signature meta exists
    let meta = signed_buffer.meta::<gstdsc::signaturemeta::SignatureMeta>().unwrap();
    assert!(meta.signature().len() > 0);

    // Test that signature meta is preserved when copying
    let copied_buffer = signed_buffer.copy_deep().unwrap();
    let copied_meta = copied_buffer.meta::<gstdsc::signaturemeta::SignatureMeta>().unwrap();
    assert_eq!(meta.signature(), copied_meta.signature());

    cleanup_test_keys(&private_key_path, &PathBuf::new());
}

#[test]
fn test_multiple_hash_methods_sequential() {
    init();

    let (private_key_path, public_key_path) = create_test_keys();
    let hash_methods = ["sha1", "sha224", "sha256", "sha384", "sha512"];

    for hash_method in &hash_methods {
        let mut h_signer = gst_check::Harness::new("dscsigner");
        h_signer.play();

        let signer = h_signer.element().unwrap();
        signer.set_property("hash-method", hash_method);
        signer.set_property(
            "private-key-path",
            private_key_path.to_str().unwrap(),
        );

        let caps = gst::Caps::builder("video/x-h264").build();
        h_signer.set_src_caps(caps.clone());

        let buffer = gst::Buffer::with_size(1024).unwrap();
        h_signer.push(buffer).unwrap();
        h_signer.push_event(gst::event::Eos::new());

        let signed_buffer = h_signer.pull().unwrap();

        // Verify with matching hash method
        let mut h_verifier = gst_check::Harness::new("dscverifier");
        h_verifier.play();

        let verifier = h_verifier.element().unwrap();
        verifier.set_property("hash-method", hash_method);
        verifier.set_property(
            "public-key-path",
            public_key_path.to_str().unwrap(),
        );

        h_verifier.set_src_caps(caps);
        h_verifier.push(signed_buffer).unwrap();
        h_verifier.push_event(gst::event::Eos::new());

        let verified_buffer = h_verifier.pull().unwrap();
        assert_eq!(verified_buffer.size(), 1024);
    }

    cleanup_test_keys(&private_key_path, &public_key_path);
}

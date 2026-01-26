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
use openssl::x509::X509;
use openssl::asn1::Asn1Time;
use openssl::bn::{BigNum, MsbOption};
use openssl::hash::MessageDigest;
use openssl::x509::X509NameBuilder;
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
    let cert_path = temp_dir.join(format!("test_cert_{}.pem", unique_id));

    let rsa = Rsa::generate(2048).unwrap();
    let pkey = PKey::from_rsa(rsa).unwrap();

    let private_pem = pkey.private_key_to_pem_pkcs8().unwrap();
    fs::write(&private_key_path, private_pem).unwrap();

    let mut x509_name = X509NameBuilder::new().unwrap();
    x509_name.append_entry_by_text("C", "US").unwrap();
    x509_name.append_entry_by_text("ST", "Test").unwrap();
    x509_name.append_entry_by_text("O", "Test Org").unwrap();
    x509_name.append_entry_by_text("CN", "test.example.com").unwrap();
    let x509_name = x509_name.build();

    let mut cert_builder = X509::builder().unwrap();
    cert_builder.set_version(2).unwrap();

    let serial_number = {
        let mut serial = BigNum::new().unwrap();
        serial.rand(159, MsbOption::MAYBE_ZERO, false).unwrap();
        serial.to_asn1_integer().unwrap()
    };
    cert_builder.set_serial_number(&serial_number).unwrap();

    cert_builder.set_subject_name(&x509_name).unwrap();
    cert_builder.set_issuer_name(&x509_name).unwrap();
    cert_builder.set_pubkey(&pkey).unwrap();

    let not_before = Asn1Time::days_from_now(0).unwrap();
    cert_builder.set_not_before(&not_before).unwrap();
    let not_after = Asn1Time::days_from_now(365).unwrap();
    cert_builder.set_not_after(&not_after).unwrap();

    cert_builder.sign(&pkey, MessageDigest::sha256()).unwrap();
    let cert = cert_builder.build();

    let cert_pem = cert.to_pem().unwrap();
    fs::write(&cert_path, cert_pem).unwrap();

    (private_key_path, cert_path)
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

fn cleanup_test_keys(private_key_path: &PathBuf, cert_path: &PathBuf) {
    let _ = fs::remove_file(private_key_path);
    let _ = fs::remove_file(cert_path);
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

    let (private_key_path, cert_path) = create_test_keys();
    let key_store_path = cert_path.parent().unwrap().to_str().unwrap();

    let pipeline = gst::Pipeline::new();

    let videotestsrc = gst::ElementFactory::make("videotestsrc")
        .property("num-buffers", 10i32)
        .build()
        .unwrap();

    let capsfilter1 = gst::ElementFactory::make("capsfilter")
        .build()
        .unwrap();
    let caps1 = gst::Caps::builder("video/x-raw")
        .field("framerate", gst::Fraction::new(30, 1))
        .build();
    capsfilter1.set_property("caps", caps1);

    let videoconvert = gst::ElementFactory::make("videoconvert")
        .build()
        .unwrap();

    let x264enc = gst::ElementFactory::make("x264enc")
        .property("key-int-max", 5u32)
        .build()
        .unwrap();

    let capsfilter2 = gst::ElementFactory::make("capsfilter")
        .build()
        .unwrap();
    let caps2 = gst::Caps::builder("video/x-h264")
        .field("stream-format", "byte-stream")
        .field("alignment", "au")
        .build();
    capsfilter2.set_property("caps", caps2);

    let signer = gst::ElementFactory::make("dscsigner")
        .property("hash-method", hash_method)
        .property("private-key-path", private_key_path.to_str().unwrap())
        .property("public-key-uri", cert_path.to_str().unwrap())
        .property("substream-length", 5u32)
        .build()
        .unwrap();

    let verifier = gst::ElementFactory::make("dscverifier")
        .property("key-store-path", key_store_path)
        .build()
        .unwrap();

    let fakesink = gst::ElementFactory::make("fakesink")
        .build()
        .unwrap();

    pipeline.add_many([
        &videotestsrc,
        &capsfilter1,
        &videoconvert,
        &x264enc,
        &capsfilter2,
        &signer,
        &verifier,
        &fakesink,
    ]).unwrap();

    gst::Element::link_many([
        &videotestsrc,
        &capsfilter1,
        &videoconvert,
        &x264enc,
        &capsfilter2,
        &signer,
        &verifier,
        &fakesink,
    ]).unwrap();

    let bus = pipeline.bus().unwrap();
    let mut verification_count = 0;
    let mut verification_success = true;
    let mut pipeline_finished = false;

    pipeline.set_state(gst::State::Playing).unwrap();

    while !pipeline_finished {
        let msg = bus.timed_pop(gst::ClockTime::from_seconds(5));

        match msg {
            Some(msg) => {
                use gst::MessageView;

                match msg.view() {
                    MessageView::Eos(..) => {
                        println!("EOS received");
                        pipeline_finished = true;
                    }
                    MessageView::Error(err) => {
                        panic!(
                            "Error from {:?}: {} ({:?})",
                            err.src().map(|s| s.path_string()),
                            err.error(),
                            err.debug()
                        );
                    }
                    MessageView::Element(element_msg) => {
                        if let Some(structure) = element_msg.structure() {
                            if structure.name() == "dsc-verification-result" {
                                if let Ok(verified) = structure.get::<bool>("verified") {
                                    verification_count += 1;
                                    verification_success &= verified;

                                    if verified {
                                        println!("Verification #{} succeeded", verification_count);
                                    } else {
                                        eprintln!("Verification #{} failed", verification_count);
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            None => {
                eprintln!("Timeout waiting for message");
                pipeline_finished = true;
            }
        }
    }

    pipeline.set_state(gst::State::Null).unwrap();

    assert_eq!(verification_count, 2,
        "Expected 2 verification messages (one per GOP), got {}", verification_count);
    assert!(verification_success,
        "All verifications should succeed");

    cleanup_test_keys(&private_key_path, &cert_path);
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

    let buffer1 = create_test_h264_buffer(0, true);
    let result = h.push(buffer1);

    for i in 1..5 {
        let buffer = create_test_h264_buffer(i, false);
        let _ = h.push(buffer); // These might pass or fail
    }

    assert!(
        result.is_err(),
        "Expected signer to fail without private key when creating signature"
    );
}

#[test]
fn test_verifier_without_signature_meta() {
    init();

    let (_, cert_path) = create_test_keys();
    let key_store_path = cert_path.parent().unwrap().to_str().unwrap();

    let mut h = gst_check::Harness::new("dscverifier");
    h.play();

    let verifier = h.element().unwrap();
    verifier.set_property("key-store-path", key_store_path);

    let caps = gst::Caps::builder("video/x-h264")
        .field("stream-format", "byte-stream")
        .field("alignment", "au")
        .build();
    h.set_src_caps(caps);

    let buffer = create_test_h264_buffer(0, true);

    let result = h.push(buffer);
    assert!(result.is_ok(), "First I-frame without signature should pass");

    cleanup_test_keys(&PathBuf::new(), &cert_path);
}

#[test]
fn test_signature_meta_preservation() {
    init();

    let (private_key_path, cert_path) = create_test_keys();

    let mut h = gst_check::Harness::new("dscsigner");

    let caps = gst::Caps::builder("video/x-h264")
        .field("stream-format", "byte-stream")
        .field("alignment", "au")
        .build();
    h.set_src_caps(caps);

    let signer = h.element().unwrap();
    signer.set_property("hash-method", "sha256");
    signer.set_property(
        "private-key-path",
        private_key_path.to_str().unwrap(),
    );
    signer.set_property("public-key-uri", cert_path.to_str().unwrap());
    signer.set_property("substream-length", 5u32);

    h.play();

    let buffer1 = create_test_h264_buffer(0, true);
    h.push(buffer1).unwrap();

    for i in 1..5 {
        let buffer = create_test_h264_buffer(i, false);
        h.push(buffer).unwrap();
    }

    h.push_event(gst::event::Eos::new());

    let mut signed_buffer_with_meta = None;
    for _ in 0..5 {
        let signed_buffer = h.pull().unwrap();
        if signed_buffer.meta::<gst_video::video_meta::VideoDSCVerificationMeta>().is_some() {
            signed_buffer_with_meta = Some(signed_buffer);
            break;
        }
    }

    // Verify signature meta exists on at least one buffer
    let signed_buffer = signed_buffer_with_meta.expect("Should have at least one buffer with signature meta");

    let has_signature_meta = signed_buffer.foreach_meta(|meta| {
        if meta.api().name() == "GstVideoDSCVerificationMeta" {
            std::ops::ControlFlow::Break(())
        } else {
            std::ops::ControlFlow::Continue(())
        }
    });

    assert!(has_signature_meta, "Buffer should have signature meta");

    // Test that signature meta is preserved when copying
    let copied_buffer = signed_buffer.copy_deep().unwrap();
    let copied_has_signature_meta = copied_buffer.foreach_meta(|meta| {
        if meta.api().name() == "GstVideoDSCVerificationMeta" {
            std::ops::ControlFlow::Break(())
        } else {
            std::ops::ControlFlow::Continue(())
        }
    });

    assert!(copied_has_signature_meta, "Copied buffer should have signature meta");

    cleanup_test_keys(&private_key_path, &cert_path);
}

#[test]
fn test_multiple_hash_methods_sequential() {
    init();

    let (private_key_path, cert_path) = create_test_keys();
    let key_store_path = cert_path.parent().unwrap().to_str().unwrap();
    let hash_methods = ["sha1", "sha224", "sha256", "sha384", "sha512"];

    for hash_method in &hash_methods {
        println!("Testing hash method: {}", hash_method);

        let pipeline = gst::Pipeline::new();

        let videotestsrc = gst::ElementFactory::make("videotestsrc")
            .property("num-buffers", 10i32)
            .build()
            .unwrap();

        let capsfilter1 = gst::ElementFactory::make("capsfilter").build().unwrap();
        capsfilter1.set_property("caps", gst::Caps::builder("video/x-raw")
            .field("framerate", gst::Fraction::new(30, 1))
            .build());

        let videoconvert = gst::ElementFactory::make("videoconvert").build().unwrap();
        let x264enc = gst::ElementFactory::make("x264enc")
            .property("key-int-max", 5u32)
            .build()
            .unwrap();

        let capsfilter2 = gst::ElementFactory::make("capsfilter").build().unwrap();
        capsfilter2.set_property("caps", gst::Caps::builder("video/x-h264")
            .field("stream-format", "byte-stream")
            .field("alignment", "au")
            .build());

        let signer = gst::ElementFactory::make("dscsigner")
            .property("hash-method", *hash_method)
            .property("private-key-path", private_key_path.to_str().unwrap())
            .property("public-key-uri", cert_path.to_str().unwrap())
            .build()
            .unwrap();

        let verifier = gst::ElementFactory::make("dscverifier")
            .property("key-store-path", key_store_path)
            .build()
            .unwrap();

        let fakesink = gst::ElementFactory::make("fakesink").build().unwrap();

        pipeline.add_many([&videotestsrc, &capsfilter1, &videoconvert, &x264enc,
                          &capsfilter2, &signer, &verifier, &fakesink]).unwrap();
        gst::Element::link_many([&videotestsrc, &capsfilter1, &videoconvert, &x264enc,
                                &capsfilter2, &signer, &verifier, &fakesink]).unwrap();

        let bus = pipeline.bus().unwrap();
        pipeline.set_state(gst::State::Playing).unwrap();

        let mut pipeline_finished = false;
        while !pipeline_finished {
            if let Some(msg) = bus.timed_pop(gst::ClockTime::from_seconds(5)) {
                use gst::MessageView;
                match msg.view() {
                    MessageView::Eos(..) => pipeline_finished = true,
                    MessageView::Error(err) => {
                        panic!("Error in {}: {} ({:?})", hash_method, err.error(), err.debug());
                    }
                    _ => {}
                }
            } else {
                pipeline_finished = true;
            }
        }

        pipeline.set_state(gst::State::Null).unwrap();
    }

    cleanup_test_keys(&private_key_path, &cert_path);
}

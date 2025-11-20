// Copyright (C) 2025, Fluendo S.A.
//      Author: Diego Nieto <dnieto@fluendo.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v2.0.
// If a copy of the MPL was not distributed with this file, You can obtain one at
// <https://mozilla.org/MPL/2.0/>.
//
// SPDX-License-Identifier: MPL-2.0

use glib::{ParamSpec, ParamSpecString, Value};

use gst::glib;
use gst::prelude::*;
use gst::subclass::prelude::*;
use gst_base::subclass::prelude::*;

use openssl::hash::Hasher;
use openssl::pkey::PKey;
use openssl::sign::Verifier;

use std::fs;
use std::sync::Mutex;
use std::sync::RwLock;
use std::sync::LazyLock;

use anyhow::Result;

use crate::signaturemeta::SignatureMeta;
use crate::common::HashMethod;

static CAT: LazyLock<gst::DebugCategory> = LazyLock::new(|| {
    gst::DebugCategory::new(
        "dscverifier",
        gst::DebugColorFlags::empty(),
        Some("GstDscVerifier"),
    )
});

struct GopVerificationState {
    hasher: Option<Hasher>,
    last_digest: Option<Vec<u8>>,
    previous_gop_digest: Option<Vec<u8>>, // NEW: Store the digest of the GOP we just finished
    gop_started: bool,
}

impl Default for GopVerificationState {
    fn default() -> Self {
        Self {
            hasher: None,
            last_digest: None,
            previous_gop_digest: None, // NEW
            gop_started: false,
        }
    }
}

#[derive(Default)]
pub struct DscVerifier {
    pub hash_method: RwLock<HashMethod>,
    pub public_key: Mutex<Option<PKey<openssl::pkey::Public>>>,
    pub public_key_path: RwLock<Option<String>>,
    pub enable_verification: RwLock<bool>,
    gop_state: Mutex<GopVerificationState>,
}

#[glib::object_subclass]
impl ObjectSubclass for DscVerifier {
    const NAME: &'static str = "DscVerifier";
    type Type = super::DscVerifier;
    type ParentType = gst_base::BaseTransform;
    type Interfaces = ();
}

impl ObjectImpl for DscVerifier {
    fn constructed(&self) {
        self.parent_constructed();
    }

    fn set_property(&self, _id: usize, value: &Value, pspec: &ParamSpec) {
        let obj = self.obj();
        match pspec.name() {
            "hash-method" => {
                let s = value.get::<String>().unwrap();
                let method = match s.as_str() {
                    "sha1" => HashMethod::Sha1,
                    "sha224" => HashMethod::Sha224,
                    "sha256" => HashMethod::Sha256,
                    "sha384" => HashMethod::Sha384,
                    "sha512" => HashMethod::Sha512,
                    _ => HashMethod::Sha256,
                };
                *self.hash_method.write().unwrap() = method;
                gst::info!(*CAT, "Set hash-method property to {} (verifier)", s);
            }
            "public-key-path" => {
                let path = value.get::<String>().unwrap();
                *self.public_key_path.write().unwrap() = Some(path.clone());
                match fs::read(&path) {
                    Ok(key_data) => {
                        match PKey::public_key_from_pem(&key_data) {
                            Ok(pkey) => {
                                *self.public_key.lock().unwrap() = Some(pkey);
                                gst::info!(*CAT, "Loaded public key from {}", path);
                            },
                            Err(e) => {
                                gst::error!(*CAT, "Invalid public key at {}: {}", path, e);
                                let msg = gst::message::Error::new(
                                    gst::CoreError::Failed,
                                    &format!("Invalid public key at {}: {}", path, e),
                                );
                                let _ = obj.post_message(msg);
                            }
                        }
                    },
                    Err(e) => {
                        gst::error!(*CAT, "Failed to read public key file {}: {}", path, e);
                        let msg = gst::message::Error::new(
                            gst::ResourceError::NotFound,
                            &format!("Failed to read public key file {}: {}", path, e),
                        );
                        let _ = obj.post_message(msg);
                    }
                }
            }
            "enable-verification" => {
                let enabled = value.get::<bool>().unwrap();
                *self.enable_verification.write().unwrap() = enabled;
                gst::info!(*CAT, "Set enable-verification property to {}", enabled);
            }
            _ => {}
        }
    }

    fn property(&self, _id: usize, pspec: &ParamSpec) -> Value {
        match pspec.name() {
            "hash-method" => self.hash_method.read().unwrap().to_string().to_value(),
            "public-key-path" => self.public_key_path.read().unwrap().clone().to_value(),
            "enable-verification" => self.enable_verification.read().unwrap().to_value(),
            _ => Value::from_type(pspec.value_type()),
        }
    }

    fn properties() -> &'static [ParamSpec] {
        static PROPERTIES: LazyLock<Vec<ParamSpec>> = LazyLock::new(|| vec![
            ParamSpecString::builder("hash-method")
                .nick("Hash Method")
                .blurb("Hash algorithm to use (sha1, sha224, sha256, sha384, sha512)")
                .default_value(Some("sha256"))
                .readwrite()
                .build(),
            ParamSpecString::builder("public-key-path")
                .nick("Public Key Path")
                .blurb("Path to PEM-encoded public key")
                .readwrite()
                .build(),
            glib::ParamSpecBoolean::builder("enable-verification")
                .nick("Enable Verification")
                .blurb("Enable or disable verification (default: true)")
                .default_value(true)
                .readwrite()
                .build(),
        ]);
        PROPERTIES.as_ref()
    }
}

impl GstObjectImpl for DscVerifier {}
impl ElementImpl for DscVerifier {
    fn metadata() -> Option<&'static gst::subclass::ElementMetadata> {
        static ELEMENT_METADATA: LazyLock<gst::subclass::ElementMetadata> = LazyLock::new(|| {
            gst::subclass::ElementMetadata::new(
                "DSC Verifier",
                "Generic",
                "Verifies video buffer signatures using a public key and hash algorithm",
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

impl BaseTransformImpl for DscVerifier {
    const MODE: gst_base::subclass::BaseTransformMode = gst_base::subclass::BaseTransformMode::AlwaysInPlace;
    const PASSTHROUGH_ON_SAME_CAPS: bool = false;
    const TRANSFORM_IP_ON_PASSTHROUGH: bool = false;

    fn transform_ip(
        &self,
        buffer: &mut gst::BufferRef,
    ) -> Result<gst::FlowSuccess, gst::FlowError> {
        gst::trace!(*CAT, imp = self, "DscVerifier transform_ip called");
        
        let is_i_frame = !buffer.flags().contains(gst::BufferFlags::DELTA_UNIT);
        
        // Check for signature metadata on ALL frames, not just I-frames
        if let Some(sig_meta) = buffer.meta::<SignatureMeta>() {
            gst::info!(*CAT, imp = self, "Found signature metadata on {} frame, signature length: {}", 
                if is_i_frame { "I" } else { "non-I" }, sig_meta.signature().len());
            gst::debug!(*CAT, imp = self, "Signature hash method: {}", sig_meta.hash_method());
        } else {
            gst::trace!(*CAT, imp = self, "No signature metadata on {} frame", 
                if is_i_frame { "I" } else { "non-I" });
        }
        
        let obj = self.obj();
        let pkey_guard = self.public_key.lock().unwrap();
        let pkey = match pkey_guard.as_ref() {
            Some(k) => k,
            None => {
                gst::error!(*CAT, imp = self, "No public key loaded");
                let msg = gst::message::Error::new(
                    gst::ResourceError::NotFound,
                    "No public key loaded",
                );
                let _ = obj.post_message(msg);
                return Err(gst::FlowError::Error);
            }
        };
        
        let hash_method = self.hash_method.read().unwrap().to_openssl();
        let mut gop_state = self.gop_state.lock().unwrap();
        
        // Handle I-frame: check for signature from previous GOP and start new GOP
        if is_i_frame {
            // FIRST: Check if this I-frame has signature meta from previous GOP (BEFORE finalizing current GOP)
            let has_signature = buffer.meta::<SignatureMeta>().is_some();
            
            // Finalize the current GOP 
            if gop_state.gop_started {
                if let Some(mut hasher) = gop_state.hasher.take() {
                    match hasher.finish() {
                        Ok(current_digest) => {
                            gst::debug!(*CAT, imp = self, "Finalized current GOP hash, digest length: {}", current_digest.len());
                            gst::debug!(*CAT, imp = self, "Current GOP digest: {:02x?}", &current_digest[..std::cmp::min(8, current_digest.len())]);
                            
                            // If we have a signature to verify, do it now BEFORE updating previous_gop_digest
                            if has_signature {
                                if let Some(ref previous_digest) = gop_state.previous_gop_digest {
                                    let sig_meta = buffer.meta::<SignatureMeta>().unwrap();
                                    let signature = sig_meta.signature();
                                    
                                    gst::debug!(*CAT, imp = self, "Verifying signature against STORED previous GOP digest: {:02x?}", &previous_digest[..std::cmp::min(8, previous_digest.len())]);
                                    
                                    // Create data packet for verification (must match signer's packet)
                                    let mut data_packet = Vec::new();
                                    
                                    // Add reference digest (last GOP's digest or zeros for first GOP)
                                    if let Some(ref last_digest) = gop_state.last_digest {
                                        data_packet.extend_from_slice(last_digest);
                                        gst::debug!(*CAT, imp = self, "VERIFIER: Added reference digest: {} bytes", last_digest.len());
                                    } else {
                                        // First GOP after stream start: use zero digest
                                        let zero_digest = vec![0u8; previous_digest.len()];
                                        data_packet.extend_from_slice(&zero_digest);
                                        gst::debug!(*CAT, imp = self, "VERIFIER: Added zero reference digest: {} bytes", zero_digest.len());
                                    }
                                    
                                    // Add previous digest (the one the signature was created for)
                                    data_packet.extend_from_slice(previous_digest);
                                    gst::debug!(*CAT, imp = self, "VERIFIER: Added current digest: {} bytes", previous_digest.len());
                                    
                                    // Add hash method type (as single byte)
                                    let hash_method_byte = match hash_method {
                                        m if m == openssl::hash::MessageDigest::sha1() => 0u8,
                                        m if m == openssl::hash::MessageDigest::sha224() => 1u8,
                                        m if m == openssl::hash::MessageDigest::sha256() => 2u8,
                                        m if m == openssl::hash::MessageDigest::sha384() => 3u8,
                                        m if m == openssl::hash::MessageDigest::sha512() => 4u8,
                                        _ => 2u8,
                                    };
                                    data_packet.push(hash_method_byte);
                                    gst::debug!(*CAT, imp = self, "VERIFIER: Added hash method byte: {}", hash_method_byte);
                                    
                                    gst::info!(*CAT, imp = self, "VERIFIER: Verifying data packet: {} bytes total", data_packet.len());
                                    gst::debug!(*CAT, imp = self, "VERIFIER: Previous digest used: {:02x?}", &previous_digest[..std::cmp::min(8, previous_digest.len())]);

                                    // Verify the signature against the data packet
                                    let mut verifier = match Verifier::new(hash_method, pkey) {
                                        Ok(v) => v,
                                        Err(e) => {
                                            gst::error!(*CAT, imp = self, "Failed to create verifier: {}", e);
                                            let msg = gst::message::Error::new(
                                                gst::CoreError::Failed,
                                                &format!("Failed to create verifier: {}", e),
                                            );
                                            let _ = obj.post_message(msg);
                                            return Err(gst::FlowError::Error);
                                        }
                                    };
                                    
                                    if let Err(e) = verifier.update(&data_packet) {
                                        gst::error!(*CAT, imp = self, "Failed to update verifier: {}", e);
                                        let msg = gst::message::Error::new(
                                            gst::CoreError::Failed,
                                            &format!("Failed to update verifier: {}", e),
                                        );
                                        let _ = obj.post_message(msg);
                                        return Err(gst::FlowError::Error);
                                    }
                                    
                                    match verifier.verify(signature) {
                                        Ok(true) => {
                                            gst::info!(*CAT, imp = self, "GOP signature verified successfully");
                                            // Store previous digest as last digest for next GOP
                                            gop_state.last_digest = Some(previous_digest.clone());
                                        },
                                        Ok(false) => {
                                            gst::error!(*CAT, imp = self, "GOP signature verification FAILED");
                                            let msg = gst::message::Error::new(
                                                gst::CoreError::Failed,
                                                "GOP signature verification failed",
                                            );
                                            let _ = obj.post_message(msg);
                                            return Err(gst::FlowError::Error);
                                        },
                                        Err(e) => {
                                            gst::error!(*CAT, imp = self, "Error during signature verification: {}", e);
                                            let msg = gst::message::Error::new(
                                                gst::CoreError::Failed,
                                                &format!("Error during signature verification: {}", e),
                                            );
                                            let _ = obj.post_message(msg);
                                            return Err(gst::FlowError::Error);
                                        }
                                    }
                                } else {
                                    gst::warning!(*CAT, imp = self, "I-frame with signature but no stored previous GOP digest available");
                                }
                            }
                            
                            // Update previous GOP digest for next verification
                            gop_state.previous_gop_digest = Some(current_digest.to_vec());
                        },
                        Err(e) => {
                            gst::error!(*CAT, imp = self, "Failed to finish hash for current GOP: {}", e);
                            let msg = gst::message::Error::new(
                                gst::CoreError::Failed,
                                &format!("Failed to finish hash: {}", e),
                            );
                            let _ = obj.post_message(msg);
                            return Err(gst::FlowError::Error);
                        }
                    }
                } else {
                    gst::warning!(*CAT, imp = self, "GOP was started but no hasher available");
                }
            } else if has_signature {
                // First I-frame without previous GOP
                gst::debug!(*CAT, imp = self, "I-frame without signature meta (first GOP or no signature)");
            }
            
            // Start new GOP - initialize hasher
            let new_hasher = match Hasher::new(hash_method) {
                Ok(h) => h,
                Err(e) => {
                    gst::error!(*CAT, imp = self, "Failed to create hasher for new GOP: {}", e);
                    let msg = gst::message::Error::new(
                        gst::CoreError::Failed,
                        &format!("Failed to create hasher: {}", e),
                    );
                    let _ = obj.post_message(msg);
                    return Err(gst::FlowError::Error);
                }
            };
            
            gop_state.hasher = Some(new_hasher);
            gop_state.gop_started = true;
            
            gst::debug!(*CAT, imp = self, "Started new GOP for verification");
        }
        
        // For ALL frames (including I-frames), accumulate data into current GOP
        if !gop_state.gop_started {
            gst::warning!(*CAT, imp = self, "Received frame before first I-frame, skipping");
            return Ok(gst::FlowSuccess::Ok);
        }
        
        let map = match buffer.map_readable() {
            Ok(m) => m,
            Err(_) => {
                gst::error!(*CAT, imp = self, "Failed to map buffer for reading");
                let msg = gst::message::Error::new(
                    gst::CoreError::Failed,
                    "Failed to map buffer for reading",
                );
                let _ = obj.post_message(msg);
                return Err(gst::FlowError::Error);
            }
        };
        
        if is_i_frame {
            gst::trace!(*CAT, imp = self, "Including I-frame data in GOP hash, size: {}", map.size());
        } else {
            gst::trace!(*CAT, imp = self, "Accumulating frame data into GOP hash, size: {}", map.size());
        }
        
        if let Some(ref mut hasher) = gop_state.hasher {
            if let Err(e) = hasher.update(&map) {
                gst::error!(*CAT, imp = self, "Failed to update hasher: {}", e);
                let msg = gst::message::Error::new(
                    gst::CoreError::Failed,
                    &format!("Failed to update hasher: {}", e),
                );
                let _ = obj.post_message(msg);
                return Err(gst::FlowError::Error);
            }
        }
        
        Ok(gst::FlowSuccess::Ok)
    }
}

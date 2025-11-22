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

use openssl::pkey::PKey;
use openssl::sign::Verifier;

use std::fs;
use std::sync::Mutex;
use std::sync::RwLock;
use std::sync::LazyLock;

use anyhow::Result;

use crate::signaturemeta::SignatureMeta;
use crate::common::HashMethod;
use crate::nal_parser::{NalParser, VideoCodec};
use crate::dsc_substream::DscSubstreamManager;

static CAT: LazyLock<gst::DebugCategory> = LazyLock::new(|| {
    gst::DebugCategory::new(
        "dscverifier",
        gst::DebugColorFlags::empty(),
        Some("GstDscVerifier"),
    )
});

struct GopVerificationState {
    dsc_manager: Option<DscSubstreamManager>,
    gop_started: bool,
    nal_parser: Option<NalParser>,
}

impl Default for GopVerificationState {
    fn default() -> Self {
        Self {
            dsc_manager: None,
            gop_started: false,
            nal_parser: None,
        }
    }
}

#[derive(Default)]
pub struct DscVerifier {
    pub hash_method: RwLock<HashMethod>,
    pub public_key: Mutex<Option<PKey<openssl::pkey::Public>>>,
    pub public_key_path: RwLock<Option<String>>,
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
            _ => {}
        }
    }

    fn property(&self, _id: usize, pspec: &ParamSpec) -> Value {
        match pspec.name() {
            "hash-method" => self.hash_method.read().unwrap().to_string().to_value(),
            "public-key-path" => self.public_key_path.read().unwrap().clone().to_value(),
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

    fn set_caps(&self, incaps: &gst::Caps, outcaps: &gst::Caps) -> Result<(), gst::LoggableError> {
        gst::debug!(*CAT, imp = self, "Negotiating caps");
        gst::debug!(*CAT, imp = self, "Input caps: {}", incaps);
        gst::debug!(*CAT, imp = self, "Output caps: {}", outcaps);

        // Initialize NAL parser based on codec
        if let Ok(codec) = VideoCodec::from_caps(incaps) {
            let mut gop_state = self.gop_state.lock().unwrap();
            gop_state.nal_parser = Some(NalParser::new(codec));
            gst::info!(*CAT, imp = self, "Initialized NAL parser for codec: {:?}", codec);
        } else {
            gst::warning!(*CAT, imp = self, "Could not determine codec from caps, falling back to raw frame verification");
        }

        Ok(())
    }

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
        
        // Handle I-frame: verify signature if present, then start new GOP  
        if is_i_frame {
            // Check if this I-frame has signature meta from previous GOP
            let signature_meta = buffer.meta::<SignatureMeta>();
            
            // If we have a signature, verify it against the PREVIOUS GOP's accumulated data
            if let Some(sig_meta) = signature_meta {
                if gop_state.gop_started {
                    if let Some(ref mut dsc_manager) = gop_state.dsc_manager {
                        match dsc_manager.create_data_packet(0) {
                            Ok(data_packet) => {
                                gst::debug!(*CAT, imp = self, "Created data packet for PREVIOUS GOP verification: {} bytes", data_packet.len());
                                gst::info!(*CAT, imp = self, "VERIFIER: Verifying PREVIOUS GOP data packet: {} bytes", data_packet.len());
                                
                                let signature = sig_meta.signature();
                                
                                gst::debug!(*CAT, imp = self, "VERIFIER: Data packet content: {:02x?}", &data_packet[..std::cmp::min(32, data_packet.len())]);
                                gst::debug!(*CAT, imp = self, "Verifying signature against PREVIOUS GOP data packet");

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
                                        gst::info!(*CAT, imp = self, "✅ PREVIOUS GOP signature verified successfully");
                                    },
                                    Ok(false) => {
                                        gst::error!(*CAT, imp = self, "❌ PREVIOUS GOP signature verification FAILED");
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
                            },
                            Err(e) => {
                                gst::error!(*CAT, imp = self, "Failed to create data packet for verification: {}", e);
                                let msg = gst::message::Error::new(
                                    gst::CoreError::Failed,
                                    &format!("Failed to create data packet: {}", e),
                                );
                                let _ = obj.post_message(msg);
                                return Err(gst::FlowError::Error);
                            }
                        }
                    }
                } else {
                    gst::debug!(*CAT, imp = self, "First I-frame with signature - no previous GOP to verify");
                }
            }
            
            // Start new GOP with DscSubstreamManager
            let hash_method_byte: u8 = (*self.hash_method.read().unwrap()).into();
            
            let new_dsc_manager = DscSubstreamManager::new(
                hash_method,
                hash_method_byte,
                None, // TODO: To be filled in case meta provides it
            );
            
            gop_state.dsc_manager = Some(new_dsc_manager);
            gop_state.gop_started = true;
            
            gst::debug!(*CAT, imp = self, "Started new GOP for verification with DscSubstreamManager");
        }
        
        // For ALL frames (including I-frames), accumulate data into CURRENT GOP
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
        
        // Extract data to hash using NAL parser
        let data_to_hash = if let Some(ref nal_parser) = gop_state.nal_parser {
            match nal_parser.extract_signable_data(&map) {
                Ok(nal_data) => {
                    gst::debug!(*CAT, imp = self, "Using NAL-level verification: {} bytes from {} raw bytes", 
                        nal_data.len(), map.len());
                    nal_data
                },
                Err(e) => {
                    gst::error!(*CAT, imp = self, "NAL parsing failed: {}", e);
                    let msg = gst::message::Error::new(
                        gst::CoreError::Failed,
                        &format!("NAL parsing failed: {}", e),
                    );
                    let _ = obj.post_message(msg);
                    return Err(gst::FlowError::Error);
                }
            }
        } else {
            gst::error!(*CAT, imp = self, "No NAL parser available - codec not supported");
            let msg = gst::message::Error::new(
                gst::CoreError::Failed,
                "No NAL parser available - codec not supported",
            );
            let _ = obj.post_message(msg);
            return Err(gst::FlowError::Error);
        };

        // Add data to DSC substream for current GOP
        if let Some(ref mut dsc_manager) = gop_state.dsc_manager {
            if let Err(e) = dsc_manager.add_to_substream(0, &data_to_hash) {
                gst::error!(*CAT, imp = self, "Failed to add data to substream: {}", e);
                return Err(gst::FlowError::Error);
            }

            if is_i_frame {
                gst::trace!(*CAT, imp = self, "Added CURRENT I-frame data to NEW GOP substream, size: {}", data_to_hash.len());
            } else {
                gst::trace!(*CAT, imp = self, "Added frame data to current GOP substream, size: {}", data_to_hash.len());
            }
        }
        
        Ok(gst::FlowSuccess::Ok)
    }
}

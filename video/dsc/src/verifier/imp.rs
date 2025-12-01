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
use std::path::Path;
use std::sync::{Mutex, RwLock};
use std::sync::LazyLock;
use std::collections::HashMap;

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
    public_key_cache: HashMap<String, PKey<openssl::pkey::Public>>,
}

impl Default for GopVerificationState {
    fn default() -> Self {
        Self {
            dsc_manager: None,
            gop_started: false,
            nal_parser: None,
            public_key_cache: HashMap::new(),
        }
    }
}

#[derive(Default)]
pub struct DscVerifier {
    key_store_path: RwLock<Option<String>>,
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
        match pspec.name() {
            "key-store-path" => {
                let path = value.get::<String>().unwrap();
                *self.key_store_path.write().unwrap() = Some(path.clone());
                gst::info!(*CAT, "Set key-store-path property to {}", path);
            }
            _ => {}
        }
    }

    fn property(&self, _id: usize, pspec: &ParamSpec) -> Value {
        match pspec.name() {
            "key-store-path" => self.key_store_path.read().unwrap().clone().to_value(),
            _ => Value::from_type(pspec.value_type()),
        }
    }

    fn properties() -> &'static [ParamSpec] {
        static PROPERTIES: LazyLock<Vec<ParamSpec>> = LazyLock::new(|| vec![
            ParamSpecString::builder("key-store-path")
                .nick("Key Store Path")
                .blurb("Directory path where public key files are stored (keyStoreDir)")
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
                "Verifies video buffer signatures using metadata-provided keys",
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

impl DscVerifier {
    fn load_public_key_from_cert_uri(
        &self,
        cert_uri: &str,
        gop_state: &mut GopVerificationState,
    ) -> std::result::Result<PKey<openssl::pkey::Public>, openssl::error::ErrorStack> {
        if let Some(cached_key) = gop_state.public_key_cache.get(cert_uri) {
            gst::debug!(*CAT, imp = self, "Using cached public key for cert_uri: {}", cert_uri);
            return Ok(cached_key.clone());
        }

        gst::info!(*CAT, imp = self, "Loading public key from cert_uri: {}", cert_uri);
        
        // In case not cached, load the key by composing the path based on key_store_path and metadata cert_uri
        let final_key_path = if let Some(key_store_path) = &*self.key_store_path.read().unwrap() {
            let key_store_dir = Path::new(key_store_path);
            let cert_path = Path::new(cert_uri);
            
            if cert_path.is_absolute() {
                cert_uri.to_string()
            } else {
                key_store_dir.join(cert_uri).to_string_lossy().to_string()
            }
        } else {
            cert_uri.to_string()
        };

        gst::debug!(*CAT, imp = self, "Final key path constructed: {}", final_key_path);
        
        match fs::read(&final_key_path) {
            Ok(key_data) => {
                match PKey::public_key_from_pem(&key_data) {
                    Ok(pkey) => {
                        gop_state.public_key_cache.insert(cert_uri.to_string(), pkey.clone());
                        gst::info!(*CAT, imp = self, "Loaded and cached public key from: {}", final_key_path);
                        Ok(pkey)
                    },
                    Err(e) => {
                        gst::error!(*CAT, imp = self, "Invalid public key at {}: {}", final_key_path, e);
                        Err(e)
                    }
                }
            },
            Err(e) => {
                gst::error!(*CAT, imp = self, "Failed to read public key from {}: {}", final_key_path, e);
                Err(openssl::error::ErrorStack::get())
            }
        }
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
        let obj = self.obj();
        let mut gop_state = self.gop_state.lock().unwrap();
        
        // Check signature of previous GOP if present
        if is_i_frame {
            let signature_meta = buffer.meta::<SignatureMeta>();
            
            // If we have a signature, verify it against the PREVIOUS GOP's accumulated data
            if let Some(sig_meta) = signature_meta {
                gst::info!(*CAT, imp = self, "Found signature metadata on I-frame, signature length: {}", 
                    sig_meta.signature().len());
                gst::debug!(*CAT, imp = self, "Signature hash method: {}, cert_uri: {:?}, content_uuid: {:?}", 
                    sig_meta.hash_method(), sig_meta.cert_uri(), sig_meta.content_uuid());
                
                if gop_state.gop_started {
                    let hash_method = HashMethod::from(sig_meta.hash_method());
                    let openssl_hash_method = hash_method.to_openssl();
                    
                    let pkey = if let Some(cert_uri) = sig_meta.cert_uri() {
                        match self.load_public_key_from_cert_uri(cert_uri, &mut gop_state) {
                            Ok(key) => key,
                            Err(e) => {
                                let msg = gst::message::Error::new(
                                    gst::CoreError::Failed,
                                    &format!("Failed to load public key from cert_uri '{}': {}", cert_uri, e),
                                );
                                let _ = obj.post_message(msg);
                                return Err(gst::FlowError::Error);
                            }
                        }
                    } else {
                        gst::error!(*CAT, imp = self, "No cert_uri provided in signature metadata");
                        let msg = gst::message::Error::new(
                            gst::ResourceError::NotFound,
                            "No cert_uri provided in signature metadata",
                        );
                        let _ = obj.post_message(msg);
                        return Err(gst::FlowError::Error);
                    };
                    
                    if let Some(ref mut dsc_manager) = gop_state.dsc_manager {
                        match dsc_manager.create_data_packet(0) {
                            Ok(data_packet) => {
                                gst::debug!(*CAT, imp = self, "Created data packet for PREVIOUS GOP verification: {} bytes", data_packet.len());
                                gst::info!(*CAT, imp = self, "VERIFIER: Verifying PREVIOUS GOP data packet: {} bytes with hash method: {:?}", data_packet.len(), hash_method);
                                
                                let signature = sig_meta.signature();
                                
                                gst::debug!(*CAT, imp = self, "VERIFIER: Data packet content: {:02x?}", &data_packet[..std::cmp::min(32, data_packet.len())]);
                                gst::debug!(*CAT, imp = self, "Verifying signature against PREVIOUS GOP data packet");

                                // Verify the signature against the data packet using metadata-provided hash method
                                let mut verifier = match Verifier::new(openssl_hash_method, &pkey) {
                                    Ok(v) => v,
                                    Err(e) => {
                                        gst::error!(*CAT, imp = self, "Failed to create verifier with hash method {:?}: {}", hash_method, e);
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
                
                // Start new GOP with DscSubstreamManager using metadata parameters
                let hash_method = HashMethod::from(sig_meta.hash_method());
                let openssl_hash_method = hash_method.to_openssl();
                
                let new_dsc_manager = DscSubstreamManager::new(
                    openssl_hash_method,
                    sig_meta.hash_method(),
                    sig_meta.content_uuid().copied(),
                );
                
                gop_state.dsc_manager = Some(new_dsc_manager);
                
                gst::info!(*CAT, imp = self, "Started new GOP for verification with DSC parameters from metadata");
                gst::debug!(*CAT, imp = self, "DSC parameters - hash_method: {:?}, cert_uri: {:?}, content_uuid: {:?}", 
                    hash_method, sig_meta.cert_uri(), sig_meta.content_uuid());
            } else {
                gst::debug!(*CAT, imp = self, "I-frame without signature metadata - starting GOP without DSC verification");
            }
            gop_state.gop_started = true;
        }
        
        // Ensure GOP has started before accumulating data
        if gop_state.dsc_manager.is_some() {
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
                        gst::trace!(*CAT, imp = self, "Using NAL-level verification: {} bytes from {} raw bytes", 
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
                gst::error!(*CAT, imp = self, "No NAL parser available");
                return Err(gst::FlowError::Error);
            };

            // Add current frame selected NALs to DSC substream in the current GOP
            if let Some(ref mut dsc_manager) = gop_state.dsc_manager {
                if let Err(e) = dsc_manager.add_to_substream(0, &data_to_hash) {
                    gst::error!(*CAT, imp = self, "Failed to add data to substream: {}", e);
                    return Err(gst::FlowError::Error);
                }

                gst::trace!(*CAT, imp = self, "Added {} frame data to current GOP substream, size: {}", 
                    if is_i_frame { "I" } else { "non-I" }, data_to_hash.len());
            }
        }
        
        Ok(gst::FlowSuccess::Ok)
    }
}

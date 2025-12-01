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
        "dscsigner",
        gst::DebugColorFlags::empty(),
        Some("GstDscSigner"),
    )
});

struct GopSigningState {
    dsc_manager: Option<DscSubstreamManager>,
    gop_started: bool,
    frames_in_gop: u32,
    nal_parser: Option<NalParser>,
}

impl Default for GopSigningState {
    fn default() -> Self {
        Self {
            dsc_manager: None,
            gop_started: false,
            frames_in_gop: 0,
            nal_parser: None,
        }
    }
}

#[derive(Default)]
pub struct DscSigner {
    pub hash_method: RwLock<HashMethod>,
    pub private_key: Mutex<Option<PKey<openssl::pkey::Private>>>,
    pub private_key_path: RwLock<Option<String>>,
    pub enable_signing: RwLock<bool>,
    pub public_key_uri: RwLock<Option<String>>,
    pub content_uuid: RwLock<Option<[u8; 16]>>,
    gop_state: Mutex<GopSigningState>,
}

#[glib::object_subclass]
impl ObjectSubclass for DscSigner {
    const NAME: &'static str = "DscSigner";
    type Type = super::DscSigner;
    type ParentType = gst_base::BaseTransform;
    type Interfaces = ();
}

impl ObjectImpl for DscSigner {
    fn constructed(&self) {
        self.parent_constructed();
    }

    fn set_property(&self, _id: usize, value: &Value, pspec: &ParamSpec) {
    let obj = self.obj();
    // All property access is under lock for thread safety
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
                gst::info!(*CAT, "Set hash-method property to {}", s);
            }
            "private-key-path" => {
                let path = value.get::<String>().unwrap();
                *self.private_key_path.write().unwrap() = Some(path.clone());
                match fs::read(&path) {
                    Ok(key_data) => {
                        match PKey::private_key_from_pem(&key_data) {
                            Ok(pkey) => {
                                // Validate key type (must be RSA or EC for most signature schemes)
                                let key_type = pkey.id();
                                if key_type != openssl::pkey::Id::RSA && key_type != openssl::pkey::Id::EC {
                                    gst::error!(*CAT, "Unsupported private key type: {:?}", key_type);
                                    let msg = gst::message::Error::new(
                                        gst::CoreError::Failed,
                                        &format!("Unsupported private key type: {:?}", key_type),
                                    );
                                    let _ = obj.post_message(msg);
                                    return;
                                }
                                *self.private_key.lock().unwrap() = Some(pkey);
                                gst::info!(*CAT, "Loaded private key from {}", path);
                            },
                            Err(e) => {
                                gst::error!(*CAT, "Invalid private key at {}: {}", path, e);
                                let msg = gst::message::Error::new(
                                    gst::CoreError::Failed,
                                    &format!("Invalid private key at {}: {}", path, e),
                                );
                                let _ = obj.post_message(msg);
                            }
                        }
                    },
                    Err(e) => {
                        gst::error!(*CAT, "Failed to read private key file {}: {}", path, e);
                        let msg = gst::message::Error::new(
                            gst::ResourceError::NotFound,
                            &format!("Failed to read private key file {}: {}", path, e),
                        );
                        let _ = obj.post_message(msg);
                    }
                }
            }
            "public-key-uri" => {
                let uri = value.get::<String>().unwrap();
                *self.public_key_uri.write().unwrap() = Some(uri.clone());
                gst::info!(*CAT, "Set public-key-uri property to {}", uri);
            }
            "content-uuid" => {
                let uuid_str = value.get::<String>().unwrap();
                if uuid_str.len() == 32 {
                    let mut uuid = [0u8; 16];
                    if hex::decode_to_slice(&uuid_str, &mut uuid).is_ok() {
                        *self.content_uuid.write().unwrap() = Some(uuid);
                        gst::info!(*CAT, "Set content-uuid property to {}", uuid_str);
                    } else {
                        gst::error!(*CAT, "Invalid hex string for content-uuid: {}", uuid_str);
                    }
                } else {
                    gst::error!(*CAT, "Content UUID must be 32 hex characters, got: {}", uuid_str.len());
                }
            }
            _ => {}
        }
    }

    fn property(&self, _id: usize, pspec: &ParamSpec) -> Value {
        match pspec.name() {
            "hash-method" => self.hash_method.read().unwrap().to_string().to_value(),
            "private-key-path" => self.private_key_path.read().unwrap().clone().to_value(),
            "public-key-uri" => self.public_key_uri.read().unwrap().clone().to_value(),
            "content-uuid" => {
                if let Some(uuid) = *self.content_uuid.read().unwrap() {
                    hex::encode(uuid).to_value()
                } else {
                    String::new().to_value()
                }
            }
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
            ParamSpecString::builder("private-key-path")
                .nick("Private Key Path")
                .blurb("Path to PEM-encoded private key")
                .readwrite()
                .build(),
            ParamSpecString::builder("public-key-uri")
                .nick("Public Key URI")
                .blurb("URI of the public key for signature verification")
                .readwrite()
                .build(),
            ParamSpecString::builder("content-uuid")
                .nick("Content UUID")
                .blurb("Content UUID as hex string (32 characters)")
                .readwrite()
                .build(),
        ]);
        PROPERTIES.as_ref()
    }
}

impl GstObjectImpl for DscSigner {}
impl ElementImpl for DscSigner {
    fn metadata() -> Option<&'static gst::subclass::ElementMetadata> {
        static ELEMENT_METADATA: LazyLock<gst::subclass::ElementMetadata> = LazyLock::new(|| {
            gst::subclass::ElementMetadata::new(
                "DSC Signer",
                "Generic",
                "Signs video buffers using a private key and hash algorithm",
                "Diego Nieto <dnieto@fluendo.com>",
            )
        });

        Some(&*ELEMENT_METADATA)
    }

    fn pad_templates() -> &'static [gst::PadTemplate] {
        static TEMPLATES: LazyLock<Vec<gst::PadTemplate>> = LazyLock::new(|| {
            let caps = gst::Caps::builder_full()
                .structure(gst::Structure::builder("video/x-h264").build())
                .structure(gst::Structure::builder("video/x-h265").build())
                .structure(gst::Structure::builder("video/x-h266").build())
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

impl BaseTransformImpl for DscSigner {
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
            gst::warning!(*CAT, imp = self, "Could not determine codec from caps, falling back to raw frame signing");
        }

        Ok(())
    }

    fn transform_ip(
        &self,
        buffer: &mut gst::BufferRef,
    ) -> Result<gst::FlowSuccess, gst::FlowError> {
        gst::trace!(*CAT, imp = self, "DscSigner transform_ip called");
        
        let is_i_frame = !buffer.flags().contains(gst::BufferFlags::DELTA_UNIT);
        let mut gop_state = self.gop_state.lock().unwrap();
        
        if is_i_frame {
            gst::debug!(*CAT, imp = self, "Processing I-frame (keyframe)");
            
            // Check whether there is a previous GOP to sign
            if gop_state.gop_started {
                if let Some(ref mut dsc_manager) = gop_state.dsc_manager {
                    // Create data packet to sign it
                    match dsc_manager.create_data_packet(0) {
                        Ok(data_packet) => {
                            gst::debug!(*CAT, imp = self, "Created data packet: {} bytes", data_packet.len());
                            gst::info!(*CAT, imp = self, "SIGNER: Creating signature for COMPLETED GOP data packet: {} bytes", data_packet.len());
                            
                            let signature = self.create_signature_from_data_packet(&data_packet)?;
                            let hash_method: u8 = (*self.hash_method.read().unwrap()).into();
                            let content_uuid = *self.content_uuid.read().unwrap();
                            let public_key_uri = self.public_key_uri.read().unwrap().clone();
                            
                            gst::info!(*CAT, imp = self, "🔐 Created signature for COMPLETED GOP, signature length: {}", signature.len());
                            
                            // Attach signature to this I-frame (start of next GOP)
                            SignatureMeta::add(
                                buffer, 
                                &signature, 
                                hash_method, 
                                public_key_uri.as_deref(), 
                                content_uuid.as_ref()
                            );
                            gst::info!(*CAT, imp = self, "✅ ATTACHED signature meta to I-frame for completed GOP (public_key_uri: {:?})", public_key_uri);
                        },
                        Err(e) => {
                            gst::error!(*CAT, imp = self, "Failed to create data packet: {}", e);
                            return Err(gst::FlowError::Error);
                        }
                    }
                }
            } else {
                gst::debug!(*CAT, imp = self, "First I-frame - no previous GOP to sign");
            }
            
            // Start new GOP with DscSubstreamManager
            let hash_method = self.hash_method.read().unwrap().to_openssl();
            let hash_method_byte: u8 = (*self.hash_method.read().unwrap()).into();
            let content_uuid = *self.content_uuid.read().unwrap();
            
            let new_dsc_manager = DscSubstreamManager::new(
                hash_method,
                hash_method_byte,
                content_uuid,
            );
            
            gop_state.dsc_manager = Some(new_dsc_manager);
            gop_state.gop_started = true;
            gop_state.frames_in_gop = 0;
            
            gst::debug!(*CAT, imp = self, "Started new GOP with DscSubstreamManager");
        } else {
            gst::trace!(*CAT, imp = self, "Processing non-I-frame");
        }
        
        let map = buffer.map_readable().map_err(|_| {
            gst::error!(*CAT, imp = self, "Failed to map buffer for reading");
            gst::FlowError::Error
        })?;
        
        // Extract only desired NAL units based on codec
        let data_to_hash = if let Some(ref nal_parser) = gop_state.nal_parser {
            match nal_parser.extract_signable_data(&map) {
                Ok(nal_data) => {
                    gst::debug!(*CAT, imp = self, "Using NAL-level signing: {} bytes from {} raw bytes", 
                        nal_data.len(), map.len());
                    nal_data
                },
                Err(e) => {
                    gst::error!(*CAT, imp = self, "NAL parsing failed: {}", e);
                    let obj = self.obj();
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
            let obj = self.obj();
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
                gst::trace!(*CAT, imp = self, "Added current I-frame data to NEW GOP substream, size: {}", data_to_hash.len());
            } else {
                gst::trace!(*CAT, imp = self, "Added frame data to current GOP substream, size: {}", data_to_hash.len());
            }
            
            gop_state.frames_in_gop += 1;
            gst::trace!(*CAT, imp = self, "GOP now has {} frames", gop_state.frames_in_gop);
        } else {
            gst::warning!(*CAT, imp = self, "No DSC manager available to accumulate frame data!");
        }
        
        Ok(gst::FlowSuccess::Ok)
    }
}

impl DscSigner {
    fn create_signature_from_data_packet(
        &self,
        data_packet: &[u8],
    ) -> Result<Vec<u8>, gst::FlowError> {
        use openssl::sign::Signer;

        let pkey_guard = self.private_key.lock().unwrap();
        let pkey = match pkey_guard.as_ref() {
            Some(k) => k,
            None => {
                gst::error!(*CAT, imp = self, "No private key loaded");
                return Err(gst::FlowError::Error);
            }
        };

        let hash_method = self.hash_method.read().unwrap().to_openssl();

        gst::debug!(*CAT, imp = self, "SIGNER: Data packet content: {:02x?}", &data_packet[..std::cmp::min(32, data_packet.len())]);
        
        let mut signer = Signer::new(hash_method, pkey).map_err(|e| {
            gst::error!(*CAT, imp = self, "Failed to create signer: {}", e);
            gst::FlowError::Error
        })?;

        signer.update(data_packet).map_err(|e| {
            gst::error!(*CAT, imp = self, "Failed to update signer: {}", e);
            gst::FlowError::Error
        })?;

        let signature = signer.sign_to_vec().map_err(|e| {
            gst::error!(*CAT, imp = self, "Failed to create signature: {}", e);
            gst::FlowError::Error
        })?;

        gst::debug!(*CAT, imp = self, "Created signature: {} bytes", signature.len());
        Ok(signature)
    }
}
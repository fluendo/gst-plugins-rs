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

use std::fs;
use std::sync::Mutex;
use std::sync::RwLock;
use std::sync::LazyLock;

use anyhow::Result;

use crate::signaturemeta::SignatureMeta;
use crate::common::HashMethod;

static CAT: LazyLock<gst::DebugCategory> = LazyLock::new(|| {
    gst::DebugCategory::new(
        "dscsigner",
        gst::DebugColorFlags::empty(),
        Some("GstDscSigner"),
    )
});

struct GopSigningState {
    hasher: Option<Hasher>,
    last_digest: Option<Vec<u8>>,
    gop_started: bool,
    pending_signature: Option<(Vec<u8>, u8, Option<String>, Option<[u8; 16]>)>, // (signature, hash_method, cert_uri, content_uuid)
    frames_in_gop: u32,
}

impl Default for GopSigningState {
    fn default() -> Self {
        Self {
            hasher: None,
            last_digest: None,
            gop_started: false,
            pending_signature: None,
            frames_in_gop: 0,
        }
    }
}

#[derive(Default)]
pub struct DscSigner {
    pub hash_method: RwLock<HashMethod>,
    pub private_key: Mutex<Option<PKey<openssl::pkey::Private>>>,
    pub private_key_path: RwLock<Option<String>>,
    pub enable_signing: RwLock<bool>,
    pub cert_uri: RwLock<Option<String>>,
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
            "enable-signing" => {
                let enabled = value.get::<bool>().unwrap();
                *self.enable_signing.write().unwrap() = enabled;
                gst::info!(*CAT, "Set enable-signing property to {}", enabled);
            }
            "cert-uri" => {
                let uri = value.get::<String>().unwrap();
                *self.cert_uri.write().unwrap() = Some(uri.clone());
                gst::info!(*CAT, "Set cert-uri property to {}", uri);
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
            "enable-signing" => self.enable_signing.read().unwrap().to_value(),
            "cert-uri" => self.cert_uri.read().unwrap().clone().to_value(),
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
            glib::ParamSpecBoolean::builder("enable-signing")
                .nick("Enable Signing")
                .blurb("Enable or disable signing (default: true)")
                .default_value(true)
                .readwrite()
                .build(),
            ParamSpecString::builder("cert-uri")
                .nick("Certificate URI")
                .blurb("URI of the certificate for signature verification")
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
                "Signs video buffers using a private key and hash algorithm",  // description
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

    fn transform_ip(
        &self,
        buffer: &mut gst::BufferRef,
    ) -> Result<gst::FlowSuccess, gst::FlowError> {
        gst::trace!(*CAT, imp = self, "DscSigner transform_ip called");
        
        let is_i_frame = !buffer.flags().contains(gst::BufferFlags::DELTA_UNIT);
        let mut gop_state = self.gop_state.lock().unwrap();
        
        if is_i_frame {
            gst::debug!(*CAT, imp = self, "Processing I-frame (keyframe)");
            
            // If we have a pending signature from previous GOP, attach it to this I-frame
            if let Some((signature, hash_method, cert_uri, content_uuid)) = gop_state.pending_signature.take() {
                gst::debug!(*CAT, imp = self, "Attaching pending signature from previous GOP, signature length: {}", signature.len());
                
                SignatureMeta::add(
                    buffer, 
                    &signature, 
                    hash_method, 
                    cert_uri.as_deref(), 
                    content_uuid.as_ref()
                );
                gst::info!(*CAT, imp = self, "✅ ATTACHED signature meta to I-frame");
            } else {
                gst::debug!(*CAT, imp = self, "No pending signature to attach (first GOP)");
            }
            
            // Finalize previous GOP and prepare signature for NEXT GOP
            if gop_state.gop_started {
                if let Some(mut hasher) = gop_state.hasher.take() {
                    let current_digest = match hasher.finish() {
                        Ok(d) => d,
                        Err(e) => {
                            gst::error!(*CAT, imp = self, "Failed to finish hash: {}", e);
                            return Err(gst::FlowError::Error);
                        }
                    };
                    
                    gst::debug!(*CAT, imp = self, "Finalized GOP hash, digest length: {}", current_digest.len());
                    
                    // Create signature for this GOP
                    let signature = self.create_signature(&gop_state.last_digest, &current_digest)?;
                    let hash_method: u8 = (*self.hash_method.read().unwrap()).into();
                    let cert_uri = self.cert_uri.read().unwrap().clone();
                    let content_uuid = *self.content_uuid.read().unwrap();
                    
                    gst::info!(*CAT, imp = self, "🔐 Created signature for GOP, signature length: {}", signature.len());
                    
                    // Store signature to be attached to NEXT I-frame
                    gop_state.pending_signature = Some((signature, hash_method, cert_uri, content_uuid));
                    gop_state.last_digest = Some(current_digest.to_vec());
                    
                    gst::debug!(*CAT, imp = self, "Stored pending signature for next GOP");
                }
            }
            
            // Start new GOP
            let hash_method = self.hash_method.read().unwrap().to_openssl();
            let new_hasher = match Hasher::new(hash_method) {
                Ok(h) => h,
                Err(e) => {
                    gst::error!(*CAT, imp = self, "Failed to create hasher: {}", e);
                    return Err(gst::FlowError::Error);
                }
            };
            
            gop_state.hasher = Some(new_hasher);
            gop_state.gop_started = true;
            gop_state.frames_in_gop = 0;
            
            gst::debug!(*CAT, imp = self, "Started new GOP for signing");
        } else {
            gst::trace!(*CAT, imp = self, "Processing non-I-frame");
        }
        
        // Accumulate current frame data for ALL frames (including I-frames)
        let map = buffer.map_readable().map_err(|_| {
            gst::error!(*CAT, imp = self, "Failed to map buffer for reading");
            gst::FlowError::Error
        })?;
        
        if let Some(ref mut hasher) = gop_state.hasher {
            hasher.update(&map).map_err(|e| {
                gst::error!(*CAT, imp = self, "Failed to update hasher: {}", e);
                gst::FlowError::Error
            })?;
            
            if is_i_frame {
                gst::trace!(*CAT, imp = self, "Included I-frame data in GOP hash, size: {}", map.size());
            } else {
                gst::trace!(*CAT, imp = self, "Accumulated frame data into GOP hash, size: {}", map.size());
            }
            
            gop_state.frames_in_gop += 1;
            gst::trace!(*CAT, imp = self, "GOP now has {} frames", gop_state.frames_in_gop);
        } else {
            gst::warning!(*CAT, imp = self, "No hasher available to accumulate frame data!");
        }
        
        Ok(gst::FlowSuccess::Ok)
    }
}

impl DscSigner {
    fn create_signature(
        &self,
        last_digest: &Option<Vec<u8>>,
        current_digest: &[u8],
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

        // Create data packet (same format as verifier expects)
        let mut data_packet = Vec::new();

        // Add reference digest (last GOP's digest or zeros for first GOP)
        if let Some(ref last_digest) = last_digest {
            data_packet.extend_from_slice(last_digest);
            gst::debug!(*CAT, imp = self, "SIGNER: Added reference digest: {} bytes", last_digest.len());
        } else {
            // First GOP: use zero digest
            let zero_digest = vec![0u8; current_digest.len()];
            data_packet.extend_from_slice(&zero_digest);
            gst::debug!(*CAT, imp = self, "SIGNER: Added zero reference digest: {} bytes", zero_digest.len());
        }

        // Add current digest
        data_packet.extend_from_slice(current_digest);
        gst::debug!(*CAT, imp = self, "SIGNER: Added current digest: {} bytes", current_digest.len());

        // Add hash method type (as single byte)
        let hash_method_byte: u8 = (*self.hash_method.read().unwrap()).into();
        data_packet.push(hash_method_byte);
        gst::debug!(*CAT, imp = self, "SIGNER: Added hash method byte: {}", hash_method_byte);

        gst::info!(*CAT, imp = self, "SIGNER: Creating signature for data packet: {} bytes total", data_packet.len());
        gst::debug!(*CAT, imp = self, "SIGNER: Current digest: {:02x?}", &current_digest[..std::cmp::min(8, current_digest.len())]);
        
        // Create signature
        let mut signer = Signer::new(hash_method, pkey).map_err(|e| {
            gst::error!(*CAT, imp = self, "Failed to create signer: {}", e);
            gst::FlowError::Error
        })?;

        signer.update(&data_packet).map_err(|e| {
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
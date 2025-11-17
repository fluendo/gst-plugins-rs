// Copyright (C) 2025, Fluendo S.A.
//      Author: Diego Nieto <dnieto@fluendo.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v2.0.
// If a copy of the MPL was not distributed with this file, You can obtain one at
// <https://mozilla.org/MPL/2.0/>.
//
// SPDX-License-Identifier: MPL-2.0

use glib::{ParamSpec, ParamSpecString, ParamSpecUInt, Value};

use gst::glib;
use gst::prelude::*;
use gst::subclass::prelude::*;
use gst_base::subclass::prelude::*;

use openssl::pkey::PKey;
use openssl::sign::Signer;

use std::fs;
use std::sync::Mutex;
use std::sync::RwLock;
use std::sync::LazyLock;
use std::ffi::CString;

use anyhow::Result;

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
    buffers_in_substream: u32,
    nal_parser: Option<NalParser>,
}

impl Default for GopSigningState {
    fn default() -> Self {
        Self {
            dsc_manager: None,
            gop_started: false,
            buffers_in_substream: 0,
            nal_parser: None,
        }
    }
}

#[derive(Default)]
pub struct DscSigner {
    pub hash_method: RwLock<HashMethod>,
    pub private_key: Mutex<Option<PKey<openssl::pkey::Private>>>,
    pub private_key_path: RwLock<Option<String>>,
    pub public_key_uri: RwLock<Option<String>>,
    pub content_uuid: RwLock<Option<[u8; 16]>>,
    pub substream_length: RwLock<u32>,
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
        match pspec.name() {
            "hash-method" => {
                let s = value.get::<String>().unwrap();
                let method = match s.as_str() {
                    "sha1" => HashMethod::Sha1,
                    "sha224" => HashMethod::Sha224,
                    "sha256" => HashMethod::Sha256,
                    "sha384" => HashMethod::Sha384,
                    "sha512" => HashMethod::Sha512,
                    _ => HashMethod::Sha512,
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
            "substream-length" => {
                let length = value.get::<u32>().unwrap();
                *self.substream_length.write().unwrap() = length;
                gst::info!(*CAT, "Set substream-length property to {}", length);
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
            "substream-length" => self.substream_length.read().unwrap().to_value(),
            _ => Value::from_type(pspec.value_type()),
        }
    }

    fn properties() -> &'static [ParamSpec] {
        static PROPERTIES: LazyLock<Vec<ParamSpec>> = LazyLock::new(|| vec![
            ParamSpecString::builder("hash-method")
                .nick("Hash Method")
                .blurb("Hash algorithm to use (sha1, sha224, sha256, sha384, sha512)")
                .default_value(Some("sha512"))
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
            ParamSpecUInt::builder("substream-length")
                .nick("Substream Length")
                .blurb("Number of buffers per substream (GOP length)")
                .default_value(5)
                .minimum(1)
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
                "Signs video buffers using H.274 DSC SEI metadata",
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

        if let Ok(codec) = VideoCodec::from_caps(incaps) {
            let mut gop_state = self.gop_state.lock().unwrap();
            gop_state.nal_parser = Some(NalParser::new(codec));
            gst::info!(*CAT, imp = self, "Initialized NAL parser for codec: {:?}", codec);
        } else {
            gst::warning!(*CAT, imp = self, "Could not determine codec from caps");
        }

        Ok(())
    }

    fn transform_ip(
        &self,
        buffer: &mut gst::BufferRef,
    ) -> Result<gst::FlowSuccess, gst::FlowError> {
        gst::trace!(*CAT, imp = self, "DscSigner transform_ip called");

        let obj = self.obj();
        let mut gop_state = self.gop_state.lock().unwrap();
        let substream_length = *self.substream_length.read().unwrap();

        let is_first_buffer_in_substream = gop_state.buffers_in_substream == 0;
        
        if is_first_buffer_in_substream {
            gst::info!(*CAT, imp = self, "📝 Starting NEW substream (buffer 1/{})", substream_length);

            let hash_method_byte: u8 = (*self.hash_method.read().unwrap()).into();
            let content_uuid = *self.content_uuid.read().unwrap();
            let public_key_uri = self.public_key_uri.read().unwrap().clone();

            self.add_initialization_meta(buffer, hash_method_byte, &content_uuid, public_key_uri.as_deref())?;

            let hash_method = self.hash_method.read().unwrap().to_openssl();
            let new_dsc_manager = match DscSubstreamManager::new(
                hash_method,
                hash_method_byte,
                content_uuid,
            ) {
                Ok(manager) => manager,
                Err(e) => {
                    gst::error!(*CAT, imp = self, "Failed to create DscSubstreamManager: {}", e);
                    let msg = gst::message::Error::new(
                        gst::CoreError::Failed,
                        &format!("Failed to create DscSubstreamManager: {}", e),
                    );
                    let _ = obj.post_message(msg);
                    return Err(gst::FlowError::Error);
                }
            };

            gop_state.dsc_manager = Some(new_dsc_manager);
            gop_state.gop_started = true;
        }

        self.add_selection_meta(buffer, 0)?;

        let map = buffer.map_readable().map_err(|_| {
            gst::error!(*CAT, imp = self, "Failed to map buffer for reading");
            gst::FlowError::Error
        })?;

        let nal_units_to_hash = if let Some(ref nal_parser) = gop_state.nal_parser {
            match nal_parser.extract_signable_data(&map) {
                Ok(nal_units) => {
                    gst::debug!(*CAT, imp = self, "Extracted {} NAL units from {} raw bytes",
                        nal_units.len(), map.len());
                    nal_units
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

        drop(map);

        if let Some(ref mut dsc_manager) = gop_state.dsc_manager {
            for nal_data in &nal_units_to_hash {
                if let Err(e) = dsc_manager.add_to_substream(0, nal_data) {
                    gst::error!(*CAT, imp = self, "Failed to add NAL to substream: {}", e);
                    return Err(gst::FlowError::Error);
                }

                gst::trace!(*CAT, imp = self, "Added NAL to substream, size: {}", nal_data.len());
            }
        }

        gop_state.buffers_in_substream += 1;
        gst::debug!(*CAT, imp = self, "Buffer {}/{} in current substream", 
            gop_state.buffers_in_substream, substream_length);

        let is_last_buffer_in_substream = gop_state.buffers_in_substream >= substream_length;

        if is_last_buffer_in_substream {
            gst::info!(*CAT, imp = self, "🔐 LAST buffer in substream - creating signature");

            if let Some(ref mut dsc_manager) = gop_state.dsc_manager {
                match dsc_manager.create_data_packet(0) {
                    Ok(data_packet) => {
                        gst::debug!(*CAT, imp = self, "Created data packet: {} bytes", data_packet.len());

                        let signature = self.create_signature_from_data_packet(&data_packet)?;
                        gst::info!(*CAT, imp = self, "✅ Created signature ({} bytes) for substream", signature.len());

                        self.add_verification_meta(buffer, &signature, 0)?;

                        gst::info!(*CAT, imp = self, "✅ Attached verification metadata to last buffer");
                    },
                    Err(e) => {
                        gst::error!(*CAT, imp = self, "Failed to create data packet: {}", e);
                        return Err(gst::FlowError::Error);
                    }
                }
            }

            gop_state.dsc_manager = None;
            gop_state.gop_started = false;
            gop_state.buffers_in_substream = 0;
        }

        Ok(gst::FlowSuccess::Ok)
    }
}

impl DscSigner {
    fn add_initialization_meta(
        &self,
        buffer: &mut gst::BufferRef,
        hash_method_type: u8,
        content_uuid: &Option<[u8; 16]>,
        key_source_uri: Option<&str>,
    ) -> Result<(), gst::FlowError> {
        use gst_video::video_meta::VideoDSCInitializationMeta;

        let key_source_uri_cstring = key_source_uri.map(|uri| CString::new(uri).unwrap());

        let dsc_init = gst_video::ffi::GstH274DigitallySignedContentInitialization {
            id: 0,
            hash_method_type,
            key_retrieval_mode_idc: 0, // URI-based
            use_key_register_idx_flag: 0,
            key_register_idx: 0,
            content_uuid_present_flag: if content_uuid.is_some() { 1 } else { 0 },
            content_uuid: content_uuid.unwrap_or([0u8; 16]),
            num_verification_substreams: 1,
            ref_substream_flag: std::ptr::null_mut(),
            vss_implicit_association_mode_flag: 0,
            signed_content_start_flag: 1,
            sei_signing_flag: 0,
            key_source_uri: key_source_uri_cstring
                .as_ref()
                .map(|s| s.as_ptr() as *mut u8)
                .unwrap_or(std::ptr::null_mut()),
        };

        VideoDSCInitializationMeta::add(buffer, &dsc_init);
        
        gst::info!(*CAT, imp = self, "📝 Added DSC initialization metadata (hash_method: {}, uuid_present: {}, uri: {:?})",
            hash_method_type, dsc_init.content_uuid_present_flag, key_source_uri);

        Ok(())
    }

    fn add_selection_meta(
        &self,
        buffer: &mut gst::BufferRef,
        substream_id: u8,
    ) -> Result<(), gst::FlowError> {
        use gst_video::video_meta::VideoDSCSelectionMeta;

        let dsc_selection = gst_video::ffi::GstH274DigitallySignedContentSelection {
            id: 0,
            verification_substream_id: substream_id,
        };

        VideoDSCSelectionMeta::add(buffer, &dsc_selection);
        
        gst::trace!(*CAT, imp = self, "Added DSC selection metadata (substream: {})", substream_id);

        Ok(())
    }

    fn add_verification_meta(
        &self,
        buffer: &mut gst::BufferRef,
        signature: &[u8],
        substream_id: u8,
    ) -> Result<(), gst::FlowError> {
        use gst_video::video_meta::VideoDSCVerificationMeta;

        // Create GArray for signature
        let signature_array = unsafe {
            let array = glib::ffi::g_array_new(0, 0, std::mem::size_of::<u8>() as u32);
            glib::ffi::g_array_append_vals(
                array,
                signature.as_ptr() as *const _,
                signature.len() as u32,
            );
            array
        };

        let dsc_verification = gst_video::ffi::GstH274DigitallySignedContentVerification {
            id: 0,
            verification_substream_id: substream_id,
            signature_length_in_octets_minus1: (signature.len() - 1) as u32,
            signature: signature_array,
            signed_content_end_flag: 1,
        };

        VideoDSCVerificationMeta::add(buffer, &dsc_verification);

        gst::info!(*CAT, imp = self, "🔐 Added DSC verification metadata (signature: {} bytes)", signature.len());

        Ok(())
    }

    fn create_signature_from_data_packet(
        &self,
        data_packet: &[u8],
    ) -> Result<Vec<u8>, gst::FlowError> {
        let pkey_guard = self.private_key.lock().unwrap();
        let pkey = match pkey_guard.as_ref() {
            Some(k) => k,
            None => {
                gst::error!(*CAT, imp = self, "No private key loaded");
                return Err(gst::FlowError::Error);
            }
        };

        let hash_method = self.hash_method.read().unwrap().to_openssl();

        gst::debug!(*CAT, imp = self, "SIGNER: Data packet first 32: {:02x?}", 
            &data_packet[..std::cmp::min(32, data_packet.len())]);
        gst::debug!(*CAT, imp = self, "SIGNER: Data packet last 32: {:02x?}", 
            &data_packet[data_packet.len().saturating_sub(32)..]);

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
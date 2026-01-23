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
use openssl::x509::X509;

use std::fs;
use std::path::Path;
use std::sync::{Mutex, RwLock};
use std::sync::LazyLock;
use std::collections::HashMap;

use anyhow::Result;

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
    current_hash_method: Option<HashMethod>,
    current_cert_uri: Option<String>,
}

impl Default for GopVerificationState {
    fn default() -> Self {
        Self {
            dsc_manager: None,
            gop_started: false,
            nal_parser: None,
            public_key_cache: HashMap::new(),
            current_hash_method: None,
            current_cert_uri: None,
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

        let final_key_path = if let Some(key_store_path) = &*self.key_store_path.read().unwrap() {
            let key_store_dir = Path::new(key_store_path);

            let uri_lower = cert_uri.to_lowercase();
            let uri_to_process = if uri_lower.starts_with("file://") {
                cert_uri
            } else {
                gst::warning!(*CAT, imp = self, "URI does not start with file://, treating as filename: {}", cert_uri);
                cert_uri
            };

            let mut uri = uri_to_process.to_string();

            if uri.ends_with('/') {
                uri.pop();
            }

            if let Some(last_slash_pos) = uri.rfind('/') {
                let filename = &uri[(last_slash_pos + 1)..];
                let final_path = key_store_dir.join(filename);

                gst::debug!(*CAT, imp = self, "Extracted filename '{}' from URI '{}'", filename, cert_uri);

                final_path.to_string_lossy().to_string()
            } else {
                key_store_dir.join(uri).to_string_lossy().to_string()
            }
        } else {
            cert_uri.to_string()
        };

        gst::debug!(*CAT, imp = self, "Final key path constructed: {}", final_key_path);

        // Load the certificate file and extract the public key
        match fs::read(&final_key_path) {
            Ok(cert_data) => {
                // Parse the X.509 certificate
                match X509::from_pem(&cert_data) {
                    Ok(cert) => {
                        match cert.public_key() {
                            Ok(pkey) => {
                                gop_state.public_key_cache.insert(cert_uri.to_string(), pkey.clone());
                                gst::info!(*CAT, imp = self, "Loaded and cached public key from certificate: {}", final_key_path);
                                Ok(pkey)
                            },
                            Err(e) => {
                                gst::error!(*CAT, imp = self, "Failed to extract public key from certificate {}: {}", final_key_path, e);
                                Err(e)
                            }
                        }
                    },
                    Err(e) => {
                        gst::error!(*CAT, imp = self, "Invalid X.509 certificate at {}: {}", final_key_path, e);
                        Err(e)
                    }
                }
            },
            Err(e) => {
                gst::error!(*CAT, imp = self, "Failed to read certificate file from {}: {}", final_key_path, e);
                Err(openssl::error::ErrorStack::get())
            }
        }
    }

    fn handle_initialization_meta(
        &self,
        init_meta: &gst_video::video_meta::VideoDSCInitializationMeta,
        gop_state: &mut GopVerificationState,
    ) -> Result<(), gst::FlowError> {
        let obj = self.obj();
        let dsc_init = init_meta.dsc_initialization();

        gst::debug!(*CAT, imp = self, "DSC initialization - id: {}, hash_method: {}, key_retrieval_mode: {}",
            dsc_init.id, dsc_init.hash_method_type, dsc_init.key_retrieval_mode_idc);

        let hash_method = HashMethod::from(dsc_init.hash_method_type);
        let openssl_hash_method = hash_method.to_openssl();

        let content_uuid = if dsc_init.content_uuid_present_flag != 0 {
            Some(dsc_init.content_uuid)
        } else {
            None
        };

        let cert_uri = if !dsc_init.key_source_uri.is_null() {
            unsafe {
                std::ffi::CStr::from_ptr(dsc_init.key_source_uri as *const i8)
                    .to_str()
                    .ok()
                    .map(|s| s.to_string())
            }
        } else {
            None
        };

        gop_state.current_hash_method = Some(hash_method);
        gop_state.current_cert_uri = cert_uri.clone();

        let new_dsc_manager = match DscSubstreamManager::new(
            openssl_hash_method,
            dsc_init.hash_method_type,
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

        gst::debug!(*CAT, imp = self, "DSC parameters - hash_method: {:?}, content_uuid_present: {}, num_verification_substreams: {}, cert_uri: {:?}",
            hash_method, dsc_init.content_uuid_present_flag, dsc_init.num_verification_substreams, cert_uri);

        Ok(())
    }

    fn extract_nal_units(
        &self,
        buffer: &gst::BufferRef,
        gop_state: &GopVerificationState,
    ) -> Result<Vec<Vec<u8>>, gst::FlowError> {
        let obj = self.obj();

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

        if let Some(ref nal_parser) = gop_state.nal_parser {
            match nal_parser.extract_signable_data(&map) {
                Ok(nal_units) => {
                    gst::trace!(*CAT, imp = self, "Extracted {} NAL units from {} raw bytes",
                        nal_units.len(), map.len());
                    Ok(nal_units)
                },
                Err(e) => {
                    gst::error!(*CAT, imp = self, "NAL parsing failed: {}", e);
                    let msg = gst::message::Error::new(
                        gst::CoreError::Failed,
                        &format!("NAL parsing failed: {}", e),
                    );
                    let _ = obj.post_message(msg);
                    Err(gst::FlowError::Error)
                }
            }
        } else {
            gst::error!(*CAT, imp = self, "No NAL parser available");
            Err(gst::FlowError::Error)
        }
    }

    fn add_nal_units_to_substream(
        &self,
        nal_units: &[Vec<u8>],
        substream_id: usize,
        gop_state: &mut GopVerificationState,
        is_verification_buffer: bool,
    ) -> Result<(), gst::FlowError> {
        if let Some(ref mut dsc_manager) = gop_state.dsc_manager {
            for nal_data in nal_units {
                if let Err(e) = dsc_manager.add_to_substream(substream_id, nal_data) {
                    gst::error!(*CAT, imp = self, "Failed to add NAL to substream: {}", e);
                    return Err(gst::FlowError::Error);
                }

                if is_verification_buffer {
                    gst::debug!(*CAT, imp = self, "Added verification NAL to substream {}, size: {}, first 32: {:02x?}",
                        substream_id, nal_data.len(), &nal_data[..std::cmp::min(32, nal_data.len())]);
                } else {
                    gst::trace!(*CAT, imp = self, "Added NAL to substream {}, size: {}, first 32: {:02x?}, last 32: {:02x?}",
                        substream_id, nal_data.len(),
                        &nal_data[..std::cmp::min(32, nal_data.len())],
                        &nal_data[nal_data.len().saturating_sub(32)..]);
                }
            }
        }
        Ok(())
    }

    fn get_substream_id(
        &self,
        selection_meta: Option<&gst_video::video_meta::VideoDSCSelectionMeta>,
        default_id: usize,
    ) -> usize {
        if let Some(sel_meta) = selection_meta {
            let substream = sel_meta.dsc_selection().verification_substream_id as usize;
            gst::trace!(*CAT, imp = self, "Found DSC selection metadata, using substream: {}", substream);
            substream
        } else {
            default_id
        }
    }

    fn get_verification_params(
        &self,
        gop_state: &mut GopVerificationState,
    ) -> Result<(HashMethod, openssl::hash::MessageDigest, PKey<openssl::pkey::Public>), gst::FlowError> {
        let obj = self.obj();

        let hash_method = match gop_state.current_hash_method {
            Some(method) => method,
            None => {
                gst::error!(*CAT, imp = self, "No hash method stored from initialization metadata");
                let msg = gst::message::Error::new(
                    gst::CoreError::Failed,
                    "No hash method stored from initialization metadata",
                );
                let _ = obj.post_message(msg);
                return Err(gst::FlowError::Error);
            }
        };

        let openssl_hash_method = hash_method.to_openssl();

        let cert_uri = gop_state.current_cert_uri.clone();

        let pkey = if let Some(uri) = cert_uri {
            match self.load_public_key_from_cert_uri(&uri, gop_state) {
                Ok(key) => key,
                Err(e) => {
                    let msg = gst::message::Error::new(
                        gst::CoreError::Failed,
                        &format!("Failed to load public key from cert_uri '{}': {}", uri, e),
                    );
                    let _ = obj.post_message(msg);
                    return Err(gst::FlowError::Error);
                }
            }
        } else {
            gst::error!(*CAT, imp = self, "No key_source_uri stored from DSC initialization metadata");
            let msg = gst::message::Error::new(
                gst::ResourceError::NotFound,
                "No key_source_uri stored from DSC initialization metadata",
            );
            let _ = obj.post_message(msg);
            return Err(gst::FlowError::Error);
        };

        Ok((hash_method, openssl_hash_method, pkey))
    }

    fn finalize_and_verify(
        &self,
        dsc_verification: &gst_video::ffi::GstH274DigitallySignedContentVerification,
        substream_id: usize,
        gop_state: &mut GopVerificationState,
    ) -> Result<(), gst::FlowError> {
        let obj = self.obj();

        let (hash_method, openssl_hash_method, pkey) = self.get_verification_params(gop_state)?;

        if let Some(ref mut dsc_manager) = gop_state.dsc_manager {
            gst::debug!(*CAT, imp = self, "About to create data packet from accumulated substream data");
            match dsc_manager.create_data_packet(substream_id) {
                Ok(data_packet) => {
                    self.verify_signature(dsc_verification, &data_packet, hash_method, openssl_hash_method, &pkey)?;
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

            gop_state.dsc_manager = None;
            gop_state.gop_started = false;
            gop_state.current_hash_method = None;
            gop_state.current_cert_uri = None;
        } else {
            gst::warning!(*CAT, imp = self, "Received verification metadata but no DSC manager available");
        }

        Ok(())
    }

    fn handle_verification_meta(
        &self,
        buffer: &gst::BufferRef,
        verif_meta: &gst_video::video_meta::VideoDSCVerificationMeta,
        selection_meta: Option<&gst_video::video_meta::VideoDSCSelectionMeta>,
        gop_state: &mut GopVerificationState,
    ) -> Result<gst::FlowSuccess, gst::FlowError> {
        let dsc_verification = verif_meta.dsc_verification();

        let signature_len = unsafe {
            if dsc_verification.signature.is_null() {
                0
            } else {
                (*dsc_verification.signature).len as usize
            }
        };

        gst::debug!(*CAT, imp = self, "Found DSC verification metadata - verifying substream, signature length: {}",
            signature_len);

        if !gop_state.gop_started {
            gst::warning!(*CAT, imp = self, "Received verification metadata but no initialization metadata was received");
            return Ok(gst::FlowSuccess::Ok);
        }

        let nal_units_to_hash = self.extract_nal_units(buffer, gop_state)?;
        let substream_id = self.get_substream_id(selection_meta, dsc_verification.verification_substream_id as usize);

        self.add_nal_units_to_substream(&nal_units_to_hash, substream_id, gop_state, true)?;
        self.finalize_and_verify(&dsc_verification, substream_id, gop_state)?;

        Ok(gst::FlowSuccess::Ok)
    }

    fn verify_signature(
        &self,
        dsc_verification: &gst_video::ffi::GstH274DigitallySignedContentVerification,
        data_packet: &[u8],
        hash_method: HashMethod,
        openssl_hash_method: openssl::hash::MessageDigest,
        pkey: &PKey<openssl::pkey::Public>,
    ) -> Result<(), gst::FlowError> {
        let obj = self.obj();

        gst::debug!(*CAT, imp = self, "Verifying substream data packet: {} bytes with hash method: {:?}", data_packet.len(), hash_method);

        let signature = unsafe {
            if dsc_verification.signature.is_null() {
                &[]
            } else {
                std::slice::from_raw_parts(
                    (*dsc_verification.signature).data as *const u8,
                    (*dsc_verification.signature).len as usize,
                )
            }
        };

        gst::log!(*CAT, imp = self, "Data packet content (first 32 bytes): {:02x?}", &data_packet[..std::cmp::min(32, data_packet.len())]);
        gst::log!(*CAT, imp = self, "Data packet content (last 32 bytes): {:02x?}", &data_packet[data_packet.len().saturating_sub(32)..]);
        gst::log!(*CAT, imp = self, "Signature content (hex): {:02x?}", &signature[..std::cmp::min(32, signature.len())]);
        gst::log!(*CAT, imp = self, "Signature content (dec): {:?}", &signature[..std::cmp::min(32, signature.len())]);
        gst::log!(*CAT, imp = self, "Verifying signature ({} bytes) against substream data packet", signature.len());

        let mut verifier = match Verifier::new(openssl_hash_method, pkey) {
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

        if let Err(e) = verifier.update(data_packet) {
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
                gst::info!(*CAT, imp = self, "✅ Substream signature verified successfully");
                Ok(())
            },
            Ok(false) => {
                gst::error!(*CAT, imp = self, "❌ Substream signature verification FAILED");
                let msg = gst::message::Error::new(
                    gst::CoreError::Failed,
                    "Substream signature verification failed",
                );
                let _ = obj.post_message(msg);
                Err(gst::FlowError::Error)
            },
            Err(e) => {
                gst::error!(*CAT, imp = self, "Error during signature verification: {}", e);
                let msg = gst::message::Error::new(
                    gst::CoreError::Failed,
                    &format!("Error during signature verification: {}", e),
                );
                let _ = obj.post_message(msg);
                Err(gst::FlowError::Error)
            }
        }
    }

    fn handle_regular_buffer(
        &self,
        buffer: &gst::BufferRef,
        selection_meta: Option<&gst_video::video_meta::VideoDSCSelectionMeta>,
        gop_state: &mut GopVerificationState,
    ) -> Result<(), gst::FlowError> {
        if !gop_state.gop_started || gop_state.dsc_manager.is_none() {
            return Ok(());
        }

        let nal_units_to_hash = self.extract_nal_units(buffer, gop_state)?;
        let substream_id = self.get_substream_id(selection_meta, 0);

        self.add_nal_units_to_substream(&nal_units_to_hash, substream_id, gop_state, false)?;

        Ok(())
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
        let mut gop_state = self.gop_state.lock().unwrap();

        let initialization_meta = buffer.meta::<gst_video::video_meta::VideoDSCInitializationMeta>();
        let verification_meta = buffer.meta::<gst_video::video_meta::VideoDSCVerificationMeta>();
        let selection_meta = buffer.meta::<gst_video::video_meta::VideoDSCSelectionMeta>();

        if let Some(init_meta) = initialization_meta {
            self.handle_initialization_meta(&init_meta, &mut gop_state)?;
        }

        if let Some(verif_meta) = verification_meta {
            return self.handle_verification_meta(buffer, &verif_meta, selection_meta.as_deref(), &mut gop_state);
        }

        self.handle_regular_buffer(buffer, selection_meta.as_deref(), &mut gop_state)?;

        Ok(gst::FlowSuccess::Ok)
    }
}

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

#[derive(Default)]
pub struct DscVerifier {
    pub hash_method: RwLock<HashMethod>,
    pub public_key: Mutex<Option<PKey<openssl::pkey::Public>>>,
    pub public_key_path: RwLock<Option<String>>,
    pub enable_verification: RwLock<bool>,
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
            let caps = gst::Caps::new_any();
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
        gst::trace!(*CAT, "DscVerifier transform_ip called");
        let obj = self.obj();
        
        let pkey_guard = self.public_key.lock().unwrap();
        let pkey = match pkey_guard.as_ref() {
            Some(k) => k,
            None => {
                gst::error!(*CAT, "No public key loaded");
                let msg = gst::message::Error::new(
                    gst::ResourceError::NotFound,
                    "No public key loaded",
                );
                let _ = obj.post_message(msg);
                return Err(gst::FlowError::Error);
            }
        };
        
        let hash_method = self.hash_method.read().unwrap().to_openssl();
        
        // Retrieve signature meta
        let sig_meta = match buffer.meta::<SignatureMeta>() {
            Some(meta) => meta,
            None => {
                gst::error!(*CAT, "No signature meta found on buffer");
                let msg = gst::message::Error::new(
                    gst::CoreError::Failed,
                    "No signature meta found on buffer",
                );
                let _ = obj.post_message(msg);
                return Err(gst::FlowError::Error);
            }
        };
        
        let signature = sig_meta.signature();
        
        let map = match buffer.map_readable() {
            Ok(m) => m,
            Err(_) => {
                gst::error!(*CAT, "Failed to map buffer for reading");
                let msg = gst::message::Error::new(
                    gst::CoreError::Failed,
                    "Failed to map buffer for reading",
                );
                let _ = obj.post_message(msg);
                return Err(gst::FlowError::Error);
            }
        };
        
        let mut hasher = match Hasher::new(hash_method) {
            Ok(h) => h,
            Err(e) => {
                gst::error!(*CAT, "Failed to create hasher: {}", e);
                let msg = gst::message::Error::new(
                    gst::CoreError::Failed,
                    &format!("Failed to create hasher: {}", e),
                );
                let _ = obj.post_message(msg);
                return Err(gst::FlowError::Error);
            }
        };
        
        if let Err(e) = hasher.update(&map) {
            gst::error!(*CAT, "Failed to update hasher: {}", e);
            let msg = gst::message::Error::new(
                gst::CoreError::Failed,
                &format!("Failed to update hasher: {}", e),
            );
            let _ = obj.post_message(msg);
            return Err(gst::FlowError::Error);
        }
        
        let digest = match hasher.finish() {
            Ok(d) => d,
            Err(e) => {
                gst::error!(*CAT, "Failed to finish hash: {}", e);
                let msg = gst::message::Error::new(
                    gst::CoreError::Failed,
                    &format!("Failed to finish hash: {}", e),
                );
                let _ = obj.post_message(msg);
                return Err(gst::FlowError::Error);
            }
        };
        
        let mut verifier = match Verifier::new(hash_method, pkey) {
            Ok(v) => v,
            Err(e) => {
                gst::error!(*CAT, "Failed to create verifier: {}", e);
                let msg = gst::message::Error::new(
                    gst::CoreError::Failed,
                    &format!("Failed to create verifier: {}", e),
                );
                let _ = obj.post_message(msg);
                return Err(gst::FlowError::Error);
            }
        };
        
        if let Err(e) = verifier.update(&digest) {
            gst::error!(*CAT, "Failed to update verifier: {}", e);
            let msg = gst::message::Error::new(
                gst::CoreError::Failed,
                &format!("Failed to update verifier: {}", e),
            );
            let _ = obj.post_message(msg);
            return Err(gst::FlowError::Error);
        }
        
        match verifier.verify(signature) {
            Ok(true) => {
                gst::info!(*CAT, "Signature verified successfully");
                Ok(gst::FlowSuccess::Ok)
            },
            Ok(false) => {
                gst::error!(*CAT, "Signature verification failed");
                let msg = gst::message::Error::new(
                    gst::CoreError::Failed,
                    "Signature verification failed",
                );
                let _ = obj.post_message(msg);
                Err(gst::FlowError::Error)
            },
            Err(e) => {
                gst::error!(*CAT, "Error during signature verification: {}", e);
                let msg = gst::message::Error::new(
                    gst::CoreError::Failed,
                    &format!("Error during signature verification: {}", e),
                );
                let _ = obj.post_message(msg);
                Err(gst::FlowError::Error)
            }
        }
    }
}

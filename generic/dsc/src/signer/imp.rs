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
use openssl::sign::Signer;

use std::fs;
use std::sync::Mutex;
use std::sync::RwLock;
use std::sync::LazyLock;

use anyhow::Result;

use crate::signaturemeta::add_signature_meta;
use crate::common::HashMethod;

static CAT: LazyLock<gst::DebugCategory> = LazyLock::new(|| {
    gst::DebugCategory::new(
        "dscsigner",
        gst::DebugColorFlags::empty(),
        Some("GstDscSigner"),
    )
});

#[derive(Default)]
pub struct DscSigner {
    pub hash_method: RwLock<HashMethod>,
    pub private_key: Mutex<Option<PKey<openssl::pkey::Private>>>,
    pub private_key_path: RwLock<Option<String>>,
    pub enable_signing: RwLock<bool>,
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
            _ => {}
        }
    }

    fn property(&self, _id: usize, pspec: &ParamSpec) -> Value {
        match pspec.name() {
            "hash-method" => self.hash_method.read().unwrap().to_string().to_value(),
            "private-key-path" => self.private_key_path.read().unwrap().clone().to_value(),
            "enable-signing" => self.enable_signing.read().unwrap().to_value(),
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

impl BaseTransformImpl for DscSigner {
    const MODE: gst_base::subclass::BaseTransformMode = gst_base::subclass::BaseTransformMode::AlwaysInPlace;
    const PASSTHROUGH_ON_SAME_CAPS: bool = false;
    const TRANSFORM_IP_ON_PASSTHROUGH: bool = false;

    fn transform_ip(
        &self,
        buffer: &mut gst::BufferRef,
    ) -> Result<gst::FlowSuccess, gst::FlowError> {
        gst::trace!(*CAT, "transform_ip called");
        let obj = self.obj();
        // Lock for key and property access for thread safety
        let pkey_guard = self.private_key.lock().unwrap();
        let pkey = match pkey_guard.as_ref() {
            Some(k) => k,
            None => {
                gst::error!(*CAT, "No private key loaded");
                let msg = gst::message::Error::new(
                    gst::ResourceError::NotFound,
                    "No private key loaded",
                );
                let _ = obj.post_message(msg);
                return Err(gst::FlowError::Error);
            }
        };
        let hash_method = self.hash_method.read().unwrap().to_openssl();
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
        gst::debug!(*CAT, "Buffer mapped for reading, size: {}", map.size());
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
        gst::debug!(*CAT, "Digest computed, length: {}", digest.len());
        let mut signer = match Signer::new(hash_method, pkey) {
            Ok(s) => s,
            Err(e) => {
                gst::error!(*CAT, "Failed to create signer: {}", e);
                let msg = gst::message::Error::new(
                    gst::CoreError::Failed,
                    &format!("Failed to create signer: {}", e),
                );
                let _ = obj.post_message(msg);
                return Err(gst::FlowError::Error);
            }
        };
        if let Err(e) = signer.update(&digest) {
            gst::error!(*CAT, "Failed to update signer: {}", e);
            let msg = gst::message::Error::new(
                gst::CoreError::Failed,
                &format!("Failed to update signer: {}", e),
            );
            let _ = obj.post_message(msg);
            return Err(gst::FlowError::Error);
        }
        let signature = match signer.sign_to_vec() {
            Ok(sig) => sig,
            Err(e) => {
                gst::error!(*CAT, "Failed to sign digest: {}", e);
                let msg = gst::message::Error::new(
                    gst::CoreError::Failed,
                    &format!("Failed to sign digest: {}", e),
                );
                let _ = obj.post_message(msg);
                return Err(gst::FlowError::Error);
            }
        };
        drop(map); // Explicitly drop immutable borrow before mut borrow
        // Attach signature as custom meta
        add_signature_meta(buffer, &signature);
        gst::info!(*CAT, "Generated signature of length: {} (meta attached)", signature.len());
        Ok(gst::FlowSuccess::Ok)
    }
}
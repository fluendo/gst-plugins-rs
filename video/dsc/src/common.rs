// Copyright (C) 2025, Fluendo S.A.
//      Author: Diego Nieto <dnieto@fluendo.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v2.0.
// If a copy of the MPL was not distributed with this file, You can obtain one at
// <https://mozilla.org/MPL/2.0/>.
//
// SPDX-License-Identifier: MPL-2.0

use openssl::hash::MessageDigest;

/// Hash method enum shared between signer and verifier
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashMethod {
    Sha1,
    Sha224,
    Sha256,
    Sha384,
    Sha512,
}

impl HashMethod {
    pub fn to_openssl(self) -> MessageDigest {
        match self {
            HashMethod::Sha1 => MessageDigest::sha1(),
            HashMethod::Sha224 => MessageDigest::sha224(),
            HashMethod::Sha256 => MessageDigest::sha256(),
            HashMethod::Sha384 => MessageDigest::sha384(),
            HashMethod::Sha512 => MessageDigest::sha512(),
        }
    }
}

impl Default for HashMethod {
    fn default() -> Self {
        HashMethod::Sha256
    }
}

impl ToString for HashMethod {
    fn to_string(&self) -> String {
        match self {
            HashMethod::Sha1 => "sha1",
            HashMethod::Sha224 => "sha224",
            HashMethod::Sha256 => "sha256",
            HashMethod::Sha384 => "sha384",
            HashMethod::Sha512 => "sha512",
        }.to_string()
    }
}

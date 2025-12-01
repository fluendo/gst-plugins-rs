// Copyright (C) 2025, Fluendo S.A.
//      Author: Diego Nieto <dnieto@fluendo.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v2.0.
// If a copy of the MPL was not distributed with this file, You can obtain one at
// <https://mozilla.org/MPL/2.0/>.
//
// SPDX-License-Identifier: MPL-2.0

use anyhow::{Result, anyhow};
use openssl::hash::Hasher;
use std::sync::LazyLock;

static CAT: LazyLock<gst::DebugCategory> = LazyLock::new(|| {
    gst::DebugCategory::new(
        "dsc-substream",
        gst::DebugColorFlags::empty(),
        Some("DSC Substream Manager")
    )
});

#[derive(Clone)]
pub struct DscSubstream {
    hasher: Option<Hasher>,
}

impl DscSubstream {
    pub fn new(hash_method: openssl::hash::MessageDigest) -> Result<Self> {
        let hasher = Hasher::new(hash_method)?;
        Ok(Self {
            hasher: Some(hasher),
        })
    }

    pub fn add_to_substream(&mut self, data: &[u8]) -> Result<()> {
        if let Some(ref mut hasher) = self.hasher {
            hasher.update(data)?;
        }
        Ok(())
    }

    pub fn finalize(&mut self) -> Result<Vec<u8>> {
        if let Some(mut hasher) = self.hasher.take() {
            let digest = hasher.finish()?;
            gst::debug!(*CAT, "Finalized substream digest: {} bytes", digest.len());
            Ok(digest.to_vec())
        } else {
            Err(anyhow!("Substream already finalized"))
        }
    }
}

pub struct DscSubstreamManager {
    hash_method: openssl::hash::MessageDigest,
    hash_method_byte: u8,
    content_uuid: Option<[u8; 16]>,
    
    substreams: Vec<Option<DscSubstream>>,
    
    last_digest: Option<Vec<u8>>,
}

impl DscSubstreamManager {
    pub fn new(
        hash_method: openssl::hash::MessageDigest, 
        hash_method_byte: u8,
        content_uuid: Option<[u8; 16]>,
    ) -> Self {
        Self {
            hash_method,
            hash_method_byte,
            content_uuid,
            substreams: vec![None],
            last_digest: None,
        }
    }

    pub fn add_to_substream(&mut self, substream_id: usize, data: &[u8]) -> Result<()> {
        if substream_id >= self.substreams.len() {
            self.substreams.resize(substream_id + 1, None);
        }

        if self.substreams[substream_id].is_none() {
            self.substreams[substream_id] = Some(DscSubstream::new(self.hash_method)?);
            gst::debug!(*CAT, "Created new substream {}", substream_id);
        }

        if let Some(ref mut substream) = self.substreams[substream_id] {
            substream.add_to_substream(data)?;
        }

        Ok(())
    }

    // Creates the data packet that will be signed: [ref_digest][current_digest][hash_method][uuid?]
    pub fn create_data_packet(&mut self, substream_id: usize) -> Result<Vec<u8>> {
        let current_digest = self.finalize_substream(substream_id)?;
        
        let mut data_packet = Vec::new();

        // Add reference digest (last GOP's digest or zeros for first GOP)
        if let Some(ref last_digest) = self.last_digest {
            data_packet.extend_from_slice(last_digest);
            gst::debug!(*CAT, "Added reference digest: {} bytes", last_digest.len());
        } else {
            // First GOP: use zero digest of same length as current digest
            let zero_digest = vec![0u8; current_digest.len()];
            data_packet.extend_from_slice(&zero_digest);
            gst::debug!(*CAT, "Added zero reference digest: {} bytes", zero_digest.len());
        }

        data_packet.extend_from_slice(&current_digest);
        gst::debug!(*CAT, "Added current digest: {} bytes", current_digest.len());

        data_packet.push(self.hash_method_byte);
        gst::debug!(*CAT, "Added hash method byte: {}", self.hash_method_byte);

        if let Some(ref uuid) = self.content_uuid {
            data_packet.extend_from_slice(uuid);
            gst::debug!(*CAT, "Added content UUID: 16 bytes");
        }

        gst::info!(*CAT, "Created data packet: {} bytes total", data_packet.len());
        
        self.last_digest = Some(current_digest);

        Ok(data_packet)
    }

    fn finalize_substream(&mut self, substream_id: usize) -> Result<Vec<u8>> {
        if let Some(ref mut substream) = self.substreams.get_mut(substream_id).and_then(|s| s.as_mut()) {
            let digest = substream.finalize()?;
            self.substreams[substream_id] = None;
            Ok(digest)
        } else {
            Err(anyhow!("Substream {} not found or already finalized", substream_id))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_substream_basic_flow() {
        let hash_method = openssl::hash::MessageDigest::sha256();
        let mut manager = DscSubstreamManager::new(hash_method, 2, None, None);

        // Add some test NAL unit data
        let nal_data1 = vec![0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x80]; // SPS-like
        let nal_data2 = vec![0x00, 0x00, 0x00, 0x01, 0x68, 0x48, 0x90]; // PPS-like

        manager.add_to_substream(0, &nal_data1).unwrap();
        manager.add_to_substream(0, &nal_data2).unwrap();

        // Create data packet (this finalizes the substream)
        let data_packet = manager.create_data_packet(0).unwrap();
        
        // Should contain: zero_digest + current_digest + hash_method_byte
        // For SHA256: 32 + 32 + 1 = 65 bytes
        assert_eq!(data_packet.len(), 65);
        assert_eq!(data_packet[64], 2); // hash_method_byte
    }

    #[test]
    fn test_substream_with_uuid() {
        let hash_method = openssl::hash::MessageDigest::sha256();
        let content_uuid = Some([0x12; 16]);
        let mut manager = DscSubstreamManager::new(hash_method, 2, content_uuid, None);

        let nal_data = vec![0x00, 0x00, 0x00, 0x01, 0x67];
        manager.add_to_substream(0, &nal_data).unwrap();

        let data_packet = manager.create_data_packet(0).unwrap();
        
        // Should contain: zero_digest + current_digest + hash_method_byte + uuid
        // For SHA256: 32 + 32 + 1 + 16 = 81 bytes
        assert_eq!(data_packet.len(), 81);
        assert_eq!(&data_packet[65..81], &[0x12; 16]); // UUID at the end
    }
}

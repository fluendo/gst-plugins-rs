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
        if self.hasher.is_none() {
            return Err(anyhow!("Substream hasher not initialized"));
        }

        if let Some(ref mut hasher) = self.hasher {
            gst::trace!(*CAT, "DscSubstream::add_to_substream - adding {} bytes", data.len());
            gst::trace!(*CAT, "  First 32 bytes: {:02x?}", &data[..std::cmp::min(32, data.len())]);
            gst::trace!(*CAT, "  Last 32 bytes: {:02x?}", &data[data.len().saturating_sub(32)..]);
            
            gst::debug!(*CAT, "  → Hashing {} bytes: {:02x?}...", data.len(), &data[..std::cmp::min(16, data.len())]);
            
            hasher.update(data)?;
        }

        Ok(())
    }

    pub fn finalize(&mut self) -> Result<Vec<u8>> {
        if let Some(mut hasher) = self.hasher.take() {
            let digest = hasher.finish()?;
            gst::debug!(*CAT, "Finalized substream digest: {} bytes", digest.len());
            gst::debug!(*CAT, "  Full digest: {:02x?}", digest.as_ref());
            Ok(digest.to_vec())
        } else {
            Err(anyhow!("Substream already finalized"))
        }
    }
}

pub struct DscSubstreamManager {
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
    ) -> Result<Self> {
        // Initialize the first substream
        let substream = DscSubstream::new(hash_method)?;
        
        Ok(Self {
            hash_method_byte,
            content_uuid,
            substreams: vec![Some(substream)],
            last_digest: None,
        })
    }

    pub fn add_to_substream(&mut self, substream_id: usize, data: &[u8]) -> Result<()> {
        if substream_id >= self.substreams.len() {
            return Err(anyhow!("Invalid substream ID: {}", substream_id));
        }

        if self.substreams[substream_id].is_none() {
            return Err(anyhow!("Substream {} not initialized", substream_id));
        }

        gst::trace!(*CAT, "DscSubstreamManager::add_to_substream - substream {}, {} bytes total", 
            substream_id, data.len());

        if let Some(ref mut substream) = self.substreams[substream_id] {
            substream.add_to_substream(data)?;
        }

        Ok(())
    }

    // Creates the data packet that will be signed: [ref_digest][current_digest][hash_method][uuid?]
    pub fn create_data_packet(&mut self, substream_id: usize) -> Result<Vec<u8>> {
        let current_digest = self.finalize_substream(substream_id)?;
        
        gst::debug!(*CAT, "Creating data packet for substream {}", substream_id);
        gst::debug!(*CAT, "Current digest ({} bytes): {:02x?}", current_digest.len(), current_digest);

        let mut data_packet = Vec::new();

        // Reference digest (all 0xFF for first GOP, or last digest from previous GOP)
        let ref_digest = if let Some(ref last) = self.last_digest {
            gst::debug!(*CAT, "Using previous digest as reference ({} bytes)", last.len());
            last.clone()
        } else {
            gst::debug!(*CAT, "First GOP - using all 0xFF as reference digest");
            vec![0xFF; current_digest.len()]
        };
        
        gst::debug!(*CAT, "Reference digest ({} bytes): {:02x?}", ref_digest.len(), &ref_digest[..std::cmp::min(32, ref_digest.len())]);
        data_packet.extend_from_slice(&ref_digest);

        // Current digest
        data_packet.extend_from_slice(&current_digest);

        // Hash method type byte
        data_packet.push(self.hash_method_byte);
        gst::debug!(*CAT, "Hash method byte: {}", self.hash_method_byte);

        // Content UUID (if present)
        if let Some(ref uuid) = self.content_uuid {
            data_packet.extend_from_slice(uuid);
            gst::debug!(*CAT, "Added content UUID: {:02x?}", uuid);
        }

        gst::debug!(*CAT, "Final data packet ({} bytes): first 32: {:02x?}, last 32: {:02x?}", 
            data_packet.len(), 
            &data_packet[..std::cmp::min(32, data_packet.len())],
            &data_packet[data_packet.len().saturating_sub(32)..]);

        // Store current digest for next GOP
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
    fn test_dsc_substream_manager_basic() {
        let hash_method = openssl::hash::MessageDigest::sha256();
        let mut manager = DscSubstreamManager::new(hash_method, 2, None).unwrap();

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
    fn test_dsc_substream_manager_with_content_uuid() {
        let hash_method = openssl::hash::MessageDigest::sha256();
        let content_uuid = Some([0u8; 16]);
        let mut manager = DscSubstreamManager::new(hash_method, 2, content_uuid).unwrap();

        let nal_data = vec![0x00, 0x00, 0x00, 0x01, 0x67];
        manager.add_to_substream(0, &nal_data).unwrap();

        let data_packet = manager.create_data_packet(0).unwrap();

        // Should contain: zero_digest + current_digest + hash_method_byte + uuid
        // For SHA256: 32 + 32 + 1 + 16 = 81 bytes
        assert_eq!(data_packet.len(), 81);
        assert_eq!(&data_packet[65..81], &[0u8; 16]); // UUID at the end
    }
}

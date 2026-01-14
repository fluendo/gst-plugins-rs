// Copyright (C) 2025, Fluendo S.A.
//      Author: Diego Nieto <dnieto@fluendo.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v2.0.
// If a copy of the MPL--2.0 was not distributed with this file, You can obtain one at
// <https://mozilla.org/MPL/2.0/>.
//
// SPDX-License-Identifier: MPL-2.0

use anyhow::{Result, bail};
use std::sync::LazyLock;

static CAT: LazyLock<gst::DebugCategory> = LazyLock::new(|| {
    gst::DebugCategory::new(
        "nal-parser",
        gst::DebugColorFlags::empty(),
        Some("NAL Unit Parser")
    )
});

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VideoCodec {
    H264,
    H265,
    H266,
}

impl VideoCodec {
    pub fn from_caps(caps: &gst::Caps) -> Result<Self> {
        if let Some(structure) = caps.structure(0) {
            let name = structure.name();
            if name == "video/x-h264" {
                Ok(VideoCodec::H264)
            } else if name == "video/x-h265" {
                Ok(VideoCodec::H265)
            } else if name == "video/x-h266" {
                Ok(VideoCodec::H266)
            } else {
                bail!("Unsupported codec in caps")
            }
        } else {
            bail!("No structure in caps")
        }
    }
}

pub struct NalParser {
    codec: VideoCodec,
}

impl NalParser {
    pub fn new(codec: VideoCodec) -> Self {
        gst::info!(*CAT, "Created NAL parser for {:?} (using manual parsing)", codec);
        Self { codec }
    }

    pub fn extract_signable_data(&self, data: &[u8]) -> Result<Vec<Vec<u8>>> {
        let mut nal_units = Vec::new();
        let mut start = 0;

        while start < data.len() {
            let start_code_len = if data[start..].starts_with(&[0, 0, 0, 1]) {
                4
            } else if data[start..].starts_with(&[0, 0, 1]) {
                3
            } else {
                bail!("Invalid NAL unit - no start code at position {}", start);
            };

            let mut end = start + start_code_len;
            while end < data.len() {
                if (end + 4 <= data.len() && &data[end..end + 4] == &[0, 0, 0, 1]) ||
                   (end + 3 <= data.len() && &data[end..end + 3] == &[0, 0, 1]) {
                    break;
                }
                end += 1;
            }

            let nal_start = start + start_code_len;
            let nal_data = &data[nal_start..end];
            
            let nal_type = self.extract_nal_type_from_header(nal_data)?;

            if self.should_include_nal(nal_type) {
                gst::info!(*CAT, "NAL_TRACE: Including NAL type {} ({} bytes) - first 32: {:02x?}",
                    nal_type, nal_data.len(),
                    &nal_data[..std::cmp::min(32, nal_data.len())]);
                
                nal_units.push(nal_data.to_vec());
            }

            start = end;
        }

        gst::info!(*CAT, "NAL_TRACE: Total signable data: {} NAL units extracted", nal_units.len());
        Ok(nal_units)
    }

    fn extract_nal_type_from_header(&self, nal_data: &[u8]) -> Result<u8> {
        if nal_data.is_empty() {
            bail!("Empty NAL data");
        }

        let nal_type = match self.codec {
            VideoCodec::H264 => {
                nal_data[0] & 0x1F
            },
            VideoCodec::H265 => {
                if nal_data.len() < 2 {
                    bail!("H.265 NAL header too short");
                }
                (nal_data[0] >> 1) & 0x3F
            },
            VideoCodec::H266 => {
                if nal_data.len() < 2 {
                    bail!("H.266 NAL header too short");
                }
                (nal_data[1] >> 3) & 0x1F
            },
        };

        Ok(nal_type)
    }

    fn should_include_nal(&self, nal_type: u8) -> bool {
        match self.codec {
            VideoCodec::H264 => self.should_include_h264_nal(nal_type),
            VideoCodec::H265 => self.should_include_h265_nal(nal_type),
            VideoCodec::H266 => self.should_include_h266_nal(nal_type),
        }
    }

    fn should_include_h264_nal(&self, nal_type: u8) -> bool {
        let nal_unit_type = nal_type & 0x1F;
        match nal_unit_type {
            // VCL NAL units (actual video content)
            1..=5 => true,    // Coded slice units

            // Essential Non-VCL NAL units
            7 => true,        // SPS
            8 => true,        // PPS

            // Excluded Non-VCL units
            6 => false,       // SEI
            9 => false,       // AUD
            12 => false,      // Filler
            _ => false,
        }
    }

    fn should_include_h265_nal(&self, nal_type: u8) -> bool {
        let nal_unit_type = (nal_type >> 1) & 0x3F;
        match nal_unit_type {
            // VCL NAL units
            0..=31 => true,   // Coded slice units (VCL)

            // Essential Non-VCL parameter sets
            33 => true,       // SPS
            34 => true,       // PPS

            // Excluded Non-VCL units
            32 => false,      // VPS
            35 => false,      // AUD
            38 => false,      // Filler
            39 => false,      // PREFIX_SEI
            40 => false,      // SUFFIX_SEI
            _ => false,
        }
    }

    fn should_include_h266_nal(&self, nal_type: u8) -> bool {
        match nal_type {
            // VCL NAL units (0-12)
            0..=12 => true,

            // Non-VCL parameter sets (INCLUDE)
            15 => true,  // VPS
            16 => true,  // SPS  
            17 => true,  // PPS
            19 => true,  // APS (Prefix)
            
            // Non-VCL units to EXCLUDE
            13 => false,  // DCI
            14 => false,  // OPI  
            18 => false,  // Picture Header
            20 => false,  // AUD
            21 => false,  // EOS
            22 => false,  // EOB
            23 => false,  // PREFIX_SEI
            24 => false,  // SUFFIX_SEI
            25 => false,  // FD (Filler Data)
            
            _ => {
                gst::warning!(*CAT, "Unknown H.266 NAL type: {}", nal_type);
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init() {
        use std::sync::Once;
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            gst::init().unwrap();
        });
    }

    #[test]
    fn test_h264_nal_parsing() {
        init();
        
        // Simple H.264 NAL unit with start code
        let data = vec![
            0x00, 0x00, 0x00, 0x01, // Start code
            0x67,                    // SPS NAL header (type 7)
            0x42, 0x80, 0x1e,       // Some SPS data
        ];

        let parser = NalParser::new(VideoCodec::H264);
        let nal_units = parser.parse_h264_nal_units(&data).unwrap();

        assert_eq!(nal_units.len(), 1);
        assert_eq!(nal_units[0].nal_type, 7); // SPS
        assert_eq!(nal_units[0].complete_data, vec![0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x80, 0x1e]);
    }

    #[test]
    fn test_h266_support() {
        init();
        
        let caps = gst::Caps::builder("video/x-h266").build();
        let codec = VideoCodec::from_caps(&caps).unwrap();
        assert_eq!(codec, VideoCodec::H266);

        let parser = NalParser::new(VideoCodec::H266);
        assert_eq!(parser.codec_name(), "H.266");
    }
}

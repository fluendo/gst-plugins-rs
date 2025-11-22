// Copyright (C) 2025, Fluendo S.A.
//      Author: Diego Nieto <dnieto@fluendo.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v2.0.
// If a copy of the MPL--2.0 was not distributed with this file, You can obtain one at
// <https://mozilla.org/MPL/2.0/>.
//
// SPDX-License-Identifier: MPL-2.0

use anyhow::{Result, anyhow};
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
    pub fn from_caps(caps: &gst::CapsRef) -> Result<Self> {
        let structure = caps.structure(0).ok_or_else(|| anyhow!("No structure in caps"))?;
        let media_type = structure.name();
        
        match media_type.as_str() {
            "video/x-h264" => Ok(VideoCodec::H264),
            "video/x-h265" => Ok(VideoCodec::H265), 
            "video/x-h266" => Ok(VideoCodec::H266),
            _ => Err(anyhow!("Unsupported codec: {}", media_type)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct NalUnit {
    pub nal_type: u8,
    pub complete_data: Vec<u8>,
}

pub struct NalParser {
    codec: VideoCodec,
}

impl NalParser {
    pub fn new(codec: VideoCodec) -> Self {
        gst::info!(*CAT, "Created NAL parser for {:?} (using manual parsing)", codec);
        Self { codec }
    }

    /// Extract NAL units suitable for signing/verification
    pub fn extract_signable_data(&self, buffer: &gst::BufferMap<gst::buffer::Readable>) -> Result<Vec<u8>> {
        let data = buffer.as_slice();
        let nal_units = self.parse_nal_units(data)?;
        let mut signable_data = Vec::new();

        let mut included_count = 0;
        for nal_unit in &nal_units {
            if self.should_include_in_signature(nal_unit) {
                signable_data.extend_from_slice(&nal_unit.complete_data);
                included_count += 1;
                
                gst::trace!(*CAT, "Including {} NAL unit type {} ({} bytes) in signature data", 
                    self.codec_name(), nal_unit.nal_type, nal_unit.complete_data.len());
            }
        }

        gst::debug!(*CAT, "Extracted {} bytes of signable NAL unit data from {} NAL units ({} included)", 
            signable_data.len(), nal_units.len(), included_count);

        Ok(signable_data)
    }

    fn codec_name(&self) -> &'static str {
        match self.codec {
            VideoCodec::H264 => "H.264",
            VideoCodec::H265 => "H.265",
            VideoCodec::H266 => "H.266",
        }
    }

    fn parse_nal_units(&self, data: &[u8]) -> Result<Vec<NalUnit>> {
        match self.codec {
            VideoCodec::H264 => self.parse_h264_nal_units(data),
            VideoCodec::H265 => self.parse_h265_nal_units(data),
            VideoCodec::H266 => self.parse_h266_nal_units(data),
        }
    }

    fn should_include_in_signature(&self, nal_unit: &NalUnit) -> bool {
        match self.codec {
            VideoCodec::H264 => self.should_include_h264_nal(nal_unit.nal_type),
            VideoCodec::H265 => self.should_include_h265_nal(nal_unit.nal_type),
            VideoCodec::H266 => self.should_include_h266_nal(nal_unit.nal_type),
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
            
            // Excluded Non-VCL units (VTM doesn't sign these)
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
        let nal_unit_type = (nal_type >> 3) & 0x1F;
        match nal_unit_type {
            // VCL NAL units
            0..=12 => true,   // Coded slice units (VCL)
            
            // Essential Non-VCL parameter sets
            17 => true,       // SPS
            18 => true,       // PPS
            19 => true,       // APS
            // Picture Header handled separately
            
            // Excluded Non-VCL units (same as VTM)
            13 => false,      // DCI
            14 => false,      // OPI
            15 => false,      // VPS
            20 => false,      // AUD
            23 => false,      // PREFIX_SEI
            24 => false,      // SUFFIX_SEI
            25 => false,      // Filler
            _ => false,
        }
    }

    fn parse_h264_nal_units(&self, data: &[u8]) -> Result<Vec<NalUnit>> {
        let mut nal_units = Vec::new();
        let mut pos = 0;

        while pos < data.len() {
            // Look for start code (0x000001 or 0x00000001)
            let start_code_len = if pos + 4 <= data.len() && 
                data[pos] == 0x00 && data[pos + 1] == 0x00 && 
                data[pos + 2] == 0x00 && data[pos + 3] == 0x01 {
                4
            } else if pos + 3 <= data.len() && 
                data[pos] == 0x00 && data[pos + 1] == 0x00 && data[pos + 2] == 0x01 {
                3  
            } else {
                pos += 1;
                continue;
            };

            let nal_start = pos + start_code_len;
            if nal_start >= data.len() {
                break;
            }

            // Find next start code
            let mut nal_end = self.find_next_start_code(data, nal_start + 1);
            if nal_end == data.len() {
                nal_end = data.len();
            }

            // Extract NAL type from header byte
            let nal_header = data[nal_start];
            let nal_type = nal_header & 0x1F;
            
            // Store complete NAL unit (start code + header + payload)
            // This matches what VTM's writeNaluWithHeader() produces
            let complete_data = data[pos..nal_end].to_vec();

            nal_units.push(NalUnit {
                nal_type,
                complete_data,
            });

            pos = nal_end;
        }

        gst::debug!(*CAT, "Parsed {} H.264 NAL units", nal_units.len());
        Ok(nal_units)
    }

    fn parse_h265_nal_units(&self, data: &[u8]) -> Result<Vec<NalUnit>> {
        let mut nal_units = Vec::new();
        let mut pos = 0;

        while pos < data.len() {
            let start_code_len = self.find_start_code(data, pos);
            if start_code_len == 0 {
                pos += 1;
                continue;
            }

            let nal_start = pos + start_code_len;
            if nal_start + 1 >= data.len() {
                break;
            }

            let mut nal_end = self.find_next_start_code(data, nal_start + 2);
            if nal_end == data.len() {
                nal_end = data.len();
            }

            // Extract NAL type from H.265 header (2 bytes)
            if nal_start + 2 > data.len() {
                break;
            }

            let nal_header_bytes = &data[nal_start..nal_start + 2];
            let nal_type = (nal_header_bytes[0] >> 1) & 0x3F;
            
            // Store complete NAL unit - this is what VTM signs
            let complete_data = data[pos..nal_end].to_vec();

            nal_units.push(NalUnit {
                nal_type,
                complete_data,
            });

            pos = nal_end;
        }

        gst::debug!(*CAT, "Parsed {} H.265 NAL units", nal_units.len());
        Ok(nal_units)
    }

    fn parse_h266_nal_units(&self, data: &[u8]) -> Result<Vec<NalUnit>> {
        let mut nal_units = Vec::new();
        let mut pos = 0;

        while pos < data.len() {
            let start_code_len = self.find_start_code(data, pos);
            if start_code_len == 0 {
                pos += 1;
                continue;
            }

            let nal_start = pos + start_code_len;
            if nal_start + 1 >= data.len() {
                break;
            }

            let mut nal_end = self.find_next_start_code(data, nal_start + 2);
            if nal_end == data.len() {
                nal_end = data.len();
            }

            // Extract NAL type from H.266 header
            if nal_start + 2 > data.len() {
                break;
            }

            let nal_header_bytes = &data[nal_start..nal_start + 2];
            let nal_type = (nal_header_bytes[0] >> 3) & 0x1F;
            
            // Store complete NAL unit
            let complete_data = data[pos..nal_end].to_vec();

            nal_units.push(NalUnit {
                nal_type,
                complete_data,
            });

            pos = nal_end;
        }

        gst::debug!(*CAT, "Parsed {} H.266 NAL units", nal_units.len());
        Ok(nal_units)
    }

    fn find_start_code(&self, data: &[u8], pos: usize) -> usize {
        if pos + 4 <= data.len() && 
            data[pos] == 0x00 && data[pos + 1] == 0x00 && 
            data[pos + 2] == 0x00 && data[pos + 3] == 0x01 {
            4
        } else if pos + 3 <= data.len() && 
            data[pos] == 0x00 && data[pos + 1] == 0x00 && data[pos + 2] == 0x01 {
            3  
        } else {
            0
        }
    }

    fn find_next_start_code(&self, data: &[u8], start_pos: usize) -> usize {
        let mut pos = start_pos;
        while pos + 2 < data.len() {
            if data[pos] == 0x00 && data[pos + 1] == 0x00 {
                if (pos + 3 < data.len() && data[pos + 2] == 0x00 && data[pos + 3] == 0x01) ||
                   data[pos + 2] == 0x01 {
                    return pos;
                }
            }
            pos += 1;
        }
        data.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_h264_nal_parsing() {
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
        let caps = gst::Caps::builder("video/x-h266").build();
        let codec = VideoCodec::from_caps(&caps).unwrap();
        assert_eq!(codec, VideoCodec::H266);
        
        let parser = NalParser::new(VideoCodec::H266);
        assert_eq!(parser.codec_name(), "H.266");
    }
}

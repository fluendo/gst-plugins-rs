// Copyright (C) 2026
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v2.0.
// If a copy of the MPL was not distributed with this file, You can obtain one at
// <https://mozilla.org/MPL/2.0/>.
//
// SPDX-License-Identifier: MPL-2.0

use anyhow::{Context, Error, bail};
use bitstream_io::{BitRead, BitWrite, FromBitStream, ToBitStream};

#[derive(Debug, Clone)]
pub struct Ac4Toc {
    pub bitstream_version: u8,
    pub b_wait_frames: bool,
    pub wait_frames: u8,
    pub br_code: u8,
    pub fs_index: u8,
    pub frame_rate_index: u8,
    pub b_iframe_global: bool,
    pub b_single_presentation: bool,
    pub b_more_presentations: Option<bool>,
    pub b_payload_base: bool,
    pub payload_base: u8,
    pub b_program_id: bool,
    pub short_program_id: u16,
    pub b_program_uuid_present: bool,
    pub program_uuid: [u8; 16],
    // FIXME: pub presentations: Vec<PresentationV1Info>,
}

fn read_presentation_version<R: bitstream_io::BitRead + ?Sized>(r: &mut R) -> Result<u32, Error> {
    let mut val = 0;
    while r.read_bit()? {
        val += 1;
    }
    Ok(val)
}
fn read_variable_bits<R: BitRead + ?Sized>(r: &mut R, n_bits: u32) -> Result<u8, Error> {
    let mut value = 0;
    loop {
        value += r.read_var::<u8>(n_bits).context("n_bits")?;
        let b_read_more: bool = r.read_bit().context("b_read_more")?;
        if b_read_more {
            value <<= n_bits;
            value += 1 << n_bits;
        } else {
            break;
        }
    }
    Ok(value)
}

// Read bits from ac4 toc
impl FromBitStream for Ac4Toc {
    type Error = Error;
    fn from_reader<R: bitstream_io::BitRead + ?Sized>(r: &mut R) -> Result<Self, Self::Error>
    where
        Self: Sized,
    {
        let mut bitstream_version = r.read::<2, u8>().context("bitstream_version")?;
        if bitstream_version == 3 {
            bitstream_version += read_variable_bits(r, 2).context("bits  ext")?;
        }

        r.skip(10).context("sequence_number")?;

        let b_wait_frames = r.read_bit().context("b_wait_frames")?;
        let mut wait_frames = 0u8;
        let mut br_code = 0u8;

        if b_wait_frames {
            wait_frames = r.read::<3, u8>().context("wait_frames")?;
            if wait_frames > 0 {
                br_code = r.read::<2, u8>().context("br_code")?;
            }
        }

        let fs_index = r.read::<1, u8>().context("fs_index")?;

        let frame_rate_index = r.read::<4, u8>().context("frame_rate_index")?;

        let b_iframe_global = r.read_bit().context("b_iframe_global")?;

        let b_single_presentation = r.read_bit().context("b_single_presentation")?;
        let mut b_more_presentations = None;
        let n_presentations = if b_single_presentation {
            1
        } else {
            let more = r.read_bit().context("b_more_presentations")?;
            b_more_presentations = Some(more);
            if more {
                read_variable_bits(r, 2)? + 2
            } else {
                0
            }
        };
        let b_payload_base = r.read_bit().context("b_payload_base")?;
        let mut payload_base = 0;
        if b_payload_base {
            let payload_base_minus1 = r.read::<5, u8>().context("payload_base_minus1")?;
            payload_base = payload_base_minus1 + 1;
            if payload_base == 0x20u8 {
                payload_base += read_variable_bits(r, 3)?;
            }
        }

        if bitstream_version <= 1 {
            bail!("AC-4 bitstream_version {bitstream_version} is not supported yet");
        }

        let b_program_id = r.read_bit().context("b_program_id")?;
        let mut short_program_id = 0u16;
        let mut b_program_uuid_present = false;
        let mut program_uuid = [0u8; 16];
        if b_program_id {
            short_program_id = r.read::<16, u16>().context("short_program_id")?;
            b_program_uuid_present = r.read_bit().context("b_program_uuid_present")?;
            if b_program_uuid_present {
                program_uuid = r.read::<128, u128>().context("program_uuid")?.to_be_bytes();
            }
        }
        // FIXME: use for parsing ac4_presentation_v1_info() once implemented
        let _ = n_presentations;
        Ok(Ac4Toc {
            bitstream_version,
            b_wait_frames,
            wait_frames,
            br_code,
            fs_index,
            frame_rate_index,
            b_iframe_global,
            b_single_presentation,
            b_more_presentations,
            b_payload_base,
            payload_base,
            b_program_id,
            short_program_id,
            b_program_uuid_present,
            program_uuid,
        })
    }
}

// Unused structs contained in toc, they would be used in the full parsing
#[derive(Debug, Clone)]
pub(crate) struct Ac4PresentationInfo {
    pub b_single_substream: bool,
    pub presentation_config: Option<u8>,
    pub md_compat: Option<u8>,
    pub b_presentation_id: Option<bool>,
    pub b_hsf_ext: Option<bool>,
    pub b_pre_virtualized: Option<bool>,
    pub b_add_emdf_substreams: Option<bool>,
    pub n_add_emdf_substreams: Option<u8>,
}

#[derive(Debug, Clone)]
pub(crate) struct PresentationV1Info {
    pub b_single_substream_group: bool, 
    pub presentation_config: u8,
    pub presentation_version: u8, 
    pub md_compat: u8,
    pub b_presentation_id: bool,
    pub presentation_id: u8,
    pub dsi_frame_rate_multiply_info: u8,
    pub dis_frame_rate_fraction_info: u8,
    pub presentation_emdf_version: u8,
    pub presentation_key_id: u16,
    pub b_presentation_filter: bool,
    pub b_enable_presentation: bool,
    pub b_multi_pid: bool,
    pub b_pre_virtualized: bool,
    pub b_add_emdf_substreames: bool,
    pub n_add_emdf_substreames: u8,
    // Todo: Probably need to add more fields
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Ac4SubstreamDsi {
    pub dsi_sf_multiplier: u8,
    pub b_substream_bitrate_indicator: bool,
    pub substream_bitrate_indicator: Option<u8>,
    pub dsi_substream_channel_groups: u32,
    // Todo: add more fields
}

#[derive(Debug, Clone)]
pub(crate) struct Ac4SubstreamGroupDsi {
    pub b_substreams_present: bool,
    pub b_hsf_ext: bool,
    pub b_channel_coded: bool,
    pub substream: Vec<Ac4SubstreamDsi>,
    pub b_content_type: bool,
    // Todo: add more fields
}


#[derive(Debug, Clone, Copy)]
pub(crate) struct Ac4BitrateDsi {
    bit_rate_mode: u8,
    bit_rate: u32,
    bit_rate_precision: u32,
}
// bitrate is not in toc, so set the default to unknown
impl Ac4BitrateDsi {
    pub(crate) const UNKNOWN: Self = Self {
        bit_rate_mode: 0,
        bit_rate: 0,
        bit_rate_precision: 0xFFFF_FFFF,
    };
}

impl ToBitStream for Ac4BitrateDsi {
    type Error = Error;

    fn to_writer<W: bitstream_io::BitWrite + ?Sized>(&self, w: &mut W) -> Result<(), Self::Error>
    where
        Self: Sized,
    {
        w.write::<2, u8>(self.bit_rate_mode)
            .context("bit_rate_mode")?;
        w.write::<32, u32>(self.bit_rate).context("bit_rate")?;
        w.write::<32, u32>(self.bit_rate_precision)
            .context("bit_rate_precision")?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct Dac4 {
    pub bitstream_version: u8,
    pub fs_index: u8,
    pub frame_rate_index: u8,
    pub n_presentations: u16,
    pub b_program_id: bool,
    pub b_program_uuid_present: bool,
    pub short_program_id: u16,
    pub program_uuid: [u8; 16],
    pub bitrate: Ac4BitrateDsi,
}

impl Dac4 {
    pub(crate) fn from_toc(toc: &Ac4Toc) -> Result<Self, Error> {
        Ok(Self {
            bitstream_version: toc.bitstream_version,
            fs_index: toc.fs_index,
            frame_rate_index: toc.frame_rate_index,
            // FIXME: use toc.n_presentations once the per-presentation DSI is written
            n_presentations: 0,
            b_program_id: toc.b_program_id,
            b_program_uuid_present: toc.b_program_uuid_present,
            short_program_id: toc.short_program_id,
            program_uuid: toc.program_uuid,
            bitrate: Ac4BitrateDsi::UNKNOWN,
        })
    }

    fn write_dsi<W: BitWrite + ?Sized>(&self, w: &mut W) -> Result<(), Error> {
        w.write::<3, u8>(1).context("ac4_dsi_version")?;
        w.write::<7, u8>(self.bitstream_version)
            .context("bitstream_version")?;
        w.write::<1, u8>(self.fs_index).context("fs_index")?;
        w.write::<4, u8>(self.frame_rate_index)
            .context("frame_rate_index")?;
        // FIXME: write the real n_presentations + per-presentation DSI (E.6.1)
        w.write::<9, u16>(self.n_presentations)
            .context("n_presentations")?;
        if self.bitstream_version > 1 {
            w.write_bit(self.b_program_id).context("b_program_id")?;
            if self.b_program_id {
                w.write::<16, u16>(self.short_program_id)
                    .context("short_program_id")?;
                w.write_bit(self.b_program_uuid_present).context("b_uuid")?;
                if self.b_program_uuid_present {
                    for b in self.program_uuid {
                        w.write::<8, u8>(b).context("program_uuid")?;
                    }
                }
            }
            self.bitrate.to_writer(w).context("ac4_bitrate_dsi")?;
            w.byte_align().context("byte_align")?;
        }
        // Todo: write presentation_version, pres_bytes, add_pres_bytes ... 
        Ok(())
    }
}

impl ToBitStream for Dac4 {
    type Error = Error;

    fn to_writer<W: BitWrite + ?Sized>(&self, w: &mut W) -> Result<(), Self::Error>
    where
        Self: Sized,
    {
        // Write body first because the size field depends on the DSI length
        let mut body = Vec::with_capacity(12);
        {
            let mut bw = bitstream_io::BitWriter::endian(&mut body, bitstream_io::BigEndian);
            self.write_dsi(&mut bw)?;
            bw.byte_align().context("byte_align")?;
        }

        w.write_from::<u32>(8 + body.len() as u32).context("size")?;
        w.write_bytes(b"dac4").context("type")?;
        w.write_bytes(&body).context("bits")?;
        Ok(())
    }
}

// Copyright (C) 2026 Pablo García <pgarcia@fluendo.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v2.0.
// If a copy of the MPL was not distributed with this file, You can obtain one at
// <https://mozilla.org/MPL/2.0/>.
//
// SPDX-License-Identifier: MPL-2.0

use anyhow::{Context, Error, bail};
use bitstream_io::{BigEndian, BitRead, BitReader};

// MHAS packet types, as defined in ISO/IEC 23008-3 Section 14.3.1.
const MHAS_PAC_TYP_MPEGH3DACFG: u32 = 1;
const MHAS_PAC_TYP_MPEGH3DAFRAME: u32 = 2;

/// MPEG-H 3D audio configuration extracted from the in-band
/// `PACTYP_MPEGH3DACFG` MHAS packet.
pub(crate) struct MhaConfig {
    pub(crate) profile_level_indication: u8,
    pub(crate) reference_channel_layout: u8,
    pub(crate) mpegh3da_config: Vec<u8>,
}

struct MhasPacketHeader {
    packet_type: u32,
    packet_length: u32,
}

// TODO: This MHAS parsing and the parsing of the MPEG-H 3D audio configuration
// (mpegh3daConfig) below only exists to extract the information needed for the
// `mhaC` box. This is the responsibility of the MPEG-H 3D audio parser element
// and must be removed from here (and moved there) once such an element exists.

/// Read a single MHAS packet header - See Section 14.2.1 at ISO/IEC 23008-3:202.
/// Each field is encoded as an escape value: a base number of bits, followed by
/// one or two additional escape levels when the field is at its maximum value.
fn read_packet_header<R: BitRead>(r: &mut R) -> Result<MhasPacketHeader, Error> {
    // MHASPacketType: escape 3 bits first, then 8 and then 8 again.
    let mut packet_type = r.read::<3, u8>().context("packet_type")? as u32;
    // if packet_type == 7 (binary 111), it doesn't fit, so continue reading
    if packet_type == (1 << 3) - 1 {
        let escape = r.read::<8, u8>().context("packet_type escape")? as u32;
        packet_type += escape;
        if escape == (1 << 8) - 1 {
            packet_type += r.read::<8, u8>().context("packet_type second escape")? as u32;
        }
    }

    // MHASPacketLabel: escape 2 bits first, then 8 and finally 32.
    let mut packet_label = r.read::<2, u8>().context("packet_label")? as u32;
    if packet_label == (1 << 2) - 1 {
        let escape = r.read::<8, u8>().context("packet_label escape")? as u32;
        packet_label += escape;
        if escape == (1 << 8) - 1 {
            packet_label += r.read::<32, u32>().context("packet_label second escape")?;
        }
    }

    // MHASPacketLenght: 11, 24 and 24
    let mut packet_length = r.read::<11, u16>().context("packet_length")? as u32;
    if packet_length == (1 << 11) - 1 {
        let escape = r.read::<24, u32>().context("packet_length escape")?;
        packet_length += escape;
        if escape == (1 << 24) - 1 {
            packet_length += r.read::<24, u32>().context("packet_length second escape")?;
        }
    }

    let _ = packet_label; // Ignore it, we don't need it for later

    Ok(MhasPacketHeader {
        packet_type,
        packet_length,
    })
}

/// Skip the payload of a non-configuration MHAS packet.
fn skip_packet<R: BitRead>(r: &mut R, packet_length: u32) -> Result<(), Error> {
    if r.byte_aligned() {
        let mut buf = vec![0u8; packet_length as usize];
        r.read_bytes(&mut buf)
            .context("Reading MHAS packet payload")?;
    } else {
        r.skip(packet_length.saturating_mul(8))
            .context("Skipping MHAS packet payload")?;
    }
    Ok(())
}

/// Parse the reference channel layout from the `SpeakerConfig3d()` syntax
/// element.
fn parse_reference_channel_layout<R: BitRead>(r: &mut R) -> Result<u8, Error> {
    let speaker_layout_type = r.read::<2, u8>().context("speakerLayoutType")?;

    // Only the CICP layout (speakerLayoutType == 0) maps to a channel
    // configuration index as defined in ISO/IEC 23091-3. All other layout types
    // carry an explicit speaker list and do not have a CICP index.
    if speaker_layout_type == 0 {
        r.read::<6, u8>().context("CICPspeakerLayoutIdx")
    } else {
        Ok(0)
    }
}

/// Parse the leading part of the MPEG-H 3D audio configuration (mpegh3daConfig)
/// to obtain the profile/level indication and the reference channel layout.
fn parse_config(config: &[u8]) -> Result<MhaConfig, Error> {
    let mut r = BitReader::endian(std::io::Cursor::new(config), BigEndian);

    let profile_level_indication = r
        .read::<8, u8>()
        .context("mpegh3daProfileLevelIndication")?;

    let usac_sampling_frequency_index = r.read::<5, u8>().context("usacSamplingFrequencyIndex")?;
    if usac_sampling_frequency_index == 0x1f {
        let _usac_sampling_frequency = r.read::<24, u32>().context("usacSamplingFrequency")?;
    }

    let _core_sbr_frame_length_index = r.read::<3, u8>().context("coreSbrFrameLengthIndex")?;
    let _cfg_reserved = r.read::<1, u8>().context("cfg_reserved")?;
    let _receiver_delay_compensation = r.read::<1, u8>().context("receiverDelayCompensation")?;

    let reference_channel_layout = parse_reference_channel_layout(&mut r)?;

    Ok(MhaConfig {
        profile_level_indication,
        reference_channel_layout,
        mpegh3da_config: config.to_vec(),
    })
}

/// Parse the first MHAS buffer of an MPEG-H stream and return the configuration
/// carried by the `PACTYP_MPEGH3DACFG` packet, if any, before the first
/// `PACTYP_MPEGH3DAFRAME` packet.
pub(crate) fn parse_mhas_config(buffer: &gst::BufferRef) -> Result<Option<MhaConfig>, Error> {
    let map = buffer
        .map_readable()
        .context("Mapping MPEG-H buffer readable")?;

    parse_mhas_config_from_slice(map.as_slice())
}

fn parse_mhas_config_from_slice(slice: &[u8]) -> Result<Option<MhaConfig>, Error> {
    let mut r = BitReader::endian(std::io::Cursor::new(slice), BigEndian);

    loop {
        let header = read_packet_header(&mut r).context("Parsing MHAS packet header")?;

        match header.packet_type {
            MHAS_PAC_TYP_MPEGH3DACFG => {
                let mut config = vec![0u8; header.packet_length as usize];
                r.read_bytes(&mut config)
                    .context("Reading MPEG-H configuration packet")?;

                return parse_config(&config).map(Some);
            }

            // Audio payload has started, abort.
            MHAS_PAC_TYP_MPEGH3DAFRAME => return Ok(None),

            // Paquet found, but it's not the config one. Skip to next packet.
            _ => skip_packet(&mut r, header.packet_length)?,
        }
    }
}

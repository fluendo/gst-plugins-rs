// Copyright (C) 2026
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v2.0.
// If a copy of the MPL was not distributed with this file, You can obtain one at
// <https://mozilla.org/MPL/2.0/>.
//
// SPDX-License-Identifier: MPL-2.0

use anyhow::{Context, Error, Ok, bail};
use bitstream_io::{BitRead, BitWrite, FromBitStream, ToBitStream};

// Helper functions

/// `presentation_version()` (ETSI TS 103 190-1, table 6).
fn read_presentation_version<R: BitRead + ?Sized>(r: &mut R) -> Result<u8, Error> {
    let mut val = 0u8;
    while r.read_bit().context("b_tmp")? {
        val = val
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("presentation_version out of range"))?;
    }
    Ok(val)
}

/// `variable_bits(n_bits)` (ETSI TS 103 190-1, clause 4.2.2).
fn read_variable_bits<R: BitRead + ?Sized>(r: &mut R, n_bits: u32) -> Result<u32, Error> {
    let mut value = 0u32;
    loop {
        value = value
            .checked_add(r.read_var::<u32>(n_bits).context("variable_bits")?)
            .ok_or_else(|| anyhow::anyhow!("variable_bits overflow"))?;
        let b_read_more = r.read_bit().context("b_read_more")?;
        if b_read_more {
            value <<= n_bits;
            value = value
                .checked_add(1 << n_bits)
                .ok_or_else(|| anyhow::anyhow!("variable_bits overflow"))?;
        } else {
            break;
        }
    }
    Ok(value)
}

/// `frame_rate_multiply_info()` (ETSI TS 103 190-1, table 7).
fn read_frame_rate_multiply_info<R: BitRead + ?Sized>(
    r: &mut R,
    frame_rate_index: u8,
) -> Result<(bool, bool), Error> {
    let mut b_multiplier = false;
    let mut multiplier_bit = false;

    match frame_rate_index {
        2..=4 => {
            b_multiplier = r.read_bit().context("b_multiplier")?;
            if b_multiplier {
                multiplier_bit = r.read_bit().context("multiplier_bit")?;
            }
        }
        0 | 1 | 7..=9 => {
            b_multiplier = r.read_bit().context("b_multiplier")?;
        }
        _ => {}
    }

    Ok((b_multiplier, multiplier_bit))
}

/// `frame_rate_factor` (ETSI TS 103 190-1, table 87).
fn calculate_frame_rate_factor(
    frame_rate_index: u8,
    b_multiplier: bool,
    multiplier_bit: bool,
) -> Result<u8, Error> {
    let factor = match frame_rate_index {
        2..=4 => match (b_multiplier, multiplier_bit) {
            (false, _) => 1,
            (true, false) => 2,
            (true, true) => 4,
        },
        0 | 1 | 7..=9 => {
            if b_multiplier {
                2
            } else {
                1
            }
        }
        5 | 6 | 10..=13 => 1,
        _ => bail!("invalid frame_rate_index {frame_rate_index}"),
    };
    Ok(factor)
}

/// `frame_rate_fractions_info()` (ETSI TS 103 190-2, clause 6.2.1.4).
fn read_frame_rate_fractions_info<R: BitRead + ?Sized>(
    r: &mut R,
    frame_rate_index: u8,
    frame_rate_factor: u8,
) -> Result<u8, Error> {
    let mut frame_rate_fraction = 1u8;

    if matches!(frame_rate_index, 5..=9)
        && frame_rate_factor == 1
        && r.read_bit().context("b_frame_rate_fraction")?
    {
        frame_rate_fraction = 2;
    }

    if matches!(frame_rate_index, 10..=12) && r.read_bit().context("b_frame_rate_fraction")? {
        frame_rate_fraction = if r.read_bit().context("b_frame_rate_fraction_is_4")? {
            4
        } else {
            2
        };
    }

    Ok(frame_rate_fraction)
}

/// `emdf_payloads_substream_info()` (ETSI TS 103 190-1, table 13).
fn emdf_payloads_substream_info<R: BitRead + ?Sized>(r: &mut R) -> Result<(), Error> {
    let substream_index = r.read::<2, u8>().context("substream_index")?;
    if substream_index == 3 {
        read_variable_bits(r, 2).context("substream_index ext")?;
    }
    Ok(())
}

/// `emdf_reserved()`, spelled `emdf_protection()` in ETSI TS 103 190-1 table 80.
fn emdf_reserved<R: BitRead + ?Sized>(r: &mut R) -> Result<(), Error> {
    let n_skip_bytes_length_primary = r.read::<2, u8>().context("n_skip_bytes_length_primary")?;
    let n_skip_bytes_length_secondary =
        r.read::<2, u8>().context("n_skip_bytes_length_secondary")?;

    let mut n_skip_bytes = 0u32;
    if n_skip_bytes_length_primary > 0 {
        n_skip_bytes += 1 << (2 * (n_skip_bytes_length_primary - 1));
    }
    if n_skip_bytes_length_secondary > 0 {
        n_skip_bytes += 1 << (2 * (n_skip_bytes_length_secondary - 1));
    }

    r.skip(8 * n_skip_bytes).context("reserved")?;
    Ok(())
}

/// `emdf_info()` (ETSI TS 103 190-1, table 8).
fn read_emdf_info<R: BitRead + ?Sized>(r: &mut R) -> Result<(u8, u32), Error> {
    let mut emdf_version = r.read::<2, u8>().context("emdf_version")?;
    if emdf_version == 3 {
        emdf_version = u8::try_from(read_variable_bits(r, 2)? + 3)
            .map_err(|_| anyhow::anyhow!("emdf_version out of range"))?;
    }

    let mut key_id = r.read::<3, u32>().context("key_id")?;
    if key_id == 7 {
        key_id += read_variable_bits(r, 3)?;
    }

    if r.read_bit().context("b_emdf_payloads_substream_info")? {
        emdf_payloads_substream_info(r)?;
    }

    emdf_reserved(r)?;

    Ok((emdf_version, key_id))
}

/// `content_type()` (ETSI TS 103 190-1, table 10).
fn read_content_type<R: BitRead + ?Sized>(r: &mut R) -> Result<TocContentType, Error> {
    let content_classifier = r.read::<3, u8>().context("content_classifier")?;

    let mut language_tag = None;
    if r.read_bit().context("b_language_indicator")? {
        if r.read_bit().context("b_serialized_language_tag")? {
            bail!("serialized AC-4 language tag is not supported yet");
        }

        let n_language_tag_bytes = r.read::<6, u8>().context("n_language_tag_bytes")?;
        let mut tag = Vec::with_capacity(n_language_tag_bytes as usize);
        for _ in 0..n_language_tag_bytes {
            tag.push(r.read::<8, u8>().context("language_tag_bytes")?);
        }
        language_tag = Some(tag);
    }

    Ok(TocContentType {
        content_classifier,
        language_tag,
    })
}

/// `channel_mode` as a prefix code (ETSI TS 103 190-2, table 56).
fn read_ch_mode<R: BitRead + ?Sized>(r: &mut R) -> Result<u8, Error> {
    // 0
    if !r.read_bit().context("channel_mode")? {
        return Ok(0);
    }
    // 10
    if !r.read_bit().context("channel_mode")? {
        return Ok(1);
    }
    // 110x
    if !r.read_bit().context("channel_mode")? {
        let bit = u8::from(r.read_bit().context("channel_mode")?);
        return Ok(2 + bit);
    }
    // 1110
    if !r.read_bit().context("channel_mode")? {
        return Ok(4);
    }

    // 1111xxx (three more bits)
    let bits = r.read::<3, u8>().context("channel_mode")?;
    if bits <= 5 {
        // 1111000 .. 1111101
        return Ok(5 + bits);
    }

    // 1111 110x / 1111 111x
    let bit = r.read_bit().context("channel_mode")?;
    match (bits, bit) {
        // 11111100 / 11111101
        (6, false) => Ok(11),
        (6, true) => Ok(12),
        // 11111110x / 11111111x
        (_, false) => {
            if r.read_bit().context("channel_mode")? {
                Ok(14)
            } else {
                Ok(13)
            }
        }
        (_, true) => {
            if !r.read_bit().context("channel_mode")? {
                Ok(15)
            } else {
                // Table 56 reserves 0b111111111 and everything above it, and no
                // channel group set is defined for those values.
                bail!("reserved channel_mode (>= 16)")
            }
        }
    }
}

/// `substream_index`, a 2 bit value with a `variable_bits(2)` escape.
fn read_substream_index<R: BitRead + ?Sized>(r: &mut R) -> Result<u32, Error> {
    let index = r.read::<2, u32>().context("substream_index")?;
    if index == 3 {
        Ok(3 + read_variable_bits(r, 2).context("substream_index ext")?)
    } else {
        Ok(index)
    }
}

/// `bitrate_indicator` (ETSI TS 103 190-1, table 90), returned as `brate_ind`.
fn read_bitrate_indicator<R: BitRead + ?Sized>(r: &mut R) -> Result<u8, Error> {
    let prefix = r.read::<3, u8>().context("bitrate_indicator")?;
    if prefix & 1 == 0 {
        Ok(prefix / 2)
    } else {
        Ok(2 * (prefix + 1) + r.read::<2, u8>().context("bitrate_indicator")?)
    }
}

/// `dsi_sf_multiplier` (ETSI TS 103 190-1, table E.5d).
fn dsi_sf_multiplier(fs_index: u8, b_sf_multiplier: bool, sf_multiplier: bool) -> u8 {
    if fs_index == 0 || !b_sf_multiplier {
        0
    } else if sf_multiplier {
        2
    } else {
        1
    }
}

/// `ac4_hsf_ext_substream_info()` (ETSI TS 103 190-1, table 12).
fn read_hsf_ext_substream_info<R: BitRead + ?Sized>(
    r: &mut R,
    b_substreams_present: bool,
) -> Result<(), Error> {
    if b_substreams_present {
        read_substream_index(r).context("ac4_hsf_ext_substream_info")?;
    }
    Ok(())
}

/// `ac4_substream_info_chan()` (ETSI TS 103 190-2, clause 6.2.1.8).
fn read_substream_info_chan<R: BitRead + ?Sized>(
    r: &mut R,
    fs_index: u8,
    b_substreams_present: bool,
    frame_rate_factor: u8,
) -> Result<TocSubstreamChan, Error> {
    let ch_mode = read_ch_mode(r)?;

    let mut b_4_back_channels_present = false;
    let mut b_centre_present = false;
    let mut top_channels_present = 0;
    if matches!(ch_mode, 11..=14) {
        b_4_back_channels_present = r.read_bit().context("b_4_back_channels_present")?;
        b_centre_present = r.read_bit().context("b_centre_present")?;
        top_channels_present = r.read::<2, u8>().context("top_channels_present")?;
    }

    let mut sf_multiplier = 0u8; // dsi_sf_multiplier
    if fs_index == 1 {
        let b_sf_multiplier = r.read_bit().context("b_sf_multiplier")?;
        if b_sf_multiplier {
            let sf = r.read_bit().context("sf_multiplier")?;
            sf_multiplier = dsi_sf_multiplier(fs_index, b_sf_multiplier, sf);
        }
    }

    let bitrate_indicator = if r.read_bit().context("b_bitrate_info")? {
        Some(read_bitrate_indicator(r)?)
    } else {
        None
    };

    if matches!(ch_mode, 7..=10) {
        r.skip(1).context("add_ch_base")?;
    }

    for _ in 0..frame_rate_factor {
        r.read_bit().context("b_audio_ndot")?;
    }

    if b_substreams_present {
        read_substream_index(r)?;
    }

    Ok(TocSubstreamChan {
        ch_mode,
        b_4_back_channels_present,
        b_centre_present,
        top_channels_present,
        dsi_sf_multiplier: sf_multiplier,
        bitrate_indicator,
    })
}

/// `ac4_substream_group_info()` (ETSI TS 103 190-2, clause 6.2.1.6).
fn read_substream_group_info<R: BitRead + ?Sized>(
    r: &mut R,
    fs_index: u8,
    frame_rate_factor: u8,
) -> Result<TocSubstreamGroup, Error> {
    let b_substreams_present = r.read_bit().context("b_substreams_present")?;
    let b_hsf_ext = r.read_bit().context("b_hsf_ext")?;

    let b_single_substream = r.read_bit().context("b_single_substream")?;
    let n_lf_substreams = if b_single_substream {
        1u32
    } else {
        let mut n = u32::from(r.read::<2, u8>().context("n_lf_substreams_minus2")?) + 2;
        if n == 5 {
            n += read_variable_bits(r, 2).context("n_lf_substreams ext")?;
        }
        n
    };

    let b_channel_coded = r.read_bit().context("b_channel_coded")?;
    if !b_channel_coded {
        bail!("object/A-JOC coded AC-4 substreams are not supported yet");
    }

    let mut substreams = Vec::with_capacity(n_lf_substreams.min(64) as usize);
    for _ in 0..n_lf_substreams {
        substreams.push(read_substream_info_chan(
            r,
            fs_index,
            b_substreams_present,
            frame_rate_factor,
        )?);
        if b_hsf_ext {
            read_hsf_ext_substream_info(r, b_substreams_present)?;
        }
    }

    let content_type = if r.read_bit().context("b_content_type")? {
        Some(read_content_type(r)?)
    } else {
        None
    };

    Ok(TocSubstreamGroup {
        b_substreams_present,
        b_hsf_ext,
        b_channel_coded,
        substreams,
        content_type,
    })
}

/// `ac4_sgi_specifier()` (ETSI TS 103 190-2, clause 6.2.1.7).
///
/// The bitstream_version 1 variant inlines an `ac4_substream_group_info()`
/// instead of referencing it by index; that version is rejected before any
/// presentation is parsed.
fn read_sgi_specifier<R: BitRead + ?Sized>(r: &mut R) -> Result<u8, Error> {
    let group_index = r.read::<3, u8>().context("group_index")?;
    if group_index == 7 {
        let ext = read_variable_bits(r, 2).context("group_index ext")?;
        u8::try_from(7 + ext).map_err(|_| anyhow::anyhow!("group_index {ext} out of range"))
    } else {
        Ok(group_index)
    }
}

/// `ac4_presentation_substream_info()` (ETSI TS 103 190-2, clause 6.2.1.12).
fn read_presentation_substream_info<R: BitRead + ?Sized>(r: &mut R) -> Result<bool, Error> {
    let b_alternative = r.read_bit().context("b_alternative")?;
    r.read_bit().context("b_pres_ndot")?;
    read_substream_index(r)?;
    Ok(b_alternative)
}

/// `ac4_presentation_v1_info()` (ETSI TS 103 190-2, clause 6.2.1.3).
fn read_presentation_v1<R: BitRead + ?Sized>(
    r: &mut R,
    frame_rate_index: u8,
) -> Result<TocPresentation, Error> {
    let b_single_substream_group = r.read_bit().context("b_single_substream_group")?;

    let mut presentation_config = None;
    if !b_single_substream_group {
        let mut config = r.read::<3, u8>().context("presentation_config")?;
        if config == 7 {
            config = u8::try_from(7 + read_variable_bits(r, 2).context("presentation_config ext")?)
                .map_err(|_| anyhow::anyhow!("presentation_config out of range"))?;
        }
        presentation_config = Some(config);
    }

    let presentation_version = read_presentation_version(r)?;

    let mut presentation = TocPresentation {
        presentation_version,
        b_single_substream_group,
        presentation_config,
        md_compat: 0,
        b_presentation_id: false,
        presentation_id: None,
        frame_rate_factor: 1,
        frame_rate_fraction: 1,
        emdf_version: 0,
        key_id: 0,
        b_presentation_filter: false,
        b_enable_presentation: false,
        b_multi_pid: false,
        group_indices: Vec::new(),
        n_substream_groups: 0,
        b_pre_virtualized: false,
        b_add_emdf_substreams: false,
        add_emdf: Vec::new(),
        skip_data: Vec::new(),
        b_alternative: false,
    };

    // "if (b_single_substream_group != 1 and presentation_config == 6)"
    if b_single_substream_group || presentation_config != Some(6) {
        presentation.md_compat = r.read::<3, u8>().context("md_compat")?;

        presentation.b_presentation_id = r.read_bit().context("b_presentation_id")?;
        if presentation.b_presentation_id {
            presentation.presentation_id =
                Some(read_variable_bits(r, 2).context("presentation_id")?);
        }

        let (b_multiplier, multiplier_bit) = read_frame_rate_multiply_info(r, frame_rate_index)?;
        presentation.frame_rate_factor =
            calculate_frame_rate_factor(frame_rate_index, b_multiplier, multiplier_bit)?;
        presentation.frame_rate_fraction =
            read_frame_rate_fractions_info(r, frame_rate_index, presentation.frame_rate_factor)?;

        let (emdf_version, key_id) = read_emdf_info(r)?;
        presentation.emdf_version = emdf_version;
        presentation.key_id = key_id;

        presentation.b_presentation_filter = r.read_bit().context("b_presentation_filter")?;
        if presentation.b_presentation_filter {
            presentation.b_enable_presentation = r.read_bit().context("b_enable_presentation")?;
        }

        if b_single_substream_group {
            read_presentation_sgi_specifier(r, &mut presentation)?;
            presentation.n_substream_groups = 1;
        } else {
            presentation.b_multi_pid = r.read_bit().context("b_multi_pid")?;

            match presentation_config {
                Some(0) => {
                    read_presentation_sgi_specifier(r, &mut presentation)?;
                    read_presentation_sgi_specifier(r, &mut presentation)?;
                    presentation.n_substream_groups = 2;
                }
                Some(1) => {
                    read_presentation_sgi_specifier(r, &mut presentation)?;
                    read_presentation_sgi_specifier(r, &mut presentation)?;
                    presentation.n_substream_groups = 1;
                }
                Some(2) => {
                    read_presentation_sgi_specifier(r, &mut presentation)?;
                    read_presentation_sgi_specifier(r, &mut presentation)?;
                    presentation.n_substream_groups = 2;
                }
                Some(3) => {
                    read_presentation_sgi_specifier(r, &mut presentation)?;
                    read_presentation_sgi_specifier(r, &mut presentation)?;
                    read_presentation_sgi_specifier(r, &mut presentation)?;
                    presentation.n_substream_groups = 3;
                }
                Some(4) => {
                    read_presentation_sgi_specifier(r, &mut presentation)?;
                    read_presentation_sgi_specifier(r, &mut presentation)?;
                    read_presentation_sgi_specifier(r, &mut presentation)?;
                    presentation.n_substream_groups = 2;
                }
                Some(5) => {
                    let mut n =
                        u32::from(r.read::<2, u8>().context("n_substream_groups_minus2")?) + 2;
                    if n == 5 {
                        n += read_variable_bits(r, 2).context("n_substream_groups ext")?;
                    }
                    presentation.n_substream_groups = u8::try_from(n)
                        .map_err(|_| anyhow::anyhow!("n_substream_groups {n} out of range"))?;
                    for _ in 0..n {
                        read_presentation_sgi_specifier(r, &mut presentation)?;
                    }
                }
                // `default: presentation_config_ext_info();` (clause 6.2.1.5).
                // Table 53 marks >= 7 as reserved, so the body is an opaque
                // byte blob that is passed through unchanged.
                Some(config) => {
                    let mut n_skip_bytes = u32::from(r.read::<5, u8>().context("n_skip_bytes")?);
                    if r.read_bit().context("b_more_skip_bytes")? {
                        n_skip_bytes += read_variable_bits(r, 2).context("n_skip_bytes ext")? << 5;
                    }
                    // ac4_presentation_v1_dsi() signals n_skip_bytes in 7 bits.
                    if n_skip_bytes > 0x7f {
                        bail!(
                            "presentation_config {config} skips {n_skip_bytes} bytes, \
                             more than ac4_presentation_v1_dsi() can signal"
                        );
                    }
                    for _ in 0..n_skip_bytes {
                        presentation
                            .skip_data
                            .push(r.read::<8, u8>().context("skip_data")?);
                    }
                }
                None => {
                    unreachable!("presentation_config is read when b_single_substream_group is 0")
                }
            }
        }

        presentation.b_pre_virtualized = r.read_bit().context("b_pre_virtualized")?;
        presentation.b_add_emdf_substreams = r.read_bit().context("b_add_emdf_substreams")?;

        presentation.b_alternative = read_presentation_substream_info(r)?;
    } else {
        presentation.b_add_emdf_substreams = true;
    }

    if presentation.b_add_emdf_substreams {
        let mut n = u32::from(r.read::<2, u8>().context("n_add_emdf_substreams")?);
        if n == 0 {
            n = read_variable_bits(r, 2).context("n_add_emdf_substreams ext")? + 4;
        }
        for _ in 0..n {
            presentation.add_emdf.push(read_emdf_info(r)?);
        }
    }

    Ok(presentation)
}

/// One `ac4_sgi_specifier()` appended to `presentation.group_indices`.
fn read_presentation_sgi_specifier<R: BitRead + ?Sized>(
    r: &mut R,
    presentation: &mut TocPresentation,
) -> Result<(), Error> {
    let index = read_sgi_specifier(r)?;
    presentation.group_indices.push(index);
    Ok(())
}

// ---------------------------------------------------------------------------
// TOC -> DSI mapping (ETSI TS 103 190-2 V1.3.1, annexes E and A.3)
// ---------------------------------------------------------------------------

/// Audio channel groups (ETSI TS 103 190-2, table A.27, "channel group index"
/// column). 
pub(crate) mod ch_group {
    pub const L_R: u32 = 1 << 0;
    pub const C: u32 = 1 << 1;
    pub const LS_RS: u32 = 1 << 2;
    pub const LB_RB: u32 = 1 << 3;
    pub const TFL_TFR: u32 = 1 << 4;
    pub const TBL_TBR: u32 = 1 << 5;
    pub const LFE: u32 = 1 << 6;
    pub const TSL_TSR: u32 = 1 << 7;
    // Group 8 is deprecated, a decoder shall read its speakers as group 7
    // (table A.27, note 3), so it is never signalled.
    pub const TFC: u32 = 1 << 9;
    pub const TBC: u32 = 1 << 10;
    pub const TC: u32 = 1 << 11;
    pub const LFE2: u32 = 1 << 12;
    pub const BFL_BFR: u32 = 1 << 13;
    pub const BFC: u32 = 1 << 14;
    pub const CB: u32 = 1 << 15;
    // Group 16 (Lscr/Rscr) is not part of any channel mode of table 56, and
    // pseudocode E.3 hardcodes it to 0.
    pub const LW_RW: u32 = 1 << 17;
}

use ch_group::*;

/// Channel groups of each `ch_mode`, built from the channel lists of table 56
/// plus the speaker-to-channel-group mapping of table A.27.
///
/// This is table driven rather than a literal transcription of pseudocode E.3
/// because that pseudocode is incomplete:
///   * it never sets group 6 (LFE), yet 5.1 has to carry 0x47 (LFE set);
///   * its `pres_ch_mode >= 11` branch sets neither group 0 (L/R) nor 1 (C);
///   * it writes the centre to index 2, which table A.27 assigns to Ls/Rs
///     (C is group 1).
const CH_MODE_GROUPS: [u32; 16] = [
    C,                                                 // 0  mono
    L_R,                                               // 1  stereo
    L_R | C,                                           // 2  3.0
    L_R | C | LS_RS,                                   // 3  5.0
    L_R | C | LS_RS | LFE,                             // 4  5.1
    L_R | C | LS_RS | LB_RB,                           // 5  7.0 3/4/0
    L_R | C | LS_RS | LB_RB | LFE,                     // 6  7.1 3/4/0.1
    L_R | C | LS_RS | LW_RW,                           // 7  7.0 5/2/0
    L_R | C | LS_RS | LW_RW | LFE,                     // 8  7.1 5/2/0.1
    L_R | C | LS_RS | TFL_TFR,                         // 9  7.0 3/2/2
    L_R | C | LS_RS | TFL_TFR | LFE,                   // 10 7.1 3/2/2.1
    L_R | C | LS_RS | LB_RB | TFL_TFR | TBL_TBR,       // 11 7.0.4
    L_R | C | LS_RS | LB_RB | TFL_TFR | TBL_TBR | LFE, // 12 7.1.4
    L_R | C | LS_RS | LB_RB | TFL_TFR | TBL_TBR,       // 13 9.0.4
    L_R | C | LS_RS | LB_RB | TFL_TFR | TBL_TBR | LFE, // 14 9.1.4
    L_R | C
        | LS_RS
        | LB_RB
        | TFL_TFR
        | TBL_TBR
        | LFE
        | TSL_TSR
        | TFC
        | TBC
        | TC
        | LFE2
        | BFL_BFR
        | BFC
        | CB
        | LW_RW, // 15 22.2
];

/// `superset()` for `ch_mode` values (ETSI TS 103 190-2, clause 6.3.3.1.27):
/// the lowest channel mode that contains every channel of both inputs.
///
/// Note: the same clause also states "The result of superset(0,1) shall be 1",
/// which contradicts its own definition (mono C plus stereo L,R is 3.0, i.e.
/// ch_mode 2). The definition is followed here; the difference only affects
/// presentations that mix mono and stereo substreams.
fn superset_ch_mode(a: Option<u8>, b: Option<u8>) -> Option<u8> {
    let (a, b) = match (a, b) {
        (None, other) | (other, None) => return other,
        (Some(a), Some(b)) => (a, b),
    };

    let union = CH_MODE_GROUPS[usize::from(a)] | CH_MODE_GROUPS[usize::from(b)];
    // `superset(4, 11) == 12` per the example in clause 6.3.3.1.27.
    (0..=15u8).find(|&mode| CH_MODE_GROUPS[usize::from(mode)] & union == union)
}

/// `ch_mode_core` of a single substream (table 71), for channel coded
/// substreams only.
fn ch_mode_core(ch_mode: u8) -> Option<u8> {
    match ch_mode {
        11 | 13 => Some(5),
        12 | 14 => Some(6),
        _ => None,
    }
}

/// `pres_top_channel_pairs` contribution of one substream (table 72).
fn top_channel_pairs(top_channels_present: u8) -> u8 {
    match top_channels_present {
        1 | 2 => 1,
        3 => 2,
        _ => 0,
    }
}

/// `presentation_v1_channel_groups` / `dsi_substream_channel_groups`
/// (pseudocode E.3, tables 57/58/59).
fn channel_groups(
    ch_mode: u8,
    b_centre_present: bool,
    b_4_back_channels_present: bool,
    pres_top_channel_pairs: u8,
) -> u32 {
    let mut groups = CH_MODE_GROUPS[usize::from(ch_mode)];

    if ch_mode >= 11 {
        // The channel layouts of table 56 list these groups; the presence flags
        // say which of them carry original content instead of silence.
        if !b_centre_present {
            groups &= !C;
        }
        if !b_4_back_channels_present {
            groups &= !LB_RB;
        }
        groups &= !(TFL_TFR | TBL_TBR | TSL_TSR);
        match pres_top_channel_pairs {
            // table 59: Tsl/Tsr are carried in Tfl/Tfr or Tbl/Tbr
            1 => groups |= TSL_TSR,
            // table 59: all four top channels carry original content
            2 => groups |= TFL_TFR | TBL_TBR,
            _ => {}
        }
    }

    groups
}

/// `dsi_frame_rate_multiply_info` (ETSI TS 103 190-2, table E.12):
/// the `frame_rate_factor` of table 87 (ETSI TS 103 190-1)
/// recoded as 1 -> 00, 2 -> 01, 4 -> 10.
fn dsi_frame_rate_multiply_info(frame_rate_factor: u8) -> Result<u8, Error> {
    match frame_rate_factor {
        1 => Ok(0),
        2 => Ok(1),
        4 => Ok(2),
        other => bail!("unexpected frame_rate_factor {other}"),
    }
}

/// `dsi_frame_rate_fraction_info` (ETSI TS 103 190-2, table E.13):
/// the `frame_rate_fraction` from `frame_rate_fractions_info()`
/// (ETSI TS 103 190-2) recoded as 1 -> 00, 2 -> 01, 4 -> 10.
fn dsi_frame_rate_fraction_info(frame_rate_fraction: u8) -> Result<u8, Error> {
    match frame_rate_fraction {
        1 => Ok(0),
        2 => Ok(1),
        4 => Ok(2),
        other => bail!("unexpected frame_rate_fraction {other}"),
    }
}

/// `dsi_presentation_channel_mode_core` (table E.14).
fn dsi_ch_mode_core(ch_mode_core: u8) -> Result<u8, Error> {
    match ch_mode_core {
        3 => Ok(0),
        4 => Ok(1),
        5 => Ok(2),
        6 => Ok(3),
        other => bail!("unexpected ch_mode_core {other}"),
    }
}

impl SubstreamDsi {
    fn from_toc(substream: &TocSubstreamChan) -> Result<Self, Error> {
        Ok(Self {
            dsi_sf_multiplier: substream.dsi_sf_multiplier,
            substream_bitrate_indicator: substream.bitrate_indicator,
            // Only channel coded substreams reach this point, see
            // read_substream_group_info().
            channel_groups: Some(channel_groups(
                substream.ch_mode,
                substream.b_centre_present,
                substream.b_4_back_channels_present,
                top_channel_pairs(substream.top_channels_present),
            )),
        })
    }
}

impl SubstreamDsiGroup {
    fn from_toc(group: &TocSubstreamGroup) -> Result<Self, Error> {
        Ok(Self {
            b_substreams_present: group.b_substreams_present,
            b_hsf_ext: group.b_hsf_ext,
            b_channel_coded: group.b_channel_coded,
            substreams: group
                .substreams
                .iter()
                .map(SubstreamDsi::from_toc)
                .collect::<Result<_, _>>()?,
            content_type: group.content_type.clone(),
        })
    }
}

/// Aggregated channel information of one presentation, derived from the
/// substream groups it references (pseudocode 25/26).
struct PresentationChannels {
    /// `None` when the presentation is not channel coded.
    ch_mode: Option<u8>,
    /// `None` when `pres_ch_mode_core` is undefined or equals `pres_ch_mode`.
    ch_mode_core: Option<u8>,
    b_centre_present: bool,
    b_4_back_channels_present: bool,
    top_channel_pairs: u8,
}

fn presentation_channels<'a>(
    groups: impl Iterator<Item = &'a TocSubstreamGroup>,
) -> PresentationChannels {
    let mut ch_mode = None;
    let mut core_mode = None;
    let mut b_centre_present = false;
    let mut b_4_back_channels_present = false;
    let mut top_pairs = 0u8;

    for group in groups {
        for substream in &group.substreams {
            ch_mode = superset_ch_mode(ch_mode, Some(substream.ch_mode));
            if let Some(core) = ch_mode_core(substream.ch_mode) {
                core_mode = superset_ch_mode(core_mode, Some(core));
            }

            b_centre_present |= substream.b_centre_present;
            b_4_back_channels_present |= substream.b_4_back_channels_present;
            top_pairs = top_pairs.max(top_channel_pairs(substream.top_channels_present));
        }
    }

    // "If pres_ch_mode_core == pres_ch_mode then pres_ch_mode_core = -1."
    if core_mode == ch_mode {
        core_mode = None;
    }

    PresentationChannels {
        ch_mode,
        ch_mode_core: core_mode,
        b_centre_present,
        b_4_back_channels_present,
        top_channel_pairs: top_pairs,
    }
}

impl Ac4PresentationV1Dsi {
    pub(crate) fn from_toc(
        presentation: &TocPresentation,
        groups: &[TocSubstreamGroup],
    ) -> Result<Self, Error> {
        if presentation.b_alternative {
            bail!("alternative AC-4 presentations are not supported yet");
        }
        if presentation.presentation_version != 1 {
            bail!(
                "ac4_presentation_v1_dsi() needs presentation_version 1, got {}",
                presentation.presentation_version
            );
        }

        let referenced = presentation
            .group_indices
            .iter()
            .map(|index| {
                groups.get(usize::from(*index)).ok_or_else(|| {
                    anyhow::anyhow!(
                        "presentation references substream group {index}, but only {} are present",
                        groups.len()
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        let channels = presentation_channels(referenced.iter().copied());

        // E.11: b_extended_presentation_id shall be set when the id does not
        // fit in the 5 bit presentation_id, and the 9 bit value overrides it.
        let (b_extended_presentation_id, presentation_id, extended_presentation_id) =
            match presentation.presentation_id {
                Some(id) if id > 0x1ff => {
                    bail!("presentation_id {id} does not fit in the 9 bit extended_presentation_id")
                }
                Some(id) if id > 0x1f => (true, (id & 0x1f) as u8, id as u16),
                Some(id) => (false, id as u8, 0),
                None => (false, 0, 0),
            };

        if presentation.emdf_version > 0x1f {
            bail!(
                "presentation_emdf_version {} does not fit in 5 bits",
                presentation.emdf_version
            );
        }
        let presentation_key_id = u16::try_from(presentation.key_id)
            .ok()
            .filter(|id| *id <= 0x3ff)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "presentation_key_id {} does not fit in 10 bits",
                    presentation.key_id
                )
            })?;

        let mut add_emdf = Vec::with_capacity(presentation.add_emdf.len());
        for (emdf_version, key_id) in &presentation.add_emdf {
            let key_id = u16::try_from(*key_id)
                .ok()
                .filter(|id| *id <= 0x3ff)
                .ok_or_else(|| {
                    anyhow::anyhow!("substream_key_id {key_id} does not fit in 10 bits")
                })?;
            add_emdf.push((*emdf_version, key_id));
        }

        let ch_mode = channels.ch_mode;
        let ch_mode_core = channels.ch_mode_core;

        Ok(Self {
            presentation_config_v1: if presentation.b_single_substream_group {
                0x1f
            } else {
                presentation
                    .presentation_config
                    .ok_or_else(|| anyhow::anyhow!("missing presentation_config"))?
            },
            md_compat: presentation.md_compat,
            b_presentation_id: presentation.b_presentation_id,
            presentation_id,
            dsi_frame_rate_multiply_info: dsi_frame_rate_multiply_info(
                presentation.frame_rate_factor,
            )?,
            dsi_frame_rate_fraction_info: dsi_frame_rate_fraction_info(
                presentation.frame_rate_fraction,
            )?,
            presentation_emdf_version: presentation.emdf_version,
            presentation_key_id,
            b_presentation_channel_coded: ch_mode.is_some(),
            dsi_presentation_ch_mode: ch_mode.unwrap_or(0),
            pres_b_4_back_channels_present: channels.b_4_back_channels_present,
            pres_top_channel_pairs: channels.top_channel_pairs,
            presentation_v1_channel_groups: match ch_mode {
                Some(mode) => channel_groups(
                    mode,
                    channels.b_centre_present,
                    channels.b_4_back_channels_present,
                    channels.top_channel_pairs,
                ),
                // No channel groups without a channel mode; the field is not
                // written in that case (E.10.1).
                None => 0,
            },
            // E.11 says "true if pres_ch_mode_core == -1", but the reference
            // encoder does the opposite (true when it is defined). Following the
            // reference encoder, which is also the only reading that makes the
            // nested b_presentation_core_channel_coded field meaningful: under
            // the literal reading it would always be 0, so
            // dsi_presentation_channel_mode_core could never be written.
            b_presentation_core_differs: ch_mode_core.is_some(),
            b_presentation_core_channel_coded: ch_mode_core.is_some(),
            dsi_presentation_channel_mode_core: match ch_mode_core {
                Some(core) => dsi_ch_mode_core(core)?,
                None => 0,
            },
            b_presentation_filter: presentation.b_presentation_filter,
            b_enable_presentation: presentation.b_enable_presentation,
            // n_filter_bytes is written as 0: filter data is reserved and
            // ignored by decoders (table E.11).
            filter_data: Vec::new(),
            b_multi_pid: presentation.b_multi_pid,
            skip_data: presentation.skip_data.clone(),
            substream_groups: referenced
                .iter()
                .map(|group| SubstreamDsiGroup::from_toc(group))
                .collect::<Result<_, _>>()?,
            b_pre_virtualized: presentation.b_pre_virtualized,
            b_add_emdf_substreams: presentation.b_add_emdf_substreams,
            add_emdf,
            b_presentation_bitrate_info: false,
            bitrate: None,
            b_alternative: false,
            // Both flags are defined by table E.11 through data that is not in
            // the TOC: de_indicator from b_de_data_present / b_dialog_max_gain
            // and immersive_audio_indicator from ac4_presentation_substream(),
            // i.e. from the substream payloads, which are not parsed. The values
            // below match the reference encoder instead of being derived.
            de_indicator: true,
            immersive_audio_indicator: false,
            b_extended_presentation_id,
            extended_presentation_id,
        })
    }
}

impl PresentationDsi {
    pub(crate) fn from_toc(
        presentation: &TocPresentation,
        groups: &[TocSubstreamGroup],
    ) -> Result<Self, Error> {
        match presentation.presentation_version {
            1 => Ok(Self::V1(Ac4PresentationV1Dsi::from_toc(
                presentation,
                groups,
            )?)),
            0 => bail!("presentation_version 0 (ac4_presentation_v0_dsi()) is not supported yet"),
            version => Ok(Self::Other(version)),
        }
    }
}

// ------------------------------------------------ //
// READ TOC
#[derive(Debug, Clone)]
pub struct Ac4Toc {
    pub bitstream_version: u8,
    pub b_wait_frames: bool,
    pub wait_frames: u8,
    pub fs_index: u8,
    pub frame_rate_index: u8,
    pub b_program_id: bool,
    pub short_program_id: u16,
    pub b_program_uuid_present: bool,
    pub program_uuid: [u8; 16],
    pub presentations: Vec<TocPresentation>,
    /// Substream groups, in bitstream order; referenced by
    /// `TocPresentation::group_indices`.
    pub groups: Vec<TocSubstreamGroup>,
}
// Read bits from ac4 toc
impl FromBitStream for Ac4Toc {
    type Error = Error;
    fn from_reader<R: bitstream_io::BitRead + ?Sized>(r: &mut R) -> Result<Self, Self::Error>
    where
        Self: Sized,
    {
        let mut bitstream_version = r.read::<2, u32>().context("bitstream_version")?;
        if bitstream_version == 3 {
            bitstream_version += read_variable_bits(r, 2).context("bitstream_version ext")?;
        }
        let bitstream_version = u8::try_from(bitstream_version)
            .map_err(|_| anyhow::anyhow!("bitstream_version out of range"))?;
        // Table 51 defines 0, 1 and 2 only; higher values are reserved.
        if bitstream_version > 2 {
            bail!("reserved AC-4 bitstream_version {bitstream_version}");
        }

        r.skip(10).context("sequence_number")?;

        let b_wait_frames = r.read_bit().context("b_wait_frames")?;
        let mut wait_frames = 0u8;

        if b_wait_frames {
            wait_frames = r.read::<3, u8>().context("wait_frames")?;
            if wait_frames > 0 {
                // br_code only selects the bit rate coding mode that the
                // bit_rate_mode in E.7 already covers.
                r.skip(2).context("br_code")?;
            }
        }

        let fs_index = r.read::<1, u8>().context("fs_index")?;

        let frame_rate_index = r.read::<4, u8>().context("frame_rate_index")?;

        r.skip(1).context("b_iframe_global")?;

        let b_single_presentation = r.read_bit().context("b_single_presentation")?;
        let n_presentations = if b_single_presentation {
            1
        } else if r.read_bit().context("b_more_presentations")? {
            read_variable_bits(r, 2)? + 2
        } else {
            0
        };

        if r.read_bit().context("b_payload_base")? {
            // payload_base locates the start of the raw AC-4 frame inside the
            // payload; the DSI does not need it, but the bits have to be consumed.
            if r.read::<5, u8>().context("payload_base_minus1")? + 1 == 0x20 {
                read_variable_bits(r, 3).context("payload_base ext")?;
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
        let mut presentations = Vec::with_capacity(n_presentations as usize);
        for _ in 0..n_presentations {
            presentations.push(read_presentation_v1(r, frame_rate_index)?);
        }

        // total_n_substream_groups = 1 + max_group_index (clause 6.3.2.1.8).
        let n_groups = presentations
            .iter()
            .flat_map(|presentation| presentation.group_indices.iter().copied())
            .max()
            .map_or(0usize, |index| usize::from(index) + 1);

        // A substream group carries one b_audio_ndot per frame of the presentation
        // that references it, and frame_rate_multiply_info() is per presentation.
        let mut group_frame_rate_factor = vec![None; n_groups];
        for presentation in &presentations {
            for index in &presentation.group_indices {
                let slot = &mut group_frame_rate_factor[usize::from(*index)];
                if matches!(slot, Some(factor) if *factor != presentation.frame_rate_factor) {
                    bail!("substream group {index} is referenced with different frame_rate_factor");
                }
                *slot = Some(presentation.frame_rate_factor);
            }
        }

        let mut groups = Vec::with_capacity(n_groups);
        for factor in &group_frame_rate_factor {
            groups.push(read_substream_group_info(r, fs_index, factor.unwrap_or(1))?);
        }
        Ok(Ac4Toc {
            bitstream_version,
            b_wait_frames,
            wait_frames,
            fs_index,
            frame_rate_index,
            b_program_id,
            short_program_id,
            b_program_uuid_present,
            program_uuid,
            presentations,
            groups,
        })
    }
}

/// `ac4_presentation_v1_info()` (ETSI TS 103 190-2, clause 6.2.1.3).
#[derive(Debug, Clone)]
pub struct TocPresentation {
    pub presentation_version: u8,
    pub b_single_substream_group: bool,
    pub presentation_config: Option<u8>,
    pub md_compat: u8,
    pub b_presentation_id: bool,
    pub presentation_id: Option<u32>,
    /// Derived from `frame_rate_multiply_info()` (ETSI TS 103 190-1, table 87).
    pub frame_rate_factor: u8,
    /// Derived from `frame_rate_fractions_info()`: 1, 2 or 4.
    pub frame_rate_fraction: u8,
    pub emdf_version: u8,
    pub key_id: u32,
    pub b_presentation_filter: bool,
    pub b_enable_presentation: bool,
    pub b_multi_pid: bool,
    /// One entry per `ac4_sgi_specifier()`, indexing into `Ac4Toc::groups`.
    pub group_indices: Vec<u8>,
    pub n_substream_groups: u8,
    pub b_pre_virtualized: bool,
    pub b_add_emdf_substreams: bool,
    pub add_emdf: Vec<(u8, u32)>,
    /// Opaque payload of `presentation_config_ext_info()` for
    /// `presentation_config` >= 7 (ETSI TS 103 190-2, clause 6.2.1.5).
    pub skip_data: Vec<u8>,
    /// Must be false, `alternative_info()` is not supported yet.
    pub b_alternative: bool,
}

/// `ac4_substream_group_info()` (ETSI TS 103 190-2, clause 6.2.1.6).
#[derive(Debug, Clone)]
pub struct TocSubstreamGroup {
    pub b_substreams_present: bool,
    pub b_hsf_ext: bool,
    pub b_channel_coded: bool,
    pub substreams: Vec<TocSubstreamChan>,
    pub content_type: Option<TocContentType>,
}

/// `ac4_substream_info_chan()` (ETSI TS 103 190-2, clause 6.2.1.8).
#[derive(Debug, Clone, Copy)]
pub struct TocSubstreamChan {
    pub ch_mode: u8,
    pub b_4_back_channels_present: bool,
    pub b_centre_present: bool,
    pub top_channels_present: u8,
    /// Already mapped to its DSI value (ETSI TS 103 190-1, table E.5d): 0, 1 or 2.
    pub dsi_sf_multiplier: u8,
    /// `None` when `b_bitrate_info` is not set.
    pub bitrate_indicator: Option<u8>,
}

/// `content_type()` (ETSI TS 103 190-1, clause 4.2.3.7).
#[derive(Debug, Clone)]
pub struct TocContentType {
    pub content_classifier: u8,
    /// Complete BCP 47 language tag; `None` if it is only signalled as a
    /// serialized chunk that has to be assembled across frames.
    pub language_tag: Option<Vec<u8>>,
}

// --------------------------------------------------
// WRITE DSI

#[derive(Debug, Clone)]
pub(crate) struct Ac4PresentationV1Dsi {
    pub presentation_config_v1: u8,
    pub md_compat: u8,
    pub b_presentation_id: bool,
    pub presentation_id: u8,
    pub dsi_frame_rate_multiply_info: u8,
    pub dsi_frame_rate_fraction_info: u8,
    pub presentation_emdf_version: u8,
    pub presentation_key_id: u16,
    pub b_presentation_channel_coded: bool,
    pub dsi_presentation_ch_mode: u8,
    pub pres_b_4_back_channels_present: bool,
    pub pres_top_channel_pairs: u8,
    pub presentation_v1_channel_groups: u32,
    pub b_presentation_core_differs: bool,
    pub b_presentation_core_channel_coded: bool,
    pub dsi_presentation_channel_mode_core: u8,
    pub b_presentation_filter: bool,
    pub b_enable_presentation: bool,
    /// `n_filter_bytes` is derived from its length when writing.
    pub filter_data: Vec<u8>,
    pub b_multi_pid: bool,
    pub skip_data: Vec<u8>,
    pub substream_groups: Vec<SubstreamDsiGroup>,
    pub b_pre_virtualized: bool,
    pub b_add_emdf_substreams: bool,
    /// `n_add_emdf_substreams` is derived from its length when writing.
    pub add_emdf: Vec<(u8, u16)>,
    pub b_presentation_bitrate_info: bool,
    pub bitrate: Option<Ac4BitrateDsi>,
    pub b_alternative: bool,
    pub de_indicator: bool,
    pub immersive_audio_indicator: bool,
    pub b_extended_presentation_id: bool,
    pub extended_presentation_id: u16,
}

/// `ac4_substream_group_dsi()` (ETSI TS 103 190-2, clause E.11).
#[derive(Debug, Clone)]
pub(crate) struct SubstreamDsiGroup {
    pub b_substreams_present: bool,
    pub b_hsf_ext: bool,
    pub b_channel_coded: bool,
    pub substreams: Vec<SubstreamDsi>,

    /// Following table E.15: Group level, not per substream. E.11.1 indents
    /// `b_content_type` inside the substream loop, but E.11.2 (table E.15)
    /// defines it as `b_content_type` of the group level `ac4_substream_group_info()`
    pub content_type: Option<TocContentType>,
}

/// One substream of an `ac4_substream_group_dsi()` (ETSI TS 103 190-2, E.11).
#[derive(Debug, Clone)]
pub(crate) struct SubstreamDsi {
    pub dsi_sf_multiplier: u8,
    /// `None` means `b_substream_bitrate_indicator == 0`.
    pub substream_bitrate_indicator: Option<u8>,
    /// 18 bit `dsi_substream_channel_groups` mask.
    pub channel_groups: Option<u32>,
}

fn write_content_type_dsi<W: BitWrite + ?Sized>(
    content_type: &Option<TocContentType>,
    w: &mut W,
) -> Result<(), Error> {
    w.write_bit(content_type.is_some())
        .context("b_content_type")?;

    let Some(content_type) = content_type else {
        return Ok(());
    };

    w.write::<3, u8>(content_type.content_classifier)
        .context("content_classifier")?;

    match &content_type.language_tag {
        None => w.write_bit(false).context("b_language_indicator")?,
        Some(tag) => {
            // Clause 4.3.3.8.6 reserves 0, 1 and 43..63 for
            // n_language_tag_bytes, so a valid language tag is 2..=42 bytes.
            if !(2..=42).contains(&tag.len()) {
                bail!("invalid AC-4 language tag length {}", tag.len());
            }
            w.write_bit(true).context("b_language_indicator")?;
            w.write::<6, u8>(tag.len() as u8)
                .context("n_language_tag_bytes")?;
            for byte in tag {
                w.write::<8, u8>(*byte).context("language_tag_bytes")?;
            }
        }
    }

    Ok(())
}

impl ToBitStream for Ac4PresentationV1Dsi {
    type Error = Error;

    fn to_writer<W: BitWrite + ?Sized>(&self, w: &mut W) -> Result<(), Self::Error> {
        w.write::<5, u8>(self.presentation_config_v1)
            .context("presentation_config_v1")?;

        // presentation_config_v1 == 0x06 means "EMDF only": jump straight to the
        // EMDF list, everything below belongs to the other configurations. (E.10.1)
        if self.presentation_config_v1 != 0x06 {
            w.write::<3, u8>(self.md_compat).context("md_compat")?;
            w.write_bit(self.b_presentation_id)
                .context("b_presentation_id")?;
            if self.b_presentation_id {
                w.write::<5, u8>(self.presentation_id)
                    .context("presentation_id")?;
            }
            w.write::<2, u8>(self.dsi_frame_rate_multiply_info)
                .context("dsi_frame_rate_multiply_info")?;
            w.write::<2, u8>(self.dsi_frame_rate_fraction_info)
                .context("dsi_frame_rate_fraction_info")?;
            w.write::<5, u8>(self.presentation_emdf_version)
                .context("presentation_emdf_version")?;
            w.write::<10, u16>(self.presentation_key_id)
                .context("presentation_key_id")?;

            w.write_bit(self.b_presentation_channel_coded)
                .context("b_presentation_channel_coded")?;
            if self.b_presentation_channel_coded {
                if self.dsi_presentation_ch_mode > 0x1f {
                    bail!(
                        "dsi_presentation_ch_mode {} does not fit in 5 bits",
                        self.dsi_presentation_ch_mode
                    );
                }
                w.write::<5, u8>(self.dsi_presentation_ch_mode)
                    .context("dsi_presentation_ch_mode")?;
                if matches!(self.dsi_presentation_ch_mode, 11..=14) {
                    w.write_bit(self.pres_b_4_back_channels_present)
                        .context("pres_b_4_back_channels_present")?;
                    w.write::<2, u8>(self.pres_top_channel_pairs)
                        .context("pres_top_channel_pairs")?;
                }
                if self.presentation_v1_channel_groups >= (1 << 18) {
                    bail!(
                        "presentation_v1_channel_groups 0x{:x} does not fit in 18 bits",
                        self.presentation_v1_channel_groups
                    );
                }
                w.write::<6, u8>(0).context("reserved_zero")?;
                w.write::<18, u32>(self.presentation_v1_channel_groups)
                    .context("presentation_v1_channel_groups")?;
            }

            w.write_bit(self.b_presentation_core_differs)
                .context("b_presentation_core_differs")?;
            if self.b_presentation_core_differs {
                w.write_bit(self.b_presentation_core_channel_coded)
                    .context("b_presentation_core_channel_coded")?;
                if self.b_presentation_core_channel_coded {
                    w.write::<2, u8>(self.dsi_presentation_channel_mode_core)
                        .context("dsi_presentation_channel_mode_core")?;
                }
            }

            w.write_bit(self.b_presentation_filter)
                .context("b_presentation_filter")?;
            if self.b_presentation_filter {
                w.write_bit(self.b_enable_presentation)
                    .context("b_enable_presentation")?;
                if self.filter_data.len() > 0xff {
                    bail!("filter_data too long: {}", self.filter_data.len());
                }
                w.write::<8, u8>(self.filter_data.len() as u8)
                    .context("n_filter_bytes")?;
                for byte in &self.filter_data {
                    w.write::<8, u8>(*byte).context("filter_data")?;
                }
            }

            if self.presentation_config_v1 == 0x1f {
                self.write_ac4_substream_group_dsi(w, 0)?;
            } else {
                w.write_bit(self.b_multi_pid).context("b_multi_pid")?;

                if matches!(self.presentation_config_v1, 0..=2) {
                    self.write_ac4_substream_group_dsi(w, 0)?;
                    self.write_ac4_substream_group_dsi(w, 1)?;
                }
                if matches!(self.presentation_config_v1, 3..=4) {
                    self.write_ac4_substream_group_dsi(w, 0)?;
                    self.write_ac4_substream_group_dsi(w, 1)?;
                    self.write_ac4_substream_group_dsi(w, 2)?;
                }
                if self.presentation_config_v1 == 5 {
                    // E.10.1: 3 bits hold n_substream_groups_minus2 with no
                    // escape, so only 2..=9 groups can be signalled.
                    let n_substream_groups = self.substream_groups.len();
                    if !(2..=9).contains(&n_substream_groups) {
                        bail!(
                            "presentation_config_v1 5 has {n_substream_groups} substream \
                             groups, but n_substream_groups_minus2 only covers 2..=9"
                        );
                    }
                    w.write::<3, u8>((n_substream_groups - 2) as u8)
                        .context("n_substream_groups_minus2")?;
                    for index in 0..n_substream_groups {
                        self.write_ac4_substream_group_dsi(w, index)?;
                    }
                }
                if self.presentation_config_v1 > 5 {
                    // 0x06 and 0x1f never get here, see the conditions above.
                    if self.skip_data.len() > 0x7f {
                        bail!("skip_data too long: {}", self.skip_data.len());
                    }
                    w.write::<7, u8>(self.skip_data.len() as u8)
                        .context("n_skip_bytes")?;
                    for byte in &self.skip_data {
                        w.write::<8, u8>(*byte).context("skip_data")?;
                    }
                }
            }

            w.write_bit(self.b_pre_virtualized)
                .context("b_pre_virtualized")?;
            w.write_bit(self.b_add_emdf_substreams)
                .context("b_add_emdf_substreams")?;
        }

        // presentation_config_v1 == 0x06 implies b_add_emdf_substreams (E.10.1, TS 103 190-2).
        if self.presentation_config_v1 == 0x06 || self.b_add_emdf_substreams {
            // n_add_emdf_substreams is a 7 bit field.
            if self.add_emdf.len() > 0x7f {
                bail!(
                    "too many additional EMDF substreams: {}",
                    self.add_emdf.len()
                );
            }
            w.write::<7, u8>(self.add_emdf.len() as u8)
                .context("n_add_emdf_substreams")?;
            for (emdf_version, key_id) in &self.add_emdf {
                w.write::<5, u8>(*emdf_version)
                    .context("substream_emdf_version")?;
                w.write::<10, u16>(*key_id).context("substream_key_id")?;
            }
        }

        w.write_bit(self.b_presentation_bitrate_info)
            .context("b_presentation_bitrate_info")?;
        if self.b_presentation_bitrate_info {
            let Some(bitrate) = &self.bitrate else {
                bail!("b_presentation_bitrate_info is set but no bitrate is available");
            };
            bitrate.to_writer(w).context("ac4_bitrate_dsi")?;
        }

        w.write_bit(self.b_alternative).context("b_alternative")?;
        if self.b_alternative {
            bail!("alternative_info() is not supported yet");
        }

        w.byte_align().context("byte_align")?;

        // E.10.1 guards this block with `bits_read() <= (pres_bytes - 1) * 8`.
        // We derive pres_bytes from this very serialization, so it always holds.
        w.write_bit(self.de_indicator).context("de_indicator")?;
        w.write_bit(self.immersive_audio_indicator)
            .context("immersive_audio_indicator")?;
        w.write::<4, u8>(0).context("reserved")?;
        w.write_bit(self.b_extended_presentation_id)
            .context("b_extended_presentation_id")?;
        if self.b_extended_presentation_id {
            w.write::<9, u16>(self.extended_presentation_id)
                .context("extended_presentation_id")?;
        } else {
            w.write_bit(false).context("reserved")?;
        }

        Ok(())
    }
}

impl Ac4PresentationV1Dsi {
    fn write_ac4_substream_group_dsi<W: BitWrite + ?Sized>(
        &self,
        w: &mut W,
        index: usize,
    ) -> Result<(), Error> {
        let Some(group) = self.substream_groups.get(index) else {
            bail!(
                "presentation_config_v1 0x{:x} needs substream group {index}, \
                 but only {} are present",
                self.presentation_config_v1,
                self.substream_groups.len()
            );
        };

        w.write_bit(group.b_substreams_present)
            .context("b_substreams_present")?;
        w.write_bit(group.b_hsf_ext).context("b_hsf_ext")?;
        w.write_bit(group.b_channel_coded)
            .context("b_channel_coded")?;
        if group.substreams.len() > 0xff {
            bail!("too many substreams: {}", group.substreams.len());
        }
        w.write::<8, u8>(group.substreams.len() as u8)
            .context("n_substreams")?;

        for substream in &group.substreams {
            w.write::<2, u8>(substream.dsi_sf_multiplier)
                .context("dsi_sf_multiplier")?;
            w.write_bit(substream.substream_bitrate_indicator.is_some())
                .context("b_substream_bitrate_indicator")?;
            if let Some(bitrate_indicator) = substream.substream_bitrate_indicator {
                // E.11 carries the brate_ind of table 90, i.e. 0..19.
                if bitrate_indicator > 19 {
                    bail!("substream_bitrate_indicator {bitrate_indicator} is out of range");
                }
                w.write::<5, u8>(bitrate_indicator)
                    .context("substream_bitrate_indicator")?;
            }

            if group.b_channel_coded {
                let Some(channel_groups) = substream.channel_groups else {
                    bail!("channel coded substream without dsi_substream_channel_groups");
                };
                if channel_groups >= (1 << 18) {
                    bail!(
                        "dsi_substream_channel_groups 0x{channel_groups:x} does not fit in 18 bits"
                    );
                }
                w.write::<6, u8>(0).context("reserved_zero")?;
                w.write::<18, u32>(channel_groups)
                    .context("dsi_substream_channel_groups")?;
            } else {
                bail!("object/A-JOC coded AC-4 substreams are not supported yet");
            }
        }

        write_content_type_dsi(&group.content_type, w)?;
        Ok(())
    }
}

impl PresentationDsi {
    fn presentation_version(&self) -> u8 {
        match self {
            Self::V1(_) => 1,
            Self::Other(version) => *version,
        }
    }
}

impl ToBitStream for PresentationDsi {
    type Error = Error;

    fn to_writer<W: BitWrite + ?Sized>(&self, w: &mut W) -> Result<(), Self::Error> {
        match self {
            Self::V1(p) => p.to_writer(w),
            // No DSI body: see PresentationDsi::Other.
            Self::Other(_) => Ok(()),
        }
    }
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
pub(crate) enum PresentationDsi {
    V1(Ac4PresentationV1Dsi),
    /// A `presentation_version` whose DSI is not defined by ETSI TS 103 190-2
    /// (e.g. 2 for immersive stereo). E.6.1 leaves `presentation_bytes` at 0 for
    /// those, i.e. nothing is written and `skip_area` would have to cover the
    /// whole `pres_bytes`.
    Other(u8),
}

#[derive(Debug, Clone)]
pub struct Dac4 {
    pub bitstream_version: u8,
    pub fs_index: u8,
    pub frame_rate_index: u8,
    pub n_presentations: Vec<PresentationDsi>,
    pub b_program_id: bool,
    pub b_program_uuid_present: bool,
    pub short_program_id: u16,
    pub program_uuid: [u8; 16],
    pub bitrate: Ac4BitrateDsi,
}

impl Dac4 {
    pub(crate) fn from_toc(toc: &Ac4Toc) -> Result<Self, Error> {
        let presentations = toc
            .presentations
            .iter()
            .map(|presentation| PresentationDsi::from_toc(presentation, &toc.groups))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            bitstream_version: toc.bitstream_version,
            fs_index: toc.fs_index,
            frame_rate_index: toc.frame_rate_index,
            n_presentations: presentations,
            b_program_id: toc.b_program_id,
            b_program_uuid_present: toc.b_program_uuid_present,
            short_program_id: toc.short_program_id,
            program_uuid: toc.program_uuid,
            bitrate: Ac4BitrateDsi {
                // E.7: constant / average / variable bit rate. wait_frames is only
                // present when b_wait_frames is set, otherwise the mode stays
                // "not specified".
                bit_rate_mode: if toc.b_wait_frames {
                    match toc.wait_frames {
                        0 => 1,
                        1..=6 => 2,
                        _ => 3,
                    }
                } else {
                    0
                },
                ..Ac4BitrateDsi::UNKNOWN
            },
        })
    }

    fn write_dsi<W: BitWrite + ?Sized>(&self, w: &mut W) -> Result<(), Error> {
        // ac4_dsi_version 1 selects ac4_dsi_v1() (table E.6).
        w.write::<3, u8>(1).context("ac4_dsi_version")?;
        w.write::<7, u8>(self.bitstream_version)
            .context("bitstream_version")?;
        w.write::<1, u8>(self.fs_index).context("fs_index")?;
        w.write::<4, u8>(self.frame_rate_index)
            .context("frame_rate_index")?;
        let n_presentations = u16::try_from(self.n_presentations.len())
            .ok()
            .filter(|n_presentations| *n_presentations < 0x200)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "{} presentations do not fit in 9 bits",
                    self.n_presentations.len()
                )
            })?;
        w.write::<9, u16>(n_presentations)
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
        }
        self.bitrate.to_writer(w).context("ac4_bitrate_dsi")?;
        w.byte_align().context("byte_align")?;
        for presentation in &self.n_presentations {
            // Serialize first: pres_bytes counts the bytes that follow it (E.6.1).
            let mut payload = Vec::new();
            {
                let mut bw = bitstream_io::BitWriter::endian(&mut payload, bitstream_io::BigEndian);
                presentation
                    .to_writer(&mut bw)
                    .context("presentation_bytes")?;
                bw.byte_align().context("presentation byte_align")?;
            }

            // `pres_bytes == 255` escapes to the 16 bit add_pres_bytes, so 255
            // can never be written directly (E.6.1).
            let (pres_bytes, add_pres_bytes) = if payload.len() >= 0xff {
                let extra = payload.len() - 0xff;
                let extra = u16::try_from(extra).map_err(|_| {
                    anyhow::anyhow!(
                        "presentation DSI of {} bytes does not fit in pres_bytes",
                        payload.len()
                    )
                })?;
                (0xffu8, Some(extra))
            } else {
                (payload.len() as u8, None)
            };

            w.write::<8, u8>(presentation.presentation_version())
                .context("presentation_version")?;
            w.write::<8, u8>(pres_bytes).context("pres_bytes")?;
            if let Some(add_pres_bytes) = add_pres_bytes {
                w.write::<16, u16>(add_pres_bytes)
                    .context("add_pres_bytes")?;
            }
            w.write_bytes(&payload).context("presentation_bytes")?;
        }
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

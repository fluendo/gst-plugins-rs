// Copyright (C) 2025, Fluendo S.A.
//      Author: Diego Nieto <dnieto@fluendo.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v2.0.
// If a copy of the MPL was not distributed with this file, You can obtain one at
// <https://mozilla.org/MPL/2.0/>.
//
// SPDX-License-Identifier: MPL-2.0

use std::collections::VecDeque;
use std::time::Instant;
use std::time::Duration;
use std::fmt;
use std::sync::LazyLock;

use gst::ffi::GstClockTime;

use procfs::process::Process;

static CAT: LazyLock<gst::DebugCategory> = LazyLock::new(|| {
    gst::DebugCategory::new(
        "VideoEncoderStats",
        gst::DebugColorFlags::empty(),
        Some("VideoEncoderStats"),
    )
});

#[derive(Clone, PartialEq, Debug)]
pub struct VideoEncoderStats {
    pub name: String,
    pub num_buffers: u64,
    pub num_bytes: u64,
    pub time_last_buffers: VecDeque<Instant>,
    pub max_buffers_inside: usize,
    pub total_processing_time: Duration,
    pub threads_utime: u64,
    pub threads_stime: u64,
    pub framerate: Option<gst::Fraction>,
    pub vmaf_score: Option<f64>,
    pub input_time: GstClockTime,
    pub pre_encode_time: GstClockTime,
    pub post_encode_time: GstClockTime,
}

impl Default for VideoEncoderStats {
    fn default() -> Self {
        Self {
            name: String::new(),
            framerate: None,
            num_bytes: 0,
            num_buffers: 0,
            time_last_buffers: VecDeque::<Instant>::new(),
            max_buffers_inside: 0,
            total_processing_time: Duration::ZERO,
            threads_utime: 0,
            threads_stime: 0,
            vmaf_score: None,
            input_time: 0,
            pre_encode_time: 0,
            post_encode_time: 0,
        }
    }
}

impl VideoEncoderStats {
    pub fn buffer_in(&mut self) {
        self.time_last_buffers.push_back(Instant::now());
        if self.time_last_buffers.len() > self.max_buffers_inside {
            self.max_buffers_inside = self.time_last_buffers.len();
        }
        gst::log!(CAT, "Current buffers lenght {}", self.time_last_buffers.len());
    }

    pub fn buffer_out(&mut self) {
        if let Some(arrive) = self.time_last_buffers.pop_front() {
            let diff = arrive.elapsed();
            self.total_processing_time += diff;
        } else {
            panic!("output buffer w/o input");
        }
    }

    pub fn avg_processing_time(&self) -> Duration {
        if self.num_buffers != 0 {
            self.total_processing_time / self.num_buffers as u32
        } else {
            Duration::ZERO
        }
    }
}

impl fmt::Display for VideoEncoderStats {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.framerate.unwrap().denom() != 1 {
            unimplemented!();
        }

        writeln!(
            f,
            "Encoder: {}",
            &self.name
        )?;
        writeln!(
            f,
            "Output size: {} KB",
            self.num_bytes / 1000, // Convert to KB
        )?;
        writeln!(
            f,
            "Max. Buffers inside: {}",
            self.max_buffers_inside
        )?;

        let framerate = self.framerate.unwrap();
        let total_time_secs = self.num_buffers as f64 / framerate.numer() as f64;
        let bitrate = if total_time_secs > 0.0 {
            (self.num_bytes as f64 * 8.0) / total_time_secs
        } else {
            0.0
        };
        let bitrate_str = bitrate/1000.0; // Convert to kbps

        writeln!(f, "Bitrate: {:.3} kbps", bitrate_str)?;

        let avg_processing_time = self.avg_processing_time().as_millis();
        writeln!(
            f,
            "Processing time: {:.2} ms",
            avg_processing_time
        )?;

        let cpu_time = self.threads_utime + self.threads_stime;
        #[cfg(target_os = "linux")]
        let cpu_time_seconds = {
            let ticks_per_second = procfs::ticks_per_second() as u64;
            cpu_time as f64 / ticks_per_second as f64
        };
        writeln!(
            f,
            "CPU: {} s",
            cpu_time_seconds
        )?;

        let vmaf_score_str = match self.vmaf_score {
            Some(score) => format!("{:.3}", score),
            None => "N/A".to_string(),
        };
        writeln!(f, "VMAF: {}", vmaf_score_str)?;

        let pre_encode_time = &self.pre_encode_time;
        let post_encode_time = &self.post_encode_time;
        let encode_latency = (*post_encode_time as f64 - *pre_encode_time as f64) / 1_000_000.0;
        writeln!(
            f,
            "Encode latency: {:.3} ms",
            encode_latency
        )
    }
}

#[cfg(target_os = "linux")]
pub fn get_cpu_usage(name: String) -> (u64, u64) {
    let my_pid = std::process::id() as i32;
    let process = Process::new(my_pid).unwrap();

    let mut total_utime: u64 = 0;
    let mut total_stime: u64 = 0;

    for thread in process.tasks().unwrap().flatten() {
        let stat = thread.stat().unwrap();
        if stat.comm.contains(&name) {
            gst::log!(CAT, "Thread: {}, Comm: {}, Utime: {}, Stime: {}", thread.tid, stat.comm, stat.utime, stat.stime);
            total_utime += stat.utime;
            total_stime += stat.stime;
        }
    }

    (total_utime, total_stime)
}

#[cfg(not(target_os = "linux"))]
pub fn get_cpu_usage(name: String) -> (u64, u64) {
    (0, 0)
}

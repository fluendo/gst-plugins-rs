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

use procfs::process::Process;
use human_bytes::human_bytes;

#[derive(Default, Clone, PartialEq, Debug)]
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
    pub vmaf_score: f64,
}

impl VideoEncoderStats {
    pub fn buffer_in(&mut self) {
        self.time_last_buffers.push_back(Instant::now());
        if self.time_last_buffers.len() > self.max_buffers_inside {
            self.max_buffers_inside = self.time_last_buffers.len();
        }
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
            "Buffers: {}",
            self.num_buffers,
        )?;
        writeln!(
            f,
            "Bytes: {}",
            self.num_bytes,
        )?;

        let framerate = self.framerate.unwrap();
        let total_time_secs = self.num_buffers as f64 / framerate.numer() as f64;
        let bitrate = if total_time_secs > 0.0 {
            (self.num_bytes as f64 * 8.0) / total_time_secs
        } else {
            0.0
        };
        let bitrate_str = human_bytes(bitrate);

        writeln!(f, "Bitrate: {}b/s", bitrate_str)?;

        let processing_time = self.avg_processing_time();
        writeln!(
            f,
            "Processing time: {:?}",
            processing_time
        )?;

        let cpu_time = self.threads_utime + self.threads_stime;
        writeln!(
            f,
            "CPU time: {}",
            cpu_time
        )?;

        let vmaf_score = self.vmaf_score;
        writeln!(
            f,
            "VMAF score: {:.3}",
            vmaf_score
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
        // FIXME
        //println!("Thread: {}, Comm: {}, Utime: {}, Stime: {}", thread.tid, stat.comm, stat.utime, stat.stime);
        if stat.comm == name {
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

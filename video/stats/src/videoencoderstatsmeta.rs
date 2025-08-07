// Copyright (C) 2025, Fluendo S.A.
//      Author: Diego Nieto <dnieto@fluendo.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v2.0.
// If a copy of the MPL was not distributed with this file, You can obtain one at
// <https://mozilla.org/MPL/2.0/>.
//
// SPDX-License-Identifier: MPL-2.0

use gst::prelude::*;
use std::fmt;
use std::mem;

use crate::videoencoderstats::*;

#[repr(transparent)]
pub struct VideoEncoderStatsMeta(imp::VideoEncoderStatsMeta);

unsafe impl Send for VideoEncoderStatsMeta {}
unsafe impl Sync for VideoEncoderStatsMeta {}

impl VideoEncoderStatsMeta {
    pub fn add(
        buffer: &mut gst::BufferRef,
        stats: VideoEncoderStats,
    ) -> gst::MetaRefMut<'_, Self, gst::meta::Standalone> {
        unsafe {
            let mut params =
                mem::ManuallyDrop::new(imp::VideoEncoderStatsMetaParams { stats });

            let meta = gst::ffi::gst_buffer_add_meta(
                buffer.as_mut_ptr(),
                imp::video_encoder_stats_meta_get_info(),
                &mut *params as *mut imp::VideoEncoderStatsMetaParams as gst::glib::ffi::gpointer,
            ) as *mut imp::VideoEncoderStatsMeta;

            Self::from_mut_ptr(buffer, meta)
        }
    }

    pub fn stats(&self) -> &VideoEncoderStats {
        &self.0.stats
    }
}

unsafe impl MetaAPI for VideoEncoderStatsMeta {
    type GstType = imp::VideoEncoderStatsMeta;

    fn meta_api() -> gst::glib::Type {
        imp::video_encoder_stats_meta_api_get_type()
    }
}

impl fmt::Debug for VideoEncoderStatsMeta {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("VideoEncoderStatsMeta")
            .finish()
    }
}

mod imp {
    use gst::glib::translate::*;
    use std::mem;
    use std::ptr;
    use std::sync::LazyLock;

    pub(super) struct VideoEncoderStatsMetaParams {
        pub stats: super::VideoEncoderStats,
    }

    #[repr(C)]
    pub struct VideoEncoderStatsMeta {
        parent: gst::ffi::GstMeta,
        pub(super) stats: super::VideoEncoderStats,
    }

    pub(super) fn video_encoder_stats_meta_api_get_type() -> glib::Type {
        static TYPE: LazyLock<glib::Type> = LazyLock::new(|| unsafe {
            let t = from_glib(gst::ffi::gst_meta_api_type_register(
                c"GstVideoEncoderStatsMetaAPI".as_ptr() as *const _,
                [ptr::null::<std::os::raw::c_char>()].as_ptr() as *mut *const _,
            ));

            assert_ne!(t, glib::Type::INVALID);

            t
        });

        *TYPE
    }

    unsafe extern "C" fn video_encoder_stats_meta_init(
        meta: *mut gst::ffi::GstMeta,
        params: glib::ffi::gpointer,
        _buffer: *mut gst::ffi::GstBuffer,
    ) -> glib::ffi::gboolean {
        assert!(!params.is_null());
        let meta = &mut *(meta as *mut VideoEncoderStatsMeta);
        let params = ptr::read(params as *const VideoEncoderStatsMetaParams);

        let VideoEncoderStatsMetaParams { stats } = params;

        ptr::write(&mut meta.stats, stats);

        true.into_glib()
    }

    unsafe extern "C" fn video_encoder_stats_meta_free(
        meta: *mut gst::ffi::GstMeta,
        _buffer: *mut gst::ffi::GstBuffer,
    ) {
        let meta = &mut *(meta as *mut VideoEncoderStatsMeta);
        meta.stats = super::VideoEncoderStats::default();
    }

    unsafe extern "C" fn video_encoder_stats_meta_transform(
        dest: *mut gst::ffi::GstBuffer,
        meta: *mut gst::ffi::GstMeta,
        _buffer: *mut gst::ffi::GstBuffer,
        _type_: glib::ffi::GQuark,
        _data: glib::ffi::gpointer,
    ) -> glib::ffi::gboolean {
        let dest = gst::BufferRef::from_mut_ptr(dest);
        let meta = &*(meta as *const VideoEncoderStatsMeta);

        if dest.meta::<super::VideoEncoderStatsMeta>().is_some() {
            return true.into_glib();
        }
        super::VideoEncoderStatsMeta::add(
            dest,
            meta.stats.clone(),
        );

        true.into_glib()
    }

    pub(super) fn video_encoder_stats_meta_get_info() -> *const gst::ffi::GstMetaInfo {
        struct MetaInfo(ptr::NonNull<gst::ffi::GstMetaInfo>);
        unsafe impl Send for MetaInfo {}
        unsafe impl Sync for MetaInfo {}

        static META_INFO: LazyLock<MetaInfo> = LazyLock::new(|| unsafe {
            MetaInfo(
                ptr::NonNull::new(gst::ffi::gst_meta_register(
                    video_encoder_stats_meta_api_get_type().into_glib(),
                    c"VideoEncoderStatsMeta".as_ptr() as *const _,
                    mem::size_of::<VideoEncoderStatsMeta>(),
                    Some(video_encoder_stats_meta_init),
                    Some(video_encoder_stats_meta_free),
                    Some(video_encoder_stats_meta_transform),
                ) as *mut gst::ffi::GstMetaInfo)
                .expect("Failed to register meta API"),
            )
        });

        META_INFO.0.as_ptr()
    }
}

#[test]
fn test() {
    gst::init().unwrap();
    let stats = VideoEncoderStats {
        name: "test_encoder".to_string(),
        num_buffers: 8,
        num_bytes: 16,
        time_last_buffers: std::collections::VecDeque::new(),
        max_buffers_inside: 5,
        total_processing_time: std::time::Duration::ZERO,
        threads_utime: 0,
        threads_stime: 0,
        framerate: None,
    };
    let mut b = gst::Buffer::with_size(10).unwrap();
    let m = VideoEncoderStatsMeta::add(b.make_mut(), stats.clone());
    assert_eq!(m.stats().name, "test_encoder");
    let b2: gst::Buffer = b.copy_deep().unwrap();
    let m = b.meta::<VideoEncoderStatsMeta>().unwrap();
    assert_eq!(m.stats().num_buffers, 8);
    let m = b2.meta::<VideoEncoderStatsMeta>().unwrap();
    assert_eq!(m.stats(), &stats);
    let b3: gst::Buffer = b2.copy_deep().unwrap();
    drop(b2);
    let m = b3.meta::<VideoEncoderStatsMeta>().unwrap();
    assert_eq!(m.stats(), &stats);
}

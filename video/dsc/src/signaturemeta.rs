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

#[repr(transparent)]
pub struct SignatureMeta(imp::SignatureMeta);

unsafe impl Send for SignatureMeta {}
unsafe impl Sync for SignatureMeta {}

impl SignatureMeta {
    pub fn add<'a>(
        buffer: &'a mut gst::BufferRef,
        signature: &[u8],
    ) -> gst::MetaRefMut<'a, Self, gst::meta::Standalone> {
        unsafe {
            let mut params = mem::ManuallyDrop::new(imp::SignatureMetaParams {
                signature: signature.to_vec(),
            });

            let meta = gst::ffi::gst_buffer_add_meta(
                buffer.as_mut_ptr(),
                imp::signature_meta_get_info(),
                &mut *params as *mut imp::SignatureMetaParams as gst::glib::ffi::gpointer,
            ) as *mut imp::SignatureMeta;

            Self::from_mut_ptr(buffer, meta)
        }
    }

    pub fn signature(&self) -> &[u8] {
        &self.0.signature
    }
}

unsafe impl MetaAPI for SignatureMeta {
    type GstType = imp::SignatureMeta;

    fn meta_api() -> gst::glib::Type {
        imp::signature_meta_api_get_type()
    }
}

impl fmt::Debug for SignatureMeta {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("SignatureMeta")
            .field("signature_len", &self.0.signature.len())
            .finish()
    }
}

pub fn add_signature_meta(buffer: &mut gst::BufferRef, signature: &[u8]) {
    SignatureMeta::add(buffer, signature);
}

pub fn register_signature_meta() {
    // Registration happens automatically when the type is first accessed
    let _ = imp::signature_meta_api_get_type();
}

mod imp {
    use gst::glib::translate::*;
    use std::mem;
    use std::ptr;
    use std::sync::LazyLock;

    pub(super) struct SignatureMetaParams {
        pub signature: Vec<u8>,
    }

    #[repr(C)]
    pub struct SignatureMeta {
        parent: gst::ffi::GstMeta,
        pub(super) signature: Vec<u8>,
    }

    pub(super) fn signature_meta_api_get_type() -> glib::Type {
        static TYPE: LazyLock<glib::Type> = LazyLock::new(|| unsafe {
            let t = from_glib(gst::ffi::gst_meta_api_type_register(
                c"GstSignatureMetaAPI".as_ptr() as *const _,
                [
                    c"dsc-signature".as_ptr() as *const _,
                    c"video".as_ptr() as *const _,
                    ptr::null::<std::os::raw::c_char>(),
                ].as_ptr() as *mut *const _,
            ));

            assert_ne!(t, glib::Type::INVALID);

            t
        });

        *TYPE
    }

    unsafe extern "C" fn signature_meta_init(
        meta: *mut gst::ffi::GstMeta,
        params: glib::ffi::gpointer,
        _buffer: *mut gst::ffi::GstBuffer,
    ) -> glib::ffi::gboolean {
        assert!(!params.is_null());
        let meta = &mut *(meta as *mut SignatureMeta);
        let params = ptr::read(params as *const SignatureMetaParams);

        let SignatureMetaParams { signature } = params;

        ptr::write(&mut meta.signature, signature);

        true.into_glib()
    }

    unsafe extern "C" fn signature_meta_free(
        meta: *mut gst::ffi::GstMeta,
        _buffer: *mut gst::ffi::GstBuffer,
    ) {
        let meta = &mut *(meta as *mut SignatureMeta);
        ptr::drop_in_place(&mut meta.signature);
    }

    unsafe extern "C" fn signature_meta_transform(
        dest: *mut gst::ffi::GstBuffer,
        meta: *mut gst::ffi::GstMeta,
        _buffer: *mut gst::ffi::GstBuffer,
        _type_: glib::ffi::GQuark,
        _data: glib::ffi::gpointer,
    ) -> glib::ffi::gboolean {
        let dest = gst::BufferRef::from_mut_ptr(dest);
        let meta = &*(meta as *const SignatureMeta);

        if dest.meta::<super::SignatureMeta>().is_some() {
            return true.into_glib();
        }
        
        super::SignatureMeta::add(dest, &meta.signature);

        true.into_glib()
    }

    pub(super) fn signature_meta_get_info() -> *const gst::ffi::GstMetaInfo {
        struct MetaInfo(ptr::NonNull<gst::ffi::GstMetaInfo>);
        unsafe impl Send for MetaInfo {}
        unsafe impl Sync for MetaInfo {}

        static META_INFO: LazyLock<MetaInfo> = LazyLock::new(|| unsafe {
            MetaInfo(
                ptr::NonNull::new(gst::ffi::gst_meta_register(
                    signature_meta_api_get_type().into_glib(),
                    c"SignatureMeta".as_ptr() as *const _,
                    mem::size_of::<SignatureMeta>(),
                    Some(signature_meta_init),
                    Some(signature_meta_free),
                    Some(signature_meta_transform),
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
    let mut b = gst::Buffer::with_size(10).unwrap();
    let signature = vec![0x01, 0x02, 0x03, 0x04, 0x05];
    let m = SignatureMeta::add(b.make_mut(), &signature);
    assert_eq!(m.signature(), &signature[..]);
    
    let b2: gst::Buffer = b.copy_deep().unwrap();
    let m = b.meta::<SignatureMeta>().unwrap();
    assert_eq!(m.signature(), &signature[..]);
    
    let m = b2.meta::<SignatureMeta>().unwrap();
    assert_eq!(m.signature(), &signature[..]);
    
    let b3: gst::Buffer = b2.copy_deep().unwrap();
    drop(b2);
    let m = b3.meta::<SignatureMeta>().unwrap();
    assert_eq!(m.signature(), &signature[..]);
}
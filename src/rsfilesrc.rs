use libc::{c_char};
use std::ffi::{CStr, CString};
use std::ptr;
use std::u64;
use std::slice;
use std::io::{Read, Seek, SeekFrom};
use std::fs::File;
use std::path::PathBuf;
use url::Url;

use std::io::Write;

macro_rules! println_err(
    ($($arg:tt)*) => { {
        let r = writeln!(&mut ::std::io::stderr(), $($arg)*);
        r.expect("failed printing to stderr");
    } }
);

#[repr(C)]
pub enum GstFlowReturn {
    Ok = 0,
    NotLinked = -1,
    Flushing = -2,
    Eos = -3,
    NotNegotiated = -4,
    Error = -5,
}

#[repr(C)]
pub enum GBoolean {
    False = 0,
    True = 1,
}

impl GBoolean {
    fn from_bool(v: bool) -> GBoolean {
        match v {
            true => GBoolean::True,
            false => GBoolean::False,
        }
    }
}

#[derive(Debug)]
pub struct FileSrc {
    location: Option<PathBuf>,
    file: Option<File>,
    position: u64,
}

impl FileSrc {
    fn new() -> FileSrc {
        FileSrc { location: None, file: None, position: 0 }
    }

    fn set_uri(&mut self, uri_str: &Option<String>) -> bool {
        match *uri_str {
            None => {
                self.location = None;
                return true;
            },
            Some(ref uri_str) => {
                let uri_parsed = Url::parse(uri_str.as_str());
                match uri_parsed {
                    Ok(u) => {
                        match u.to_file_path().ok() {
                            Some(p) => {
                                self.location = Some(p);
                                return true;
                            },
                            None => {
                                self.location = None;
                                println_err!("Unsupported file URI '{}'", uri_str);
                                return false;
                            }
                        }
                    },
                    Err(err) => {
                        self.location = None;
                        println_err!("Failed to parse URI '{}': {}", uri_str, err);
                        return false;
                    }
                }
            }
        }
    }

    fn get_uri(&self) -> Option<String> {
        match self.location {
            None => None,
            Some(ref location) => Url::from_file_path(&location).map(|u| u.into_string()).ok()
        }
    }

    fn is_seekable(&self) -> bool {
        true
    }

    fn get_size(&self) -> u64 {
        match self.file {
            None => return u64::MAX,
            Some(ref f) => {
                return f.metadata().map(|m| m.len()).unwrap_or(u64::MAX);
            },
        }
    }

    fn start(&mut self) -> bool {
        self.file = None;
        self.position = 0;

        match self.location {
            None => return false,
            Some(ref location) => {
                match File::open(location.as_path()) {
                    Ok(file) => {
                        self.file = Some(file);
                        return true;
                    },
                    Err(err) => {
                        println_err!("Failed to open file '{}': {}", location.to_str().unwrap_or("Non-UTF8 path"), err.to_string());
                        return false;
                    },
                }
            },
        }
    }

    fn stop(&mut self) -> bool {
        self.file = None;
        self.position = 0;

        true
    }

    fn fill(&mut self, offset: u64, data: &mut [u8]) -> Result<usize, GstFlowReturn> {
        match self.file {
            None => return Err(GstFlowReturn::Error),
            Some(ref mut f) => {
                if self.position != offset {
                    match f.seek(SeekFrom::Start(offset)) {
                        Ok(_) => {
                            self.position = offset;
                        },
                        Err(err) => {
                            println_err!("Failed to seek to {}: {}", offset, err.to_string());
                            return Err(GstFlowReturn::Error);
                        }
                    }
                }

                match f.read(data) {
                    Ok(size) => {
                        self.position += size as u64;
                        return Ok(size)
                    },
                    Err(err) => {
                        println_err!("Failed to read at {}: {}", offset, err.to_string());
                        return Err(GstFlowReturn::Error);
                    },
                }
            },
        }
    }
}

#[no_mangle]
pub extern "C" fn filesrc_new() -> *mut FileSrc {
    let instance = Box::new(FileSrc::new());
    return Box::into_raw(instance);
}

#[no_mangle]
pub extern "C" fn filesrc_drop(ptr: *mut FileSrc) {
    unsafe { Box::from_raw(ptr) };
}

#[no_mangle]
pub extern "C" fn filesrc_set_uri(ptr: *mut FileSrc, uri_ptr: *const c_char) -> GBoolean{
    let filesrc: &mut FileSrc = unsafe { &mut *ptr };

    if uri_ptr.is_null() {
        GBoolean::from_bool(filesrc.set_uri(&None))
    } else {
        let uri = unsafe { CStr::from_ptr(uri_ptr) };
        GBoolean::from_bool(filesrc.set_uri(&Some(String::from(uri.to_str().unwrap()))))
    }
}

#[no_mangle]
pub extern "C" fn filesrc_get_uri(ptr: *mut FileSrc) -> *mut c_char {
    let filesrc: &mut FileSrc = unsafe { &mut *ptr };

    match filesrc.get_uri() {
        Some(ref uri) =>
            CString::new(uri.clone().into_bytes()).unwrap().into_raw(),
        None =>
            ptr::null_mut()
    }
}

#[no_mangle]
pub extern "C" fn filesrc_fill(ptr: *mut FileSrc, offset: u64, data_ptr: *mut u8, data_len_ptr: *mut usize) -> GstFlowReturn {
    let filesrc: &mut FileSrc = unsafe { &mut *ptr };

    let mut data_len: &mut usize = unsafe { &mut *data_len_ptr };
    let mut data = unsafe { slice::from_raw_parts_mut(data_ptr, *data_len) };
    match filesrc.fill(offset, data) {
        Ok(actual_len) => {
            *data_len = actual_len;
            GstFlowReturn::Ok
        },
        Err(ret) => ret,
    }
}

#[no_mangle]
pub extern "C" fn filesrc_get_size(ptr: *mut FileSrc) -> u64 {
    let filesrc: &mut FileSrc = unsafe { &mut *ptr };

    return filesrc.get_size();
}

#[no_mangle]
pub extern "C" fn filesrc_start(ptr: *mut FileSrc) -> GBoolean {
    let filesrc: &mut FileSrc = unsafe { &mut *ptr };

    GBoolean::from_bool(filesrc.start())
}

#[no_mangle]
pub extern "C" fn filesrc_stop(ptr: *mut FileSrc) -> GBoolean {
    let filesrc: &mut FileSrc = unsafe { &mut *ptr };

    GBoolean::from_bool(filesrc.stop())
}

#[no_mangle]
pub extern "C" fn filesrc_is_seekable(ptr: *mut FileSrc) -> GBoolean {
    let filesrc: &mut FileSrc = unsafe { &mut *ptr };

    GBoolean::from_bool(filesrc.is_seekable())
}


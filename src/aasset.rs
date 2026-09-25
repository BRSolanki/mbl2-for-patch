// Explanation: AAsset is NOT thread-safe anyway, so we don't add thread safety here either.
use crate::{loader::Buffer, LockResultExt};
use libc::{c_char, c_int, c_void, off64_t, off_t, size_t};
use ndk_sys::{AAsset, AAssetManager};
use rustc_hash::FxHashMap;
use std::{
    ffi::{CStr, OsStr},
    io::{self, Read, Seek},
    os::unix::ffi::OsStrExt,
    path::Path,
    sync::{LazyLock, Mutex},
};

// Newtype so raw AAsset pointers can be used as HashMap keys.
// All we do is compare the pointer value — the Mutex ensures exclusive access.
#[derive(PartialEq, Eq, Hash)]
struct AAssetPtr(*const ndk_sys::AAsset);
// SAFETY: we never dereference the pointer, only compare it as a usize key.
unsafe impl Send for AAssetPtr {}

/// Assets we have intercepted — maps AAsset* -> our in-memory buffer.
/// FxHashMap is used here because:
///   - Keys are pointer-sized integers — SipHash's DoS resistance is wasted.
///   - This is the hottest lock in the library (every AAsset call hits it).
///   - FxHash is ~2-3x faster than SipHash for integer/pointer keys.
/// `LazyLock<Mutex<_>>` already provides interior mutability, so no `static mut` needed.
static WANTED_ASSETS: LazyLock<Mutex<FxHashMap<AAssetPtr, Buffer>>> =
    LazyLock::new(|| Mutex::new(FxHashMap::default()));

// ── open ────────────────────────────────────────────────────────────────────

pub unsafe extern "C" fn open(
    man: *mut AAssetManager,
    fname: *const c_char,
    mode: c_int,
) -> *mut AAsset {
    let asset = ndk_sys::AAssetManager_open(man, fname, mode);
    let c_str = CStr::from_ptr(fname);
    let c_path = Path::new(OsStr::from_bytes(c_str.to_bytes()));

    // Try to serve this asset from the active resource pack instead of the APK.
    if let Some(buf) = crate::loader::get_pack_file(c_path) {
        WANTED_ASSETS.lock().ignore_poison().insert(AAssetPtr(asset), buf);
    }
    asset
}

// ── helpers ─────────────────────────────────────────────────────────────────

macro_rules! handle_result {
    ($expr:expr) => {
        match $expr {
            Ok(val) => val,
            Err(e) => {
                log::error!("{e}");
                return -1;
            }
        }
    };
}

#[inline]
fn seek_facade(offset: i64, whence: c_int, file: &mut Buffer) -> i64 {
    let seek_from = match whence {
        libc::SEEK_SET => {
            let u64_off = handle_result!(u64::try_from(offset));
            io::SeekFrom::Start(u64_off)
        }
        libc::SEEK_CUR => io::SeekFrom::Current(offset),
        libc::SEEK_END => io::SeekFrom::End(offset),
        _ => {
            log::error!("Invalid seek whence: {whence}");
            return -1;
        }
    };
    match file.seek(seek_from) {
        Ok(new_pos) => handle_result!(new_pos.try_into()),
        Err(e) => {
            log::error!("seek error: {e}");
            -1
        }
    }
}

// ── AAsset hook implementations ─────────────────────────────────────────────

pub unsafe extern "C" fn seek64(aasset: *mut AAsset, off: off64_t, whence: c_int) -> off64_t {
    let mut assets = WANTED_ASSETS.lock().ignore_poison();
    let Some(file) = assets.get_mut(&AAssetPtr(aasset)) else {
        return ndk_sys::AAsset_seek64(aasset, off, whence);
    };
    handle_result!(seek_facade(off, whence, file).try_into())
}

pub unsafe extern "C" fn seek(aasset: *mut AAsset, off: off_t, whence: c_int) -> off_t {
    let mut assets = WANTED_ASSETS.lock().ignore_poison();
    let Some(file) = assets.get_mut(&AAssetPtr(aasset)) else {
        return ndk_sys::AAsset_seek(aasset, off, whence);
    };
    handle_result!(seek_facade(off.into(), whence, file).try_into())
}

pub unsafe extern "C" fn read(aasset: *mut AAsset, buf: *mut c_void, count: size_t) -> c_int {
    let mut assets = WANTED_ASSETS.lock().ignore_poison();
    let Some(file) = assets.get_mut(&AAssetPtr(aasset)) else {
        return ndk_sys::AAsset_read(aasset, buf, count);
    };
    let rs_buf = core::slice::from_raw_parts_mut(buf as *mut u8, count);
    let n = handle_result!(file.read(rs_buf));
    handle_result!(n.try_into())
}

pub unsafe extern "C" fn len(aasset: *mut AAsset) -> off_t {
    let assets = WANTED_ASSETS.lock().ignore_poison();
    let Some(file) = assets.get(&AAssetPtr(aasset)) else {
        return ndk_sys::AAsset_getLength(aasset);
    };
    handle_result!(file.get_ref().as_ref().len().try_into())
}

pub unsafe extern "C" fn len64(aasset: *mut AAsset) -> off64_t {
    let assets = WANTED_ASSETS.lock().ignore_poison();
    let Some(file) = assets.get(&AAssetPtr(aasset)) else {
        return ndk_sys::AAsset_getLength64(aasset);
    };
    handle_result!(file.get_ref().as_ref().len().try_into())
}

pub unsafe extern "C" fn rem(aasset: *mut AAsset) -> off_t {
    let assets = WANTED_ASSETS.lock().ignore_poison();
    let Some(file) = assets.get(&AAssetPtr(aasset)) else {
        return ndk_sys::AAsset_getRemainingLength(aasset);
    };
    let total = file.get_ref().as_ref().len();
    // saturating_sub guards against position() > total (e.g. after SEEK_END + positive offset)
    handle_result!(total.saturating_sub(file.position() as usize).try_into())
}

pub unsafe extern "C" fn rem64(aasset: *mut AAsset) -> off64_t {
    let assets = WANTED_ASSETS.lock().ignore_poison();
    let Some(file) = assets.get(&AAssetPtr(aasset)) else {
        return ndk_sys::AAsset_getRemainingLength64(aasset);
    };
    let total = file.get_ref().as_ref().len();
    handle_result!(total.saturating_sub(file.position() as usize).try_into())
}

pub unsafe extern "C" fn close(aasset: *mut AAsset) {
    WANTED_ASSETS.lock().ignore_poison().remove(&AAssetPtr(aasset));
    ndk_sys::AAsset_close(aasset);
}

pub unsafe extern "C" fn get_buffer(aasset: *mut AAsset) -> *const c_void {
    let assets = WANTED_ASSETS.lock().ignore_poison();
    let Some(file) = assets.get(&AAssetPtr(aasset)) else {
        return ndk_sys::AAsset_getBuffer(aasset);
    };
    file.get_ref().as_ref().as_ptr().cast()
}

pub unsafe extern "C" fn fd_dummy(
    aasset: *mut AAsset,
    out_start: *mut off_t,
    out_len: *mut off_t,
) -> c_int {
    if WANTED_ASSETS.lock().ignore_poison().contains_key(&AAssetPtr(aasset)) {
        // We can't give a real file descriptor for an in-memory buffer
        -1
    } else {
        ndk_sys::AAsset_openFileDescriptor(aasset, out_start, out_len)
    }
}

pub unsafe extern "C" fn fd_dummy64(
    aasset: *mut AAsset,
    out_start: *mut off64_t,
    out_len: *mut off64_t,
) -> c_int {
    if WANTED_ASSETS.lock().ignore_poison().contains_key(&AAssetPtr(aasset)) {
        -1
    } else {
        ndk_sys::AAsset_openFileDescriptor64(aasset, out_start, out_len)
    }
}

pub unsafe extern "C" fn is_alloc(aasset: *mut AAsset) -> c_int {
    if WANTED_ASSETS.lock().ignore_poison().contains_key(&AAssetPtr(aasset)) {
        false as c_int
    } else {
        ndk_sys::AAsset_isAllocated(aasset)
    }
}

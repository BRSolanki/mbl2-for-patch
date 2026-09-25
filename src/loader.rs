use crate::{
    cpp_string::{ResourceLocation, StackString},
    LockResultExt,
};
use cxx::CxxString;
use std::{
    io::{Cursor, Read, Seek, Write},
    mem::transmute,
    os::unix::ffi::OsStrExt,
    path::Path,
    pin::Pin,
};

/// A loaded resource-pack file ready for reading.
pub type Buffer = Cursor<StackString>;

macro_rules! folder_list {
    ($( apk: $apk_folder:literal -> pack: $pack_folder:expr),
        *,
    ) => {
        [
            $(($apk_folder, $pack_folder)),*,
        ]
    }
}

/// Try to load `path` from the active resource pack.
/// Returns `None` if the path doesn't map to any resource-pack folder or the
/// file doesn't exist in the pack.
#[inline]
pub fn get_pack_file(path: &Path) -> Option<Buffer> {
    let stripped = path.strip_prefix("assets/").unwrap_or(path);
    let replacement_list = folder_list! {
        apk: "gui/dist/hbui/"              -> pack: "hbui/",
        apk: "skin_packs/persona/"         -> pack: "persona/",
        apk: "renderer/"                   -> pack: "renderer/",
        apk: "resource_packs/vanilla/cameras/" -> pack: "vanilla_cameras/",
    };
    for (apk_prefix, pack_prefix) in replacement_list {
        if let Ok(file) = stripped.strip_prefix(apk_prefix) {
            let mut resource_loc = ResourceLocation::new();
            let mut cpppath = ResourceLocation::get_path(&mut resource_loc);
            path_join_into(cpppath.as_mut(), &[Path::new(pack_prefix), file]);
            let packm = crate::PACKM_OBJ.lock().ignore_poison();
            let Some(packm) = packm.as_ref() else {
                log::error!("ResourcePackManager ptr is null");
                return None;
            };
            let Some(stack_str) = packm.load_resource(resource_loc) else {
                log::debug!("Not in pack: {}", cpppath.as_ref());
                return None;
            };
            log::debug!("Pack hit: {}", cpppath.as_ref());
            return Some(Cursor::new(stack_str));
        }
    }
    None
}

// This lint is not really applicable
#[allow(clippy::unused_io_amount)]
/// Write joined path segments directly into a C++ string, pre-reserving capacity.
fn path_join_into(mut out: Pin<&mut CxxString>, paths: &[&Path]) {
    let total_len: usize = paths.iter().map(|p| p.as_os_str().len()).sum();
    out.as_mut().reserve(total_len);
    for path in paths {
        out.write(path.as_os_str().as_bytes())
            .expect("Error while writing path into CxxString");
    }
}

pub struct ResourcePackManager(*mut libc::c_void);
unsafe impl Send for ResourcePackManager {}
impl ResourcePackManager {
    #[inline]
    pub fn wrap(ptr: *mut libc::c_void) -> Self {
        Self(ptr)
    }
    pub fn load_resource(&self, loc: ResourceLocation) -> Option<StackString> {
        // Walk vtable: vtable[2] is the load function
        let vptr = unsafe { *transmute::<*mut libc::c_void, *mut *mut *const u8>(self.0) };
        let loadfn = unsafe {
            transmute::<
                *const u8,
                unsafe extern "C" fn(
                    *mut libc::c_void,
                    ResourceLocation,
                    Pin<&mut CxxString>,
                ) -> bool,
            >(*vptr.offset(2))
        };
        let mut cxx_storage = StackString::new();
        let mut cxx_ptr = unsafe { cxx_storage.init("") };
        unsafe { loadfn(self.0, loc, cxx_ptr.as_mut()) };
        if cxx_ptr.is_empty() {
            None
        } else {
            Some(cxx_storage)
        }
    }
}

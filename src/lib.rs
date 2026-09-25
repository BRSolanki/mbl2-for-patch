#[deny(clippy::indexing_slicing)]
mod cpp_string;
mod loader;
mod aasset;
mod plthook;
use std::{
    fs,
    sync::{LockResult, Mutex},
};
use crate::{loader::ResourcePackManager, plthook::replace_plt_functions};
use bhook::hook_fn;
use bstr::ByteSlice;
use atoi::FromRadix16;
use plt_rs::DynamicLibrary;
use tinypatscan::Pattern;

#[cfg(target_arch = "aarch64")]
const RPMC_PATTERNS: [Pattern; 5] = [
    // v26.50
    Pattern::from_str("?? ?? ?? D1 ?? ?? ?? A9 ?? ?? ?? F9 ?? ?? ?? A9 ?? ?? ?? A9 ?? ?? ?? A9 ?? ?? ?? 91 ?? ?? ?? D5 F6 03 03 2A F5 03 02 AA ?? ?? ?? F9 F3 03 00 AA"),
    // v26.40
    Pattern::from_str("?? ?? ?? D1 ?? ?? ?? A9 ?? ?? ?? A9 ?? ?? ?? A9 ?? ?? ?? A9 ?? ?? ?? A9 ?? ?? ?? 91 ?? ?? ?? D5 F6 03 03 2A F5 03 02 AA ?? ?? ?? F9 F3 03 00 AA"),
    // 1.21.120.4
    Pattern::from_str("FF ?? 02 D1 FD 7B ?? A9 ?? ?? ?? ?? FA 67 ?? A9 F8 5F ?? A9 F6 57 ?? A9 F4 4F ?? A9 FD ?? 01 91 ?? D0 3B D5 ?? 03 03 2A ?? 03 02 AA ?? 17 40 F9 F3 03 00 AA A8 83 1F F8"),
    // 1.21.60.21
    Pattern::from_str("FF 83 02 D1 FD 7B 06 A9 FD 83 01 91 F8 5F 07 A9 F6 57 08 A9 F4 4F 09 A9 58 D0 3B D5 F6 03 03 2A 08 17 40 F9 F5 03 02 AA F3 03 00 AA A8 83 1F F8 28 10 40 F9 28 01 00 B4"),
    // 1.19.50 – 1.21.50
    Pattern::from_str("FF 03 03 D1 FD 7B 07 A9 FD C3 01 91 F9 43 00 F9 F8 5F 09 A9 F6 57 0A A9 F4 4F 0B A9 59 D0 3B D5 F6 03 03 2A 28 17 40 F9 F5 03 02 AA F3 03 00 AA A8 83 1F F8 28 10 40 F9"),
];

#[cfg(target_arch = "arm")]
const RPMC_PATTERNS: [Pattern; 2] = [
    // 1.21.120.4
    Pattern::from_str(
        "F0 B5 03 AF 2D E9 00 0F 8B B0 82 46 DF F8 ?? ?? 9B 46 91 46 78 44 00 68 00 68 0A 90",
    ),
    // 1.21.110 – 1.19.50
    Pattern::from_str(
        "F0 B5 03 AF 2D E9 00 ?? ?? B0 ?? 46 ?? 48 98 46 92 46 78 44 00 68 00 68 ?? 90 08 69",
    ),
];

#[cfg(target_arch = "x86_64")]
const RPMC_PATTERNS: [Pattern; 2] = [
    Pattern::from_str("55 41 57 41 56 41 55 41 54 53 48 83 EC ? 41 89 CF 49 89 D6 48 89 FB 64 48 8B 04 25 28 00 00 00 48 89 44 24 ? 48 8B 7E"),
    Pattern::from_str("55 41 57 41 56 53 48 83 EC ? 41 89 CF 49 89 D6 48 89 FB 64 48 8B 04 25 28 00 00 00 48 89 44 24 ? 48 8B 7E"),
];

pub fn setup_logging() {
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info),
    );
}

#[ctor::ctor]
fn safe_setup() {
    setup_logging();
    std::panic::set_hook(Box::new(|panic_info| {
        log::error!("Thread crashed: {}", panic_info);
    }));
    main();
}

fn main() {
    log::info!("Starting mbl2 v0.1.12");
    let mcmaps = match find_minecraft_library_manually() {
        Ok(m) => m,
        Err(e) => {
            log::error!("Cannot find libminecraftpe.so in memory maps: {e}");
            return;
        }
    };
    let Some(addr) = find_signatures(&RPMC_PATTERNS, &mcmaps) else {
        log::error!("No RPM signature matched — unsupported MC version, bailing out");
        return;
    };
    log::info!("Hooking ResourcePackManager constructor");
    unsafe { rpm_ctor::hook_address(addr as *mut u8) };
    log::info!("Hooking AssetManager functions");
    hook_aasset();
}

// ── /proc/self/maps parsing ──────────────────────────────────────────────────

/// Minimal memory map range (start addr + size).
struct SimpleMapRange {
    start: usize,
    size: usize,
}

fn find_minecraft_library_manually() -> Result<Vec<SimpleMapRange>, Box<dyn std::error::Error>> {
    let contents = fs::read("/proc/self/maps")?;
    let ranges: Vec<SimpleMapRange> = contents
        .lines()
        .filter_map(|line| {
            if line.trim_ascii().is_empty() {
                return None;
            }
            let (addr_start, addr_end) = parse_range(line)?;
            let start = usize::from_radix_16(addr_start).0;
            let end = usize::from_radix_16(addr_end).0;
            log::info!("Found libminecraftpe.so region: {:x}-{:x}", start, end);
            Some(SimpleMapRange { start, size: end - start })
        })
        .collect();

    if ranges.is_empty() {
        Err("libminecraftpe.so not found in memory maps".into())
    } else {
        Ok(ranges)
    }
}

/// Parse one line from /proc/self/maps, returning `(start_hex, end_hex)` bytes
/// only for executable regions belonging to libminecraftpe.so.
#[inline]
fn parse_range(buf: &[u8]) -> Option<(&[u8], &[u8])> {
    let mut fields = buf.split(|b| b.is_ascii_whitespace());
    let addr_range = fields.next()?;
    let perms = fields.next()?;
    let pathname = fields.next_back()?;
    if perms.contains(&b'x') && pathname.ends_with(b"libminecraftpe.so") {
        return addr_range.split_once_str(b"-");
    }
    None
}

// ── signature scanning ───────────────────────────────────────────────────────

fn find_signatures(signatures: &[Pattern], ranges: &[SimpleMapRange]) -> Option<*const u8> {
    for sig in signatures {
        for range in ranges {
            let lib_bytes =
                unsafe { core::slice::from_raw_parts(range.start as *const u8, range.size) };
            if let Some(offset) = sig.search(lib_bytes, tinypatscan::Algorithm::Simd) {
                let addr = unsafe { lib_bytes.as_ptr().byte_add(offset) };
                #[cfg(target_arch = "arm")]
                let addr = unsafe { addr.offset(1) };
                log::info!(
                    "Signature matched in {:x}-{:x} at offset {:x}",
                    range.start,
                    range.start + range.size,
                    offset
                );
                return Some(addr);
            }
        }
        log::error!("Signature not found in any region");
    }
    None
}

// ── PLT hooking ──────────────────────────────────────────────────────────────

macro_rules! cast_array {
    ($($func_name:literal -> $hook:expr),* $(,)?) => {
        [$(($func_name, $hook as *const u8)),*]
    }
}

/// Hook all AAsset* PLT entries in libminecraftpe.so.
fn hook_aasset() {
    let lib_entry = match find_lib("libminecraftpe") {
        Some(e) => e,
        None => {
            log::error!("Cannot find libminecraftpe in loaded modules — PLT hooks skipped");
            return;
        }
    };
    let dyn_lib = match DynamicLibrary::initialize(lib_entry) {
        Ok(d) => d,
        Err(e) => {
            log::error!("Failed to parse libminecraftpe ELF: {e:?} — PLT hooks skipped");
            return;
        }
    };
    let asset_fn_list = cast_array! {
        "AAssetManager_open"         -> aasset::open,
        "AAsset_read"                -> aasset::read,
        "AAsset_close"               -> aasset::close,
        "AAsset_seek"                -> aasset::seek,
        "AAsset_seek64"              -> aasset::seek64,
        "AAsset_getLength"           -> aasset::len,
        "AAsset_getLength64"         -> aasset::len64,
        "AAsset_getRemainingLength"  -> aasset::rem,
        "AAsset_getRemainingLength64"-> aasset::rem64,
        "AAsset_openFileDescriptor"  -> aasset::fd_dummy,
        "AAsset_openFileDescriptor64"-> aasset::fd_dummy64,
        "AAsset_getBuffer"           -> aasset::get_buffer,
        "AAsset_isAllocated"         -> aasset::is_alloc,
    };
    replace_plt_functions(&dyn_lib, asset_fn_list);
}

fn find_lib<'a>(target_name: &str) -> Option<plt_rs::LoadedLibrary<'a>> {
    plt_rs::collect_modules()
        .into_iter()
        .find(|lib| lib.name().contains(target_name))
}

// ── Global state ─────────────────────────────────────────────────────────────

/// The captured ResourcePackManager instance, set once by the rpm_ctor hook.
pub static PACKM_OBJ: Mutex<Option<ResourcePackManager>> = Mutex::new(None);

hook_fn! {
    fn rpm_ctor(this: *mut libc::c_void, unk1: usize, unk2: usize, needs_init: bool) -> *mut libc::c_void = {
        use crate::loader::ResourcePackManager;
        use crate::LockResultExt;
        log::info!("rpm_ctor called");
        let result = call_original(this, unk1, unk2, needs_init);
        *crate::PACKM_OBJ.lock().ignore_poison() = Some(ResourcePackManager::wrap(this));
        // Disable the hook — we only need the pointer once
        self_disable();
        log::info!("rpm_ctor done");
        result
    }
}

// ── Utilities ────────────────────────────────────────────────────────────────

pub trait LockResultExt {
    type Guard;
    fn ignore_poison(self) -> Self::Guard;
}

impl<Guard> LockResultExt for LockResult<Guard> {
    type Guard = Guard;
    fn ignore_poison(self) -> Guard {
        self.unwrap_or_else(|e| e.into_inner())
    }
}

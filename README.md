# MaterialBinLoader 2
A well optimized loader for the block game, stripped of MB Loader app JNI bindings and optimized for direct Smali injection into the Minecraft APK.

# Features
- Supports loading materialbins, camera files, and replacing existing oreui files
- Low overhead on every asset call
- No crashes on unsupported MC versions — fails soft, not hard
- No JNI / MB Loader app bindings — designed for direct Smali injection into the Minecraft APK
- Low size

> [!NOTE]
> This fork has the MB Loader app JNI layer removed (`addCustomFile`, `setAutofixVersions`, etc.).
> It is intended to be injected directly into the Minecraft APK via Smali, **not** loaded through an external launcher app.

> [!CAUTION]
> MBL2 is not responsible for what happens to your shaders after a Minecraft update.
> If your shader breaks, please update it from the shader developer's page.

# What was changed vs original MBL2

## Removed
| Removed | Reason |
|---|---|
| `jniopts.rs` (all JNI exports) | Only needed by MB Loader app — bloat & unused symbols when Smali-injected |
| `jni` crate dependency | No longer needed without JNI exports |
| `once_cell`, `scroll`, `page_size`, `memchr` deps | All unused in source — dead weight on binary size |
| `FAFAFILES` custom-file injection path | MB Loader app feature — ran a mutex lock + O(n) scan on **every** asset open call |
| `BufferCursor` enum | Single-variant enum — pure indirection overhead; replaced with `Cursor<StackString>` directly |
| `Buffer` newtype + `Deref`/`DerefMut` | Unnecessary wrapper; now a plain `type Buffer = Cursor<StackString>` |
| `FileLoader` struct + `MC_FILELOADER` static | Zero-state struct in a `LazyLock` — pointless global mutex init; replaced with a free function |
| `static mut` on `WANTED_ASSETS` | `LazyLock<Mutex<_>>` already provides interior mutability — `static mut` was wrong and unsafe |
| Autofix / lightmap / texture LOD stubs | Were no-ops; dead code in Smali context |
| Log level `Trace` → `Info` | `debug!`/`trace!` format strings bloat the binary and spam logcat |

## Fixed
| Fixed | Details |
|---|---|
| **Crashes on unsupported MC versions** | All `.expect()` panics in `main()` and `hook_aasset()` replaced with graceful early-returns. With `panic=abort`, any unmatched signature or missing library would take down the entire Minecraft process. |
| Memory leak in `AAsset_close` | `WANTED_ASSETS` HashMap entries were never removed, causing unbounded growth during a session |
| **arm32 PLT hooks never worked** | `get_function_table` (32-bit) always returned `None` even after populating the hashmap — shaders never loaded on arm32 |
| `get_buffer` held exclusive lock unnecessarily | Changed from `get_mut` to `get` — avoids exclusive lock on a read-only path |
| `rem`/`rem64` silent integer underflow | Used `saturating_sub` instead of bare `-`. A past-end `SEEK_END` seek could make `position() > total`, causing a wrapping underflow and garbage return value |

## Optimized (runtime performance)
| Change | Effect |
|---|---|
| `HashMap` → `FxHashMap` (rustc-hash) | Keys are raw pointers (integer-sized). SipHash's DoS resistance is wasted here. FxHash is ~2-3x faster for pointer/integer keys — this runs on every single AAsset call in the game |
| `opt-level = 3` (was `"z"`) | Prioritize speed over size. LTO + `codegen-units=1` already handles dead-code elimination so the size impact is small |

## Optimized (binary size)
| Change | Effect |
|---|---|
| `lto = true` + `codegen-units = 1` | Dead code stripped across all crates at link time |
| `strip = true` | Debug symbols removed from final `.so` |
| Log level `Trace` → `Info` | All `debug!`/`trace!` format strings compiled out entirely |
| Removed 4 unused direct dependencies | Smaller dependency graph = smaller binary |

# Supported platforms
- Android arm64
- Android arm32 *(PLT hook bug fixed — hooks now actually apply)*
- Chromeos/android x86_64 (untested)

# Building
## Requirements
- Rust (latest as possible)
- Your target android architecture's rust target installed
- Ndk r25+ installed

## Building the .so
``` bash
cargo build --release --target {android target triple here}
```

## Injecting via Smali
Load the compiled `.so` from your Smali patch using `System.loadLibrary` or `Runtime.loadLibrary`.
The library initializes itself automatically via `#[ctor]` — no explicit Java/JNI call is needed.
If the MC version is unsupported (no signature match), the library exits cleanly without crashing the game.

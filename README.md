# MaterialBinLoader 2
A well optimized loader for the block game, stripped of MB Loader app JNI bindings and optimized for direct Smali injection into the Minecraft APK.

# Features
- Supports loading materialbins, camera files, and replacing existing oreui files
- Low file loading overhead
- Highly modifiable sourcecode
- Low size (300-450kb)
- No JNI / MB Loader app bindings — designed for direct Smali injection into the Minecraft APK

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
| `FAFAFILES` custom-file injection path | MB Loader app feature — ran a mutex lock + linear scan on **every** asset open call |
| `BufferCursor` enum | Single-variant enum — pure indirection overhead, replaced with `Cursor<StackString>` directly |
| `Buffer` newtype + `Deref`/`DerefMut` | Unnecessary wrapper, now a plain `type Buffer = Cursor<StackString>` |
| `FileLoader` struct + `MC_FILELOADER` static | Zero-state struct wrapped in a `LazyLock` — pointless global mutex init; replaced with a free function |
| Autofix / lightmap / texture LOD stubs | Were no-ops; dead code in Smali context |
| Log level `Trace` → `Info` | `debug!`/`trace!` format strings bloat the binary and spam logcat |

## Fixed
| Fixed | Details |
|---|---|
| Memory leak in `AAsset_close` | `WANTED_ASSETS` HashMap entries were never removed, causing unbounded growth during a session |
| **arm32 PLT hooks never worked** | `get_function_table` (32-bit) always returned `None` even after populating the hashmap — shaders never loaded on arm32 |
| `get_buffer` held exclusive lock unnecessarily | Changed from `get_mut` to `get` — avoids exclusive lock on a read-only path |

## Optimized (binary size)
| Change | Effect |
|---|---|
| `opt-level = "z"` | LLVM optimizes for minimum size |
| `lto = true` + `codegen-units = 1` | Dead code stripped across all crates at link time |
| `strip = true` | Debug symbols removed from final `.so` |
| Log level `Trace` → `Info` | All `debug!`/`trace!` format strings compiled out entirely |

# Supported platforms
- Android arm64
- Android arm32 *(PLT hook bug fixed — now actually works)*
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

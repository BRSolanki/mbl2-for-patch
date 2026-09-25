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

# What was removed vs original MBL2
| Removed | Reason |
|---|---|
| `jniopts.rs` (all JNI exports) | Only needed by MB Loader app — causes bloat & unused symbol overhead when Smali-injected |
| `jni` crate dependency | No longer needed without JNI exports |
| `FAFAFILES` custom-file injection path | MB Loader app feature — ran a mutex lock + linear scan on **every** asset open call even when empty |
| Autofix / lightmap / texture LOD stubs | Were no-ops; dead code in Smali injection context |

# What was fixed vs original MBL2
| Fixed | Details |
|---|---|
| Memory leak in `AAsset_close` | `WANTED_ASSETS` HashMap entries were never removed on close, causing unbounded growth during a session |

# Supported platforms
- Android arm64
- Android arm32
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

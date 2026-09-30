# MaterialBinLoader 2
A well optimized loader for the block game, stripped of MB Loader app JNI bindings and optimized for direct injection into the Minecraft APK.

# Features
- Supports loading materialbins, camera files, and replacing existing oreui files
- Low overhead on every asset call
- No crashes on unsupported MC versions — fails soft, not hard
- No JNI / MB Loader app bindings — designed for direct injection into the Minecraft APK
- Two injection methods: **patchelf** (recommended) or **smali**
- Low size

> [!NOTE]
> This fork has the MB Loader app JNI layer removed (`addCustomFile`, `setAutofixVersions`, etc.).
> It is intended to be injected directly into the Minecraft APK, **not** loaded through an external launcher app.

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

# Injection

There are two methods to inject MBL2 into a Minecraft APK. **Patchelf is recommended** — it doesn't touch Java/dex code, so it can't break popups, Xbox sign-in, or Activity lifecycle behavior.

The library initializes itself automatically via `#[ctor]` — no explicit Java/JNI call is needed.
If the MC version is unsupported (no signature match), the library exits cleanly without crashing the game.

## Method 1: Patchelf (Recommended)

Add `libfusembl2.so` as a `DT_NEEDED` dependency to `libminecraftpe.so`. The Android linker will load it automatically — **no smali/dex modification required**.

### Steps

1. **Build** the `.so` (see above) or grab a release binary.

2. **Copy** the compiled `libfusembl2.so` into the APK's native lib directory:
   ```
   lib/arm64-v8a/libfusembl2.so    # for arm64
   lib/armeabi-v7a/libfusembl2.so  # for arm32
   ```

3. **Patch** `libminecraftpe.so` to depend on it:
   ```bash
   patchelf --add-needed libfusembl2.so lib/arm64-v8a/libminecraftpe.so
   ```

4. **Re-sign** the APK and install.

That's it. When Minecraft loads `libminecraftpe.so`, the linker automatically loads `libfusembl2.so` as a dependency, and the `#[ctor]` initializer sets up all hooks.

### How it works

```
Java calls System.loadLibrary("minecraftpe")
  └─ Linker maps libminecraftpe.so into memory
  └─ Linker sees DT_NEEDED: libfusembl2.so
      └─ Loads & maps libfusembl2.so
      └─ Runs #[ctor] → spawns background thread
          └─ Pattern-scans libminecraftpe.so (already mapped)
          └─ Installs PLT hooks for AAsset* functions
  └─ Runs libminecraftpe.so constructors
  └─ Returns to Java
```

### Why patchelf is better than smali

| | Patchelf | Smali |
|---|---|---|
| Dex/Java modification | **None** | Yes |
| Risk of popup/lifecycle breakage | **Zero** | Possible if placed wrong |
| Load ordering | **Automatic** (linker handles it) | Manual |
| Hooks installed before MC code runs | **Yes** | Depends on placement |

## Method 2: Smali (Alternative)

If you can't use patchelf, you can inject a `System.loadLibrary` call into the Minecraft APK's smali code instead.

### Where to inject

Add the `loadLibrary` call inside the **static initializer** (`<clinit>`) of `com/mojang/minecraftpe/MainActivity.smali`, **after** the `minecraftpe` library is loaded.

> [!IMPORTANT]
> MBL2 hooks into `libminecraftpe.so` at load time. It **must** be loaded **after** `minecraftpe` or the hooks will silently fail.

### Exact smali to add

Insert these two lines at the end of the existing `<clinit>` method, right after the `minecraftpe` loadLibrary call and before `return-void`:

```smali
    const-string v0, "fusembl2"

    invoke-static {v0}, Ljava/lang/System;->loadLibrary(Ljava/lang/String;)V
```

### Full `<clinit>` example (Minecraft 1.21.x)

```smali
.method public static constructor <clinit>()V
    .registers 2

    const-string v0, "MCPE"

    const-string v1, "c++_shared"
    invoke-static {v1}, Ljava/lang/System;->loadLibrary(Ljava/lang/String;)V

    :try_start_7
    const-string v1, "maesdk"
    invoke-static {v1}, Ljava/lang/System;->loadLibrary(Ljava/lang/String;)V
    :try_end_c
    .catch Ljava/lang/UnsatisfiedLinkError; {:try_start_7 .. :try_end_c} :catch_d
    goto :goto_12

    :catch_d
    const-string v1, "maesdk library not found. This is expected if we\'re not in Edu mode"
    invoke-static {v0, v1}, Landroid/util/Log;->d(Ljava/lang/String;Ljava/lang/String;)I

    :goto_12
    const-string v1, "HttpClient.Android"
    invoke-static {v1}, Ljava/lang/System;->loadLibrary(Ljava/lang/String;)V

    :try_start_17
    const-string v1, "PlayFabMultiplayer"
    invoke-static {v1}, Ljava/lang/System;->loadLibrary(Ljava/lang/String;)V
    :try_end_1c
    .catch Ljava/lang/UnsatisfiedLinkError; {:try_start_17 .. :try_end_1c} :catch_1d
    goto :goto_22

    :catch_1d
    const-string v1, "playfabmultiplayer library not found."
    invoke-static {v0, v1}, Landroid/util/Log;->d(Ljava/lang/String;Ljava/lang/String;)I

    :goto_22
    const-string v0, "fmod"
    invoke-static {v0}, Ljava/lang/System;->loadLibrary(Ljava/lang/String;)V

    const-string v0, "minecraftpe"
    invoke-static {v0}, Ljava/lang/System;->loadLibrary(Ljava/lang/String;)V

    # ── MBL2 shader loader (must be AFTER minecraftpe) ──
    const-string v0, "fusembl2"
    invoke-static {v0}, Ljava/lang/System;->loadLibrary(Ljava/lang/String;)V

    return-void
.end method
```

### Common mistakes to avoid

| ❌ Don't | Why |
|---|---|
| Load in `onCreate()` | Blocks the Activity lifecycle — can break popups, Xbox sign-in, and cause ANR |
| Load before `minecraftpe` | MBL2 hooks into `libminecraftpe.so` — it must already be in memory |
| Add UI/View/Dialog code in smali | MBL2 is headless — it needs zero Java-side UI |
| Override `onResume`/`onPause`/`onStop` | Breaks Android lifecycle flow — causes popup and toast failures |
| Load the library more than once | `<clinit>` runs exactly once per classloader — no duplicates needed |

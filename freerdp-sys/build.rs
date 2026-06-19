//! Build the vendored FreeRDP 3 from source (with the MS-RDPEGFX H.264 path
//! turned on) and generate FFI bindings against the freshly installed headers.
//!
//! This script only ever runs when `freerdp-sys` is compiled, which only
//! happens when pier-core's `rdp-freerdp` feature pulls it in — so a default
//! Pier-X build never touches cmake, bindgen, or the vendored C tree.

use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=wrapper.h");
    println!("cargo:rerun-if-changed=build.rs");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let vendor = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("vendor/FreeRDP");
    assert!(
        vendor.join("CMakeLists.txt").exists(),
        "vendored FreeRDP missing at {} — run `git submodule update --init --recursive`",
        vendor.display()
    );

    // ── 1. Configure + build + install FreeRDP into OUT_DIR ──────────────
    let mut cfg = cmake::Config::new(&vendor);
    cfg.profile("Release")
        .define("CMAKE_INSTALL_LIBDIR", "lib")
        .define("BUILD_SHARED_LIBS", "ON")
        .define("BUILD_TESTING", "OFF")
        .define("WITH_SAMPLE", "OFF")
        .define("WITH_SERVER", "OFF")
        .define("WITH_SHADOW", "OFF")
        .define("WITH_PROXY", "OFF")
        // Build only the client *common* library (libfreerdp-client3); skip the
        // SDL/X11/Mac/Windows GUI client binaries — we embed the lib directly.
        .define("WITH_CLIENT_COMMON", "ON")
        .define("WITH_CLIENT", "OFF")
        .define("WITH_CLIENT_SDL", "OFF")
        .define("WITH_MANPAGES", "OFF")
        .define("WITH_WINPR_TOOLS", "OFF")
        // Drop redirection channels we don't use that drag in heavy native deps
        // (USB → libusb, camera → ffmpeg/v4l). We only need drdynvc + rdpgfx
        // (graphics), cliprdr, and disp, which stay enabled by default.
        .define("CHANNEL_URBDRC", "OFF")
        .define("CHANNEL_RDPECAM", "OFF")
        // We never redirect audio, so disable every optional sound codec.
        // FFmpeg pulls Opus/etc. onto the host as transitive deps, which
        // FreeRDP would otherwise auto-enable and then fail to find headers for.
        .define("WITH_OPUS", "OFF")
        .define("WITH_LAME", "OFF")
        .define("WITH_FAAD2", "OFF")
        .define("WITH_FAAC", "OFF")
        .define("WITH_GSM", "OFF")
        .define("WITH_SOXR", "OFF")
        // No smartcard / printer / FUSE clipboard redirection.
        .define("WITH_PCSC", "OFF")
        .define("WITH_CUPS", "OFF")
        .define("WITH_FUSE", "OFF")
        // Some bundled third-party CMakeLists still declare a pre-3.5 minimum,
        // which CMake 4.x errors on without a compatibility floor.
        .define("CMAKE_POLICY_VERSION_MINIMUM", "3.5");

    // ── codec backend: OS-native H.264 per target ──────────────────────
    // The MS-RDPEGFX AVC420/AVC444 path needs a decoder backend; FreeRDP then
    // auto-enables WITH_GFX_H264. Each target uses its OS-native decoder, and a
    // missing decoder is a hard cmake error (loud, not a silent non-H.264 build).
    match target_os.as_str() {
        "windows" => {
            // H.264 via Media Foundation (built into Windows; no extra lib).
            cfg.define("WITH_MEDIA_FOUNDATION", "ON")
                .define("WITH_FFMPEG", "OFF");
        }
        "macos" => {
            // H.264 via the FFmpeg backend with VideoToolbox hardware decode
            // (FreeRDP 3.26+ routes VideoToolbox through libavcodec, so it is
            // not a standalone subsystem — FFmpeg must be on). Audio stays off.
            cfg.define("WITH_FFMPEG", "ON")
                .define("WITH_VIDEO_FFMPEG", "ON")
                .define("WITH_VIDEOTOOLBOX", "ON")
                .define("WITH_DSP_FFMPEG", "OFF")
                .define("WITH_SWSCALE", "ON")
                .define("WITH_MACAUDIO", "OFF");
            // Point cmake at Homebrew so it finds OpenSSL + FFmpeg.
            let prefix =
                env::var("PIERX_FREERDP_PREFIX").unwrap_or_else(|_| "/opt/homebrew".into());
            cfg.define("CMAKE_PREFIX_PATH", &prefix)
                .define("OPENSSL_ROOT_DIR", format!("{prefix}/opt/openssl@3"));
        }
        _ => {
            // Linux / other: FFmpeg (can route to VA-API for HW decode).
            cfg.define("WITH_FFMPEG", "ON")
                .define("WITH_VIDEO_FFMPEG", "ON");
        }
    }

    let dst = cfg.build();

    // ── 2. Link the freshly built libraries ─────────────────────────────
    let lib_dir = dst.join("lib");
    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    // Embed an rpath so the dev/test binary resolves the shared libs without a
    // separate bundling step. Release packaging relocates + bundles the libs
    // per target (Windows DLLs beside the exe, macOS @rpath into the .app,
    // Linux into the bundle libdir) — that is handled by the Tauri bundler.
    if target_os != "windows" {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib_dir.display());
    }
    for lib in ["freerdp-client3", "freerdp3", "winpr3"] {
        println!("cargo:rustc-link-lib=dylib={lib}");
    }
    // Expose the lib dir so downstream (src-tauri bundling) can find the libs.
    println!("cargo:lib_dir={}", lib_dir.display());

    // ── 3. Generate bindings against the installed headers ──────────────
    let inc_freerdp = dst.join("include/freerdp3");
    let inc_winpr = dst.join("include/winpr3");
    let bindings = bindgen::Builder::default()
        .header("wrapper.h")
        .clang_arg(format!("-I{}", inc_freerdp.display()))
        .clang_arg(format!("-I{}", inc_winpr.display()))
        .clang_arg(format!("-I{}", dst.join("include").display()))
        .allowlist_function("freerdp_.*")
        .allowlist_function("gdi_.*")
        .allowlist_function("PubSub_.*")
        .allowlist_function("WaitForMultipleObjects")
        .allowlist_type("freerdp")
        .allowlist_type("rdpContext")
        .allowlist_type("rdpSettings")
        .allowlist_type("rdpInput")
        .allowlist_type("rdpGdi")
        .allowlist_type("rdpUpdate")
        .allowlist_type("RdpgfxClientContext")
        .allowlist_type("ChannelConnectedEventArgs")
        .allowlist_type("ChannelDisconnectedEventArgs")
        .allowlist_type("wPubSub")
        .allowlist_type("PierxConst")
        .allowlist_type("FreeRDP_Settings_Keys_.*")
        .allowlist_var("FreeRDP_.*")
        .allowlist_var("PTR_FLAGS_.*")
        .allowlist_var("PTR_XFLAGS_.*")
        .allowlist_var("KBD_FLAGS_.*")
        .allowlist_var("WheelRotationMask")
        .allowlist_var("PIERX_.*")
        // Keep the surface small + stable across point releases.
        .derive_default(false)
        .layout_tests(false)
        .generate_comments(false)
        .generate()
        .expect("bindgen failed to generate FreeRDP bindings");

    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out.join("bindings.rs"))
        .expect("failed to write bindings.rs");
}

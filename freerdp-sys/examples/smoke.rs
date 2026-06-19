//! Link / load smoke test for the vendored libfreerdp3.
//!
//! Proves the source-built dylibs link and resolve at runtime (via the rpath
//! build.rs embeds) and that a FreeRDP instance can be created and freed.
//!
//!   cargo run -p freerdp-sys --example smoke

fn main() {
    unsafe {
        let instance = freerdp_sys::freerdp_new();
        assert!(!instance.is_null(), "freerdp_new() returned null");
        freerdp_sys::freerdp_free(instance);
    }
    println!("freerdp-sys smoke: OK — libfreerdp3 linked + loaded, instance created & freed");
}

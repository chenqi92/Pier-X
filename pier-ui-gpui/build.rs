fn main() {
    println!("cargo:rerun-if-changed=app.rc");
    println!("cargo:rerun-if-changed=assets/app-icons/icon.ico");

    #[cfg(target_os = "windows")]
    {
        // GPUI loads the main window icon from Win32 resource id 1.
        embed_resource::compile("app.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("failed to compile Windows app icon resource");
    }
}

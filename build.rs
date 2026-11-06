fn main() {
    // Embed the app icon in rustdu.exe so Explorer, the taskbar and the Start Menu show it.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/disk-explorer.ico");
        if let Err(e) = res.compile() {
            // Cross-checking from macOS or Linux has no resource compiler; a Windows build must have one.
            if cfg!(windows) {
                panic!("embedding the Windows icon: {e}");
            }
            println!(
                "cargo:warning=Windows icon not embedded (no resource compiler on this host): {e}"
            );
        }
    }
}

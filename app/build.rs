// Windows: embed the app icon in the .exe, so Explorer and shortcuts show it. (The window
// and taskbar icon is set at run time in main.rs, on every platform.)
fn main() {
    println!("cargo:rerun-if-changed=assets/icon/opensights.ico");
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon/opensights.ico");
        res.compile().expect("embedding the Windows icon");
    }
}

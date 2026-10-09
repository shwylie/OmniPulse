// Windows shows this program as "OmniPulse Offset" (Wave Link and the Sound Mixer use the file's
// description), with OmniPulse's icon
fn main() {
    #[cfg(windows)]
    {
        let mut res = tauri_winres::WindowsResource::new();
        res.set_icon("../../src-tauri/icons/icon.ico");
        res.set("FileDescription", "OmniPulse Offset");
        res.set("ProductName", "OmniPulse Offset");
        res.set("InternalName", "omnipulse-offset");
        res.set("OriginalFilename", "omnipulse-offset.exe");
        if let Err(e) = res.compile() {
            println!("cargo:warning=couldn't add the program's name and icon: {e}");
        }
    }
}

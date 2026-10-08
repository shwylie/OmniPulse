// Windows shows this program as "OmniPulse Input" (Wave Link and the Sound Mixer use the file's
// description), with OmniPulse's icon
fn main() {
    #[cfg(windows)]
    {
        let mut res = tauri_winres::WindowsResource::new();
        res.set_icon("../../src-tauri/icons/icon.ico");
        res.set("FileDescription", "OmniPulse Input");
        res.set("ProductName", "OmniPulse Input");
        res.set("InternalName", "omnipulse-input");
        res.set("OriginalFilename", "omnipulse-input.exe");
        if let Err(e) = res.compile() {
            println!("cargo:warning=couldn't add the program's name and icon: {e}");
        }
    }
}

// Windows shows this program as "Offset OmniPulse" (Wave Link and the Sound Mixer use the file's
// description), with OmniPulse's icon
fn main() {
    #[cfg(windows)]
    {
        let mut res = tauri_winres::WindowsResource::new();
        res.set_icon("../../src-tauri/icons/icon.ico");
        res.set("FileDescription", "Offset OmniPulse");
        res.set("ProductName", "Offset OmniPulse");
        res.set("InternalName", "offset-omnipulse");
        res.set("OriginalFilename", "offset-omnipulse.exe");
        if let Err(e) = res.compile() {
            println!("cargo:warning=couldn't add the program's name and icon: {e}");
        }
    }
}

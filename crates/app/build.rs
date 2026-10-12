fn main() {
    slint_build::compile("ui/app.slint").expect("the UI compiles");
    // Windows: the app's icon and name in the exe itself (Task Manager, Windows' media controls, Explorer)
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let v = env!("CARGO_PKG_VERSION");
        let n: Vec<u16> = v.split('.').map(|x| x.parse().unwrap_or(0)).chain(std::iter::repeat(0)).take(4).collect();
        let icon = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/icons/icon.ico").canonicalize().expect("the icon");
        let icon = icon.to_string_lossy().trim_start_matches(r"\\?\").replace('\\', "/");
        let rc = format!(
            "1 ICON \"{icon}\"\n1 VERSIONINFO\nFILEVERSION {a},{b},{c},{d}\nPRODUCTVERSION {a},{b},{c},{d}\nFILEOS 0x40004\nFILETYPE 0x1\n{{\n BLOCK \"StringFileInfo\"\n {{\n  BLOCK \"040904b0\"\n  {{\n   VALUE \"CompanyName\", \"OmniPulse\"\n   VALUE \"FileDescription\", \"OmniPulse\"\n   VALUE \"ProductName\", \"OmniPulse\"\n   VALUE \"OriginalFilename\", \"OmniPulse.exe\"\n   VALUE \"FileVersion\", \"{v}\"\n   VALUE \"ProductVersion\", \"{v}\"\n  }}\n }}\n BLOCK \"VarFileInfo\"\n {{\n  VALUE \"Translation\", 0x409, 1200\n }}\n}}\n",
            a = n[0], b = n[1], c = n[2], d = n[3]
        );
        let out = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("omnipulse.rc");
        std::fs::write(&out, rc).expect("the resource script");
        embed_resource::compile(&out, embed_resource::NONE).manifest_optional().expect("the icon and name embedded");
    }
}

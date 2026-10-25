fn main() {
    // Windows: its icon, its name and its manifest in the exe (no version of OmniPulse's in it: it's the same
    // file for every one)
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let at = |p: &str| {
            let f = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(p).canonicalize().expect(p);
            f.to_string_lossy().trim_start_matches(r"\\?\").replace('\\', "/")
        };
        let (icon, manifest) = (at("../../src-tauri/icons/icon.ico"), at("setup.manifest"));
        println!("cargo:rerun-if-changed=setup.manifest");
        let rc = format!(
            "1 ICON \"{icon}\"\n1 24 \"{manifest}\"\n1 VERSIONINFO\nFILEVERSION 1,0,0,0\nPRODUCTVERSION 1,0,0,0\nFILEOS 0x40004\nFILETYPE 0x1\n{{\n BLOCK \"StringFileInfo\"\n {{\n  BLOCK \"040904b0\"\n  {{\n   VALUE \"CompanyName\", \"OmniPulse\"\n   VALUE \"FileDescription\", \"OmniPulse Setup\"\n   VALUE \"ProductName\", \"OmniPulse\"\n   VALUE \"OriginalFilename\", \"OmniPulse-Setup.exe\"\n   VALUE \"FileVersion\", \"1.0.0\"\n   VALUE \"ProductVersion\", \"1.0.0\"\n  }}\n }}\n BLOCK \"VarFileInfo\"\n {{\n  VALUE \"Translation\", 0x409, 1200\n }}\n}}\n"
        );
        let out = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("setup.rc");
        std::fs::write(&out, rc).expect("the resource script");
        embed_resource::compile(&out, embed_resource::NONE).manifest_optional().expect("the icon, name and manifest embedded");
    }
}

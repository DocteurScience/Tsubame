use std::path::PathBuf;
use std::process::Command;

fn main() {
    glib_build_tools::compile_resources(
        &["data"],
        "data/resources.gresource.xml",
        "tsubame.gresource",
    );

    // Schéma GSettings compilé pour `cargo run`. Une fois installé (Meson,
    // Flatpak), l'application utilise le schéma du système.
    let schema_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("schemas");
    std::fs::create_dir_all(&schema_dir).unwrap();
    std::fs::copy(
        "data/io.github.DocteurScience.Tsubame.gschema.xml",
        schema_dir.join("io.github.DocteurScience.Tsubame.gschema.xml"),
    )
    .unwrap();
    let status = Command::new("glib-compile-schemas")
        .arg("--strict")
        .arg(&schema_dir)
        .status()
        .expect("glib-compile-schemas introuvable");
    assert!(status.success(), "schéma GSettings invalide");
    println!("cargo:rerun-if-changed=data/io.github.DocteurScience.Tsubame.gschema.xml");
    println!(
        "cargo:rustc-env=TSUBAME_DEV_SCHEMA_DIR={}",
        schema_dir.display()
    );
}

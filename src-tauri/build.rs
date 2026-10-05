fn main() {
    println!("cargo:rerun-if-env-changed=SPLIT_EDITION");
    println!("cargo:rustc-check-cfg=cfg(split_edition, values(\"borderless\", \"fullscreen\"))");

    let edition = std::env::var("SPLIT_EDITION").unwrap_or_else(|_| "borderless".to_string());

    if !matches!(edition.as_str(), "borderless" | "fullscreen") {
        panic!("Unsupported SPLIT_EDITION={edition:?}; expected \"borderless\" or \"fullscreen\"");
    }

    println!("cargo:rustc-cfg=split_edition=\"{edition}\"");
    println!("cargo:rustc-env=SPLIT_EDITION={edition}");

    tauri_build::build()
}

fn main() {
    println!("cargo:rerun-if-env-changed=SPLIT_EDITION");
    println!(
        "cargo:rustc-check-cfg=cfg(split_edition, values(\"borderless\", \"panorama\"))"
    );

    let edition = std::env::var("SPLIT_EDITION")
        .unwrap_or_else(|_| "borderless".to_string());

    if !matches!(edition.as_str(), "borderless" | "panorama") {
        panic!(
            "Unsupported SPLIT_EDITION={edition:?}; expected \"borderless\" or \"panorama\""
        );
    }

    println!("cargo:rustc-cfg=split_edition=\"{edition}\"");
    println!("cargo:rustc-env=SPLIT_EDITION={edition}");

    tauri_build::build()
}

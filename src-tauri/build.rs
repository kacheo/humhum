fn main() {
    tauri_build::build();
    // Embedded only by the opt-in test benchmark, never queried from its runner shell.
    let rustc = std::process::Command::new(std::env::var("RUSTC").unwrap())
        .arg("-Vv").output().expect("rustc provenance");
    assert!(rustc.status.success());
    println!("cargo:rustc-env=HUMHUM_BUILD_RUSTC={}", String::from_utf8(rustc.stdout).unwrap().replace('\n', "; "));
    for key in ["TARGET", "PROFILE", "CARGO_CFG_TARGET_FEATURE"] {
        println!("cargo:rustc-env=HUMHUM_BUILD_{key}={}", std::env::var(key).unwrap_or_default());
    }
    let mut flags: Vec<_> = std::env::vars().filter(|(k, _)| {
        k.starts_with("CARGO_FEATURE_") || k.starts_with("GGML_") || k.starts_with("WHISPER_")
            || k.starts_with("CMAKE_") || k == "CARGO_ENCODED_RUSTFLAGS" || k == "RUSTFLAGS"
    }).collect();
    flags.sort();
    println!("cargo:rustc-env=HUMHUM_BUILD_FLAGS={flags:?}");
    for (key, _) in flags { println!("cargo:rerun-if-env-changed={key}"); }
}

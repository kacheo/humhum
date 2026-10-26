fn main() {
    tauri_build::build();
    let revision = std::process::Command::new("git")
        .args(["describe", "--always", "--dirty"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=HUMHUM_BUILD_REVISION={revision}");
    // Include the dependency identity without exposing environment variables.
    let lock_hash = std::process::Command::new("shasum")
        .args(["-a", "256", "Cargo.lock"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| {
            String::from_utf8_lossy(&out.stdout)
                .split_whitespace()
                .next()
                .unwrap_or("unknown")
                .to_owned()
        })
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=HUMHUM_BUILD_LOCK_SHA256={lock_hash}");
    println!("cargo:rerun-if-changed=Cargo.lock");
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/refs");
    // Build metadata shared by opt-in diagnostics and the real-model benchmark.
    let rustc = std::process::Command::new(std::env::var("RUSTC").unwrap())
        .arg("-Vv")
        .output()
        .expect("rustc provenance");
    assert!(rustc.status.success());
    println!(
        "cargo:rustc-env=HUMHUM_BUILD_RUSTC={}",
        String::from_utf8(rustc.stdout).unwrap().replace('\n', "; ")
    );
    for key in [
        "TARGET",
        "PROFILE",
        "CARGO_CFG_TARGET_FEATURE",
        "OPT_LEVEL",
        "DEBUG",
    ] {
        println!(
            "cargo:rustc-env=HUMHUM_BUILD_{key}={}",
            std::env::var(key).unwrap_or_default()
        );
    }
    let mut flags: Vec<_> = std::env::vars()
        .filter(|(k, _)| {
            k.starts_with("CARGO_FEATURE_")
                || k.starts_with("CARGO_PROFILE_")
                || k.starts_with("GGML_")
                || k.starts_with("WHISPER_")
                || k.starts_with("CMAKE_")
                || k == "CARGO_ENCODED_RUSTFLAGS"
                || k == "RUSTFLAGS"
        })
        .collect();
    flags.sort();
    println!("cargo:rustc-env=HUMHUM_BUILD_FLAGS={flags:?}");
    for (key, _) in flags {
        println!("cargo:rerun-if-env-changed={key}");
    }
}

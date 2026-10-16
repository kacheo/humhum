//! Opt-in real-model regression measurements; excluded from normal test runs.
use super::*;
use std::{fs, time::Instant};

#[test]
#[ignore = "requires verified real models and the fixed corpus; run through scripts/transcription-benchmark.py"]
fn real_model() {
    assert!(
        !cfg!(debug_assertions),
        "benchmark requires a release test binary"
    );
    let model = std::env::var("HUMHUM_BENCH_MODEL").expect("model path");
    let pcm = fs::read(std::env::var("HUMHUM_BENCH_PCM").expect("PCM path")).unwrap();
    assert_eq!(pcm.len() % 2, 0, "truncated PCM16");
    let samples: Vec<f32> = pcm
        .chunks_exact(2)
        .map(|s| i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0)
        .collect();
    let load_start = Instant::now();
    let ctx = load_ctx_from(Path::new(&model)).expect("load model");
    let load_ms = load_start.elapsed().as_secs_f64() * 1000.0;
    let mut runs = Vec::new();
    for _ in 0..6 {
        let start = Instant::now();
        let text = match transcribe_chunks(&samples, 16_000, |p| infer_chunk(&ctx, p)) {
            Ok(text) => text,
            Err(TranscriptionError::EmptyResult) => String::new(),
            Err(error) => panic!("inference failed: {error}"),
        };
        runs.push(serde_json::json!({"text": text, "ms": start.elapsed().as_secs_f64() * 1000.0}));
    }
    let mut usage: nix::libc::rusage = unsafe { std::mem::zeroed() };
    assert_eq!(
        unsafe { nix::libc::getrusage(nix::libc::RUSAGE_SELF, &mut usage) },
        0
    );
    #[cfg(target_os = "macos")]
    let peak_rss_bytes = usage.ru_maxrss;
    #[cfg(not(target_os = "macos"))]
    let peak_rss_bytes = usage.ru_maxrss * 1024;
    let info = whisper_rs::print_system_info().replace(" ", "");
    let blas = info.contains("BLAS=1") || info.contains("ACCELERATE=1");
    let result = serde_json::json!({"rustc": env!("HUMHUM_BUILD_RUSTC"), "build": {"target": env!("HUMHUM_BUILD_TARGET"), "profile": env!("HUMHUM_BUILD_PROFILE"), "target_features": env!("HUMHUM_BUILD_CARGO_CFG_TARGET_FEATURE"), "opt_level": env!("HUMHUM_BUILD_OPT_LEVEL"), "debug": env!("HUMHUM_BUILD_DEBUG"), "flags": env!("HUMHUM_BUILD_FLAGS")}, "backend": {"gpu": WhisperContextParameters::default().use_gpu, "blas": blas}, "system_info": whisper_rs::print_system_info(), "load_ms": load_ms, "peak_rss_bytes": peak_rss_bytes, "runs": runs});
    fs::write(
        std::env::var("HUMHUM_BENCH_OUTPUT").expect("output path"),
        serde_json::to_vec_pretty(&result).unwrap(),
    )
    .unwrap();
}

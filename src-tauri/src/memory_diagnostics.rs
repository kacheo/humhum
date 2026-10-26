//! Opt-in, bounded, metadata-only memory diagnostics. No audio/text/keys.
use std::io::{self, Write};
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
const OUTPUT_LIMIT: usize = 5_000_000;

fn write_bounded(
    out: &mut impl Write,
    written: &mut usize,
    limit: usize,
    record: &[u8],
) -> io::Result<bool> {
    if record.len() > limit.saturating_sub(*written) {
        return Ok(false);
    }
    out.write_all(record)?;
    *written += record.len();
    Ok(true)
}

/// Current, not high-water, RSS and physical footprint from the kernel.
#[cfg(target_os = "macos")]
pub(crate) fn process_memory() -> Option<(u64, u64)> {
    use nix::libc;
    let mut usage: libc::rusage_info_v0 = unsafe { std::mem::zeroed() };
    // SAFETY: the v0 flavor writes exactly rusage_info_v0 into this buffer.
    let status = unsafe {
        libc::proc_pid_rusage(
            std::process::id() as i32,
            libc::RUSAGE_INFO_V0,
            (&mut usage as *mut libc::rusage_info_v0).cast(),
        )
    };
    (status == 0).then_some((usage.ri_resident_size, usage.ri_phys_footprint))
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn process_memory() -> Option<(u64, u64)> {
    None
}

/// Set HUMHUM_MEMORY_DIAGNOSTICS to a NEW output file path. create_new protects
/// existing files; reaching 5 MB stops diagnostics instead of growing logs.
pub(crate) fn start(phase: Arc<AtomicU8>) {
    let Some(path) = std::env::var_os("HUMHUM_MEMORY_DIAGNOSTICS") else {
        return;
    };
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path);
    let mut file = match file {
        Ok(file) => file,
        Err(error) => {
            log::warn!("memory diagnostics could not open output: {error}");
            return;
        }
    };
    let result = std::thread::Builder::new().name("humhum-memory".into()).spawn(move || {
        let mut written = 0;
        let metadata = serde_json::json!({
            "kind": "build", "pid": std::process::id(),
            "revision": env!("HUMHUM_BUILD_REVISION"),
            "lockfile_sha256": env!("HUMHUM_BUILD_LOCK_SHA256"),
            "target": env!("HUMHUM_BUILD_TARGET"), "profile": env!("HUMHUM_BUILD_PROFILE"),
            "rustc": env!("HUMHUM_BUILD_RUSTC"),
            "target_features": env!("HUMHUM_BUILD_CARGO_CFG_TARGET_FEATURE"),
            "opt_level": env!("HUMHUM_BUILD_OPT_LEVEL"), "debug": env!("HUMHUM_BUILD_DEBUG"),
            "gpu_requested": whisper_rs::WhisperContextParameters::default().use_gpu,
            "system_info": whisper_rs::print_system_info(),
        });
        let mut emit = |value: serde_json::Value| {
            let record = format!("{value}\n");
            write_bounded(&mut file, &mut written, OUTPUT_LIMIT, record.as_bytes()).unwrap_or(false)
        };
        if !emit(metadata) { return; }
        loop {
            let memory = process_memory();
            let (active, model, samples) = crate::transcription::memory_snapshot();
            let phase = crate::state::RecordingPhase::try_from(phase.load(Ordering::Relaxed));
            if !emit(serde_json::json!({
                "kind": "memory", "unix_secs": SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
                "rss_bytes": memory.map(|m| m.0), "footprint_bytes": memory.map(|m| m.1),
                "phase": format!("{:?}", phase.unwrap_or(crate::state::RecordingPhase::Idle)),
                "active_workers": active, "cached_model": model, "cached_contexts": usize::from(model != "none"),
                "inference_samples": samples, "capture_samples": crate::audio::captured_sample_count(),
            })) { break; }
            std::thread::sleep(Duration::from_secs(30));
        }
    });
    if let Err(error) = result {
        log::warn!("memory diagnostics could not start: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn output_stops_at_limit_without_partial_records() {
        let mut output = Vec::new();
        let mut bytes = 0;
        assert!(write_bounded(&mut output, &mut bytes, 8, b"1234\n").unwrap());
        assert!(!write_bounded(&mut output, &mut bytes, 8, b"5678\n").unwrap());
        assert_eq!(output, b"1234\n");
        assert_eq!(bytes, 5);
    }
    #[test]
    #[cfg(target_os = "macos")]
    fn current_process_has_resident_memory_and_footprint() {
        let (rss, footprint) = process_memory().unwrap();
        assert!(rss > 0);
        assert!(footprint > 0);
    }
}

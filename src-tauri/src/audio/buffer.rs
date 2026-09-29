use cpal::Sample;

/// In-memory PCM capture buffer.
///
/// **Intentionally NOT `Serialize`/`Deserialize`.** Per LLD §868, raw audio is
/// never written to disk or sent over IPC — it lives only in memory and is
/// dropped immediately after transcription or on cancel. Withholding serde
/// keeps that privacy guarantee from being accidentally broken.
#[derive(Debug, Clone)]
pub struct AudioBuffer {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub duration_ms: u64,
}

/// Whisper's required sample rate.
pub const TARGET_SAMPLE_RATE: u32 = 16_000;

/// Convert an interleaved CPAL sample buffer of any supported type (`f32`,
/// `i16`, `u16`, …) to mono `f32` by averaging channels. Used by the capture
/// callback so the conversion path is unit-testable without a live stream.
pub fn to_mono_f32<T>(interleaved: &[T], channels: u16) -> Vec<f32>
where
    T: cpal::Sample,
    f32: cpal::FromSample<T>,
{
    let as_f32: Vec<f32> = interleaved.iter().map(|s| f32::from_sample(*s)).collect();
    downmix_to_mono(&as_f32, channels)
}

/// Average interleaved multi-channel samples down to mono. `channels == 1` is a
/// passthrough. A trailing partial frame (shouldn't happen from CPAL) is ignored.
pub fn downmix_to_mono(interleaved: &[f32], channels: u16) -> Vec<f32> {
    let channels = channels.max(1) as usize;
    if channels == 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// Duration in whole milliseconds for a mono sample count at 16 kHz.
pub fn duration_ms(sample_count: usize) -> u64 {
    sample_count as u64 * 1000 / TARGET_SAMPLE_RATE as u64
}

/// Resample a mono `f32` signal to 16 kHz (LLD §189). Identity when already at
/// 16 kHz. On resampler-construction failure, falls back to the raw samples
/// rather than failing the recording.
pub fn resample_to_16k(samples: Vec<f32>, from_rate: u32) -> Vec<f32> {
    use rubato::{FftFixedIn, Resampler};

    if from_rate == TARGET_SAMPLE_RATE || samples.is_empty() {
        return samples;
    }

    const CHUNK: usize = 1024;
    let mut resampler = match FftFixedIn::<f32>::new(
        from_rate as usize,
        TARGET_SAMPLE_RATE as usize,
        CHUNK,
        2,
        1,
    ) {
        Ok(r) => r,
        Err(_) => return samples,
    };

    let mut out: Vec<f32> = Vec::with_capacity(
        samples.len() * TARGET_SAMPLE_RATE as usize / from_rate.max(1) as usize + CHUNK,
    );
    let mut pos = 0;
    while pos < samples.len() {
        // FftFixedIn requires exactly `input_frames_next()` samples per call;
        // pad the final chunk with silence.
        let need = resampler.input_frames_next();
        let mut input = vec![0.0f32; need];
        let end = (pos + need).min(samples.len());
        input[..end - pos].copy_from_slice(&samples[pos..end]);
        pos += need;
        if let Ok(mut chunks) = resampler.process(&[input.as_slice()], None) {
            out.append(&mut chunks[0]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Locks the field shape. No serde test — serde is deliberately absent
    /// (see the type doc; LLD §868).
    #[test]
    fn audio_buffer_shape() {
        let buf = AudioBuffer {
            samples: vec![0.0, 0.5, -0.5],
            sample_rate: 16_000,
            duration_ms: 42,
        };
        let cloned = buf.clone();
        assert_eq!(cloned.samples.len(), 3);
        assert_eq!(cloned.sample_rate, 16_000);
        assert_eq!(cloned.duration_ms, 42);
        assert!(format!("{buf:?}").contains("AudioBuffer"));
    }

    #[test]
    fn downmix_stereo_averages_channels() {
        // Two frames: (0.0,1.0) -> 0.5 ; (-1.0,1.0) -> 0.0
        let interleaved = [0.0, 1.0, -1.0, 1.0];
        assert_eq!(downmix_to_mono(&interleaved, 2), vec![0.5, 0.0]);
    }

    #[test]
    fn downmix_mono_passthrough() {
        let mono = [0.1, -0.2, 0.3];
        assert_eq!(downmix_to_mono(&mono, 1), vec![0.1, -0.2, 0.3]);
    }

    #[test]
    fn duration_ms_is_samples_over_16_per_ms() {
        // 16 kHz -> 16 samples per ms.
        assert_eq!(duration_ms(16_000), 1000);
        assert_eq!(duration_ms(8), 0); // sub-millisecond floors
        assert_eq!(duration_ms(0), 0);
    }

    #[test]
    fn to_mono_f32_converts_and_downmixes_i16() {
        // i16 stereo: frame (0, MAX) -> ((0.0 + ~1.0)/2) ~= 0.5
        let data: [i16; 2] = [0, i16::MAX];
        let mono = to_mono_f32(&data, 2);
        assert_eq!(mono.len(), 1);
        assert!((mono[0] - 0.5).abs() < 0.01, "got {}", mono[0]);
    }

    #[test]
    fn resample_at_target_rate_is_identity() {
        let samples = vec![0.1, -0.2, 0.3, -0.4];
        assert_eq!(resample_to_16k(samples.clone(), 16_000), samples);
    }

    #[test]
    fn resample_48k_to_16k_thirds_the_length() {
        // 1 second of 48 kHz -> ~1 second of 16 kHz (~16000 samples).
        let input: Vec<f32> = (0..48_000).map(|i| (i as f32 * 0.01).sin()).collect();
        let out = resample_to_16k(input, 48_000);
        assert!(
            (15_000..=17_000).contains(&out.len()),
            "expected ~16000 samples, got {}",
            out.len()
        );
    }

    #[test]
    fn resample_upsamples_8k_to_16k() {
        // 1 second of 8 kHz -> ~1 second of 16 kHz (~doubles, no panic).
        let input: Vec<f32> = (0..8_000).map(|i| (i as f32 * 0.02).sin()).collect();
        let out = resample_to_16k(input, 8_000);
        assert!(
            (15_000..=17_500).contains(&out.len()),
            "expected ~16000 samples, got {}",
            out.len()
        );
    }

    #[test]
    fn resample_handles_non_integer_ratio_44100() {
        // The common Mac default. 1 second 44.1k -> ~16000.
        let input: Vec<f32> = (0..44_100).map(|i| (i as f32 * 0.01).sin()).collect();
        let out = resample_to_16k(input, 44_100);
        assert!(
            (15_000..=17_000).contains(&out.len()),
            "expected ~16000 samples, got {}",
            out.len()
        );
    }

    #[test]
    fn resample_empty_is_empty() {
        assert!(resample_to_16k(Vec::new(), 48_000).is_empty());
    }

    #[test]
    fn resample_falls_back_to_raw_samples_when_the_resampler_cannot_be_built() {
        // `from_rate: 0` fails `FftFixedIn::new`'s internal rate validation, so
        // the function must fall back to returning the input unchanged rather
        // than panicking.
        let samples = vec![0.1, 0.2, 0.3, 0.4];
        assert_eq!(resample_to_16k(samples.clone(), 0), samples);
    }

    #[test]
    fn to_mono_f32_u16_and_f32_passthrough() {
        // u16 is offset-binary: midpoint u16::MAX/2+1 ~= 0.0.
        let u: [u16; 2] = [u16::MAX / 2 + 1, u16::MAX / 2 + 1];
        let m = to_mono_f32(&u, 2);
        assert_eq!(m.len(), 1);
        assert!(m[0].abs() < 0.01, "u16 midpoint ~= 0, got {}", m[0]);

        // f32 passthrough (mono).
        assert_eq!(to_mono_f32(&[0.25f32, -0.5], 1), vec![0.25, -0.5]);
    }

    #[test]
    fn downmix_channels_zero_treated_as_mono() {
        // Defensive `.max(1)` guard — never divides by zero / panics.
        assert_eq!(downmix_to_mono(&[0.1, 0.2], 0), vec![0.1, 0.2]);
    }

    #[test]
    fn downmix_drops_trailing_partial_frame() {
        // 3 samples with 2 channels: one full frame averaged, the stray sample dropped.
        assert_eq!(downmix_to_mono(&[0.0, 1.0, 0.5], 2), vec![0.5]);
    }
}

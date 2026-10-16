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

/// Resample an in-memory mono clip to 16 kHz. Preserve exact clip duration,
/// trim the FFT startup delay and flush the final partial chunk. A failure must
/// never label unconverted device-rate samples as 16 kHz audio.
pub fn resample_to_16k(samples: Vec<f32>, from_rate: u32) -> Result<Vec<f32>, super::AudioError> {
    use rubato::{Fft, FixedSync, WindowFunction};

    if from_rate == 0 {
        return Err(super::AudioError::StreamError(
            "resampling: invalid input sample rate 0".into(),
        ));
    }
    if from_rate == TARGET_SAMPLE_RATE || samples.is_empty() {
        return Ok(samples);
    }

    // Retain the previous FFT chunk/sub-chunk sizes and anti-aliasing window.
    let mut resampler = Fft::<f32>::new_custom(
        from_rate as usize,
        TARGET_SAMPLE_RATE as usize,
        1024,
        2,
        1,
        WindowFunction::BlackmanHarris2,
        FixedSync::Input,
    )
    .map_err(|error| super::AudioError::StreamError(format!("resampler construction: {error}")))?;
    resample_clip(&mut resampler, samples)
}

fn resample_clip(
    resampler: &mut impl rubato::Resampler<f32>,
    samples: Vec<f32>,
) -> Result<Vec<f32>, super::AudioError> {
    use rubato::audioadapter_buffers::owned::InterleavedOwned;

    let len = samples.len();
    let input = InterleavedOwned::new_from(samples, 1, len)
        .map_err(|error| super::AudioError::StreamError(format!("resampler input: {error}")))?;
    let output = resampler
        .process_all(&input, len, None)
        .map_err(|error| super::AudioError::StreamError(format!("resampling: {error}")))?;
    Ok(output.take_data())
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
        assert_eq!(resample_to_16k(samples.clone(), 16_000).unwrap(), samples);
    }

    #[test]
    fn resample_48k_to_16k_thirds_the_length() {
        // 1 second of 48 kHz -> ~1 second of 16 kHz (~16000 samples).
        let input: Vec<f32> = (0..48_000).map(|i| (i as f32 * 0.01).sin()).collect();
        let out = resample_to_16k(input, 48_000).unwrap();
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
        let out = resample_to_16k(input, 8_000).unwrap();
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
        let out = resample_to_16k(input, 44_100).unwrap();
        assert!(
            (15_000..=17_000).contains(&out.len()),
            "expected ~16000 samples, got {}",
            out.len()
        );
    }

    #[test]
    fn resample_empty_is_empty() {
        assert!(resample_to_16k(Vec::new(), 48_000).unwrap().is_empty());
    }

    #[test]
    fn resample_invalid_rate_fails_with_existing_stream_error() {
        for samples in [vec![0.1, 0.2, 0.3, 0.4], Vec::new()] {
            let error = resample_to_16k(samples, 0).unwrap_err();
            assert!(matches!(error, super::super::AudioError::StreamError(_)));
            assert_eq!(error.code(), "stream_error");
        }
    }

    #[test]
    fn resample_processing_error_does_not_return_partial_audio() {
        use rubato::{Fft, FixedSync};
        // A real resampler requiring two channels rejects the mono clip. The
        // same processing path used by capture must propagate that failure.
        let mut resampler = Fft::<f32>::new(48_000, 16_000, 1024, 2, FixedSync::Input).unwrap();
        let error = resample_clip(&mut resampler, vec![0.25; 48_000]).unwrap_err();
        assert!(matches!(error, super::super::AudioError::StreamError(_)));
        assert_eq!(error.code(), "stream_error");
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
    #[test]
    fn resample_preserves_duration_for_short_and_partial_chunks() {
        for rate in [8_000u32, 44_100, 48_000] {
            for len in [1, 17, 1023, 1025, rate as usize + 13] {
                let out = resample_to_16k(vec![0.25; len], rate).unwrap();
                let expected = (len * TARGET_SAMPLE_RATE as usize).div_ceil(rate as usize);
                assert_eq!(out.len(), expected, "{rate} Hz, {len} input frames");
                assert!(out.iter().all(|sample| sample.is_finite()));
            }
        }
    }

    #[test]
    fn resample_retains_the_last_partial_chunk_signal() {
        for rate in [8_000u32, 44_100, 48_000] {
            let mut input = vec![0.0; rate as usize + 17];
            let start = input.len() - (rate / 100) as usize;
            input[start..].fill(0.5);
            let out = resample_to_16k(input, rate).unwrap();
            assert!(
                out[out.len() - 100..].iter().any(|x| x.abs() > 0.1),
                "lost final speech-like signal at {rate} Hz"
            );
        }
    }
    #[test]
    fn resample_preserves_audible_tone_amplitude_and_timing() {
        for rate in [8_000u32, 44_100, 48_000] {
            let tone = |i: usize, rate: u32| {
                0.5 * (std::f32::consts::TAU * 1_000.0 * i as f32 / rate as f32).sin()
            };
            let input = (0..rate as usize).map(|i| tone(i, rate)).collect();
            let out = resample_to_16k(input, rate).unwrap();
            // FFT delay is reported in whole output frames. Some rate pairs
            // retain a fractional-frame phase offset: require <1 frame, rather
            // than assuming an integer-delay filter has zero fractional delay.
            let range = 64..out.len() - 64;
            let omega = std::f64::consts::TAU * 1_000.0 / TARGET_SAMPLE_RATE as f64;
            let sin = range
                .clone()
                .map(|i| out[i] as f64 * (omega * i as f64).sin())
                .sum::<f64>()
                * 2.0
                / range.len() as f64;
            let cos = range
                .clone()
                .map(|i| out[i] as f64 * (omega * i as f64).cos())
                .sum::<f64>()
                * 2.0
                / range.len() as f64;
            let amplitude = sin.hypot(cos);
            let phase = cos.atan2(sin);
            assert!(
                (amplitude - 0.5).abs() < 0.005,
                "amplitude at {rate}: {amplitude}"
            );
            assert!(
                phase.abs() < omega,
                "delay exceeds one frame at {rate}: {phase}"
            );
            let error = range
                .clone()
                .map(|i| (out[i] as f64 - amplitude * (omega * i as f64 + phase).sin()).powi(2))
                .sum::<f64>()
                / range.len() as f64;
            assert!(
                error.sqrt() < 0.005,
                "tone distortion at {rate}: {}",
                error.sqrt()
            );
        }
    }
    #[test]
    fn resample_keeps_an_impulse_at_its_absolute_time() {
        for rate in [8_000u32, 44_100, 48_000] {
            let mut input = vec![0.0; rate as usize];
            input[(rate / 2) as usize] = 1.0;
            let out = resample_to_16k(input, rate).unwrap();
            let peak = out
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| a.abs().total_cmp(&b.abs()))
                .unwrap()
                .0;
            assert!(
                peak.abs_diff(8_000) <= 1,
                "impulse shifted at {rate}: frame {peak}"
            );
        }
    }
}

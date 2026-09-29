pub fn chunk_pcm(
    samples: &[f32],
    chunk_secs: f32,
    overlap_secs: f32,
    sample_rate: u32,
) -> Vec<Vec<f32>> {
    let chunk_len = (chunk_secs * sample_rate as f32) as usize;
    let overlap_len = (overlap_secs * sample_rate as f32) as usize;

    if samples.is_empty() {
        return vec![];
    }

    assert!(
        overlap_len < chunk_len,
        "chunk_pcm: overlap_len ({overlap_len}) must be < chunk_len ({chunk_len})"
    );
    let step = chunk_len - overlap_len; // > 0, guaranteed by the assertion above

    let mut chunks = Vec::new();
    let mut start = 0;
    while start < samples.len() {
        // Skip a non-first chunk that lies entirely within the previous chunk's
        // overlap region (it would contribute no new samples → duplicate audio).
        if start > 0 && samples.len() - start <= overlap_len {
            break;
        }
        let end = (start + chunk_len).min(samples.len());
        chunks.push(samples[start..end].to_vec());
        start += step;
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each sample's value equals its index, so chunk contents can be asserted by range.
    fn samples(n: usize) -> Vec<f32> {
        (0..n).map(|i| i as f32).collect()
    }

    #[test]
    fn empty_input_returns_empty() {
        assert!(chunk_pcm(&[], 30.0, 2.0, 16_000).is_empty());
    }

    #[test]
    fn no_redundant_trailing_chunk() {
        // 90 samples, chunk 30, overlap 10, step 20 → starts 0,20,40,60; NOT 80.
        let chunks = chunk_pcm(&samples(90), 30.0, 10.0, 1);
        assert_eq!(chunks.len(), 4);
        assert_eq!(chunks[3], samples(90)[60..90].to_vec());
        // The dropped chunk would have been samples[80..90].
        assert!(chunks.iter().all(|c| c.first() != Some(&80.0)));
    }

    #[test]
    fn exact_multiple_final_chunk_ends_at_len() {
        // 50 samples, chunk 30, overlap 10, step 20 → [0..30],[20..50]; no [40..50].
        let chunks = chunk_pcm(&samples(50), 30.0, 10.0, 1);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0], samples(50)[0..30].to_vec());
        assert_eq!(chunks[1], samples(50)[20..50].to_vec());
    }

    #[test]
    fn partial_final_chunk() {
        // 65 samples, chunk 30, overlap 10, step 20 → [0..30],[20..50],[40..65].
        let chunks = chunk_pcm(&samples(65), 30.0, 10.0, 1);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[2], samples(65)[40..65].to_vec());
        assert_eq!(chunks[2].len(), 25);
    }

    #[test]
    fn samples_shorter_than_one_chunk() {
        let chunks = chunk_pcm(&samples(20), 30.0, 10.0, 1);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], samples(20));
    }

    #[test]
    fn samples_shorter_than_overlap() {
        // First-chunk guard must not drop the only chunk.
        let chunks = chunk_pcm(&samples(5), 30.0, 10.0, 1);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], samples(5));
    }

    #[test]
    #[should_panic(expected = "overlap_len")]
    fn overlap_ge_chunk_panics() {
        let _ = chunk_pcm(&samples(50), 10.0, 30.0, 1);
    }
}

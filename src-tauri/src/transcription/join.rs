//! Joining per-chunk transcripts produced from overlapping audio windows.

/// Upper bound on words/sec used only to size the search window for the
/// actual overlap between adjacent chunks — not to guess the overlap length
/// directly (that's what the fixed `overlap_secs * wps` heuristic this
/// replaces got wrong: fast/dense speech under-stripped, slow speech or a
/// pause in the overlap region over-stripped). Generously high so real
/// overlaps of any speaking rate fall within it; the exact length is found by
/// text matching, so an oversized bound only costs a few extra comparisons.
const MAX_WORDS_PER_SEC: f32 = 6.0;

/// Lowercase and drop non-alphanumeric characters, so a word match at a chunk
/// boundary survives whisper re-punctuating or re-casing the same speech
/// differently across the two overlapping transcriptions (e.g. "world." in
/// one chunk's tail vs "World" at the head of the next).
fn normalize_word(word: &str) -> String {
    word.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Length of the longest run where a suffix of `prev_tail` equals a prefix of
/// `next_head` (compared via [`normalize_word`]), searched only within
/// `max_words` of each end. Returns 0 if no such run exists — safer than
/// guessing, since finding nothing means we keep every word rather than
/// dropping real content.
fn overlap_len(prev_tail: &[&str], next_head: &[&str], max_words: usize) -> usize {
    let a_search = prev_tail.len().min(max_words);
    let b_search = next_head.len().min(max_words);
    for len in (1..=a_search.min(b_search)).rev() {
        let a = &prev_tail[prev_tail.len() - len..];
        let b = &next_head[..len];
        if a.iter()
            .zip(b)
            .all(|(x, y)| normalize_word(x) == normalize_word(y))
        {
            return len;
        }
    }
    0
}

/// Concatenate per-chunk transcripts, dropping from each chunk (after the
/// first) whatever leading run of words is an exact (normalized) match for
/// the trailing run already emitted — i.e. the words whisper transcribed
/// twice because they fall in the overlap region shared with the previous
/// chunk. Word boundaries are plain whitespace splits.
///
/// Known residual limitation (unchanged from the fixed-count heuristic this
/// replaces): if whisper transcribes a word that spans the exact chunk
/// boundary inconsistently between the two chunks (e.g. cut mid-word), no
/// text-matching strategy can detect that as an overlap — there is no shared
/// token to find.
pub fn join_transcripts(parts: &[String], overlap_secs: f32) -> String {
    let max_words = ((overlap_secs * MAX_WORDS_PER_SEC).ceil() as usize).max(1);
    let mut words: Vec<&str> = Vec::new();
    for part in parts {
        let chunk_words: Vec<&str> = part.split_whitespace().collect();
        if words.is_empty() {
            words.extend(chunk_words);
            continue;
        }
        let overlap = overlap_len(&words, &chunk_words, max_words);
        words.extend(&chunk_words[overlap..]);
    }
    words.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_parts_join_to_empty() {
        assert_eq!(join_transcripts(&[], 2.0), "");
    }

    #[test]
    fn single_part_is_returned_unchanged() {
        let parts = vec!["hello there world".to_string()];
        assert_eq!(join_transcripts(&parts, 2.0), "hello there world");
    }

    #[test]
    fn dedupes_exact_overlap_between_chunks() {
        let parts = vec![
            "the quick brown fox jumps".to_string(),
            "fox jumps over the lazy dog".to_string(),
        ];
        assert_eq!(
            join_transcripts(&parts, 2.0),
            "the quick brown fox jumps over the lazy dog"
        );
    }

    #[test]
    fn no_lexical_overlap_keeps_all_words() {
        // A pause spanning the overlap region: whisper produced no shared
        // words at the boundary. The old fixed-count heuristic would still
        // have blindly stripped 5 words here, dropping real content.
        let parts = vec!["a b c d e f g".to_string(), "h i j k l m n".to_string()];
        assert_eq!(join_transcripts(&parts, 2.0), "a b c d e f g h i j k l m n");
    }

    #[test]
    fn dense_fast_speech_dedupes_beyond_the_old_fixed_guess() {
        // 2.0s * 2.5 wps = 5 words — the old heuristic's fixed guess. Dense
        // fast speech can pack more real overlap than that into the same 2s
        // window; the search-based match still finds all of it exactly.
        let parts = vec![
            "one two three four five six seven eight".to_string(),
            "five six seven eight nine ten".to_string(),
        ];
        assert_eq!(
            join_transcripts(&parts, 2.0),
            "one two three four five six seven eight nine ten"
        );
    }

    #[test]
    fn overlap_match_tolerates_punctuation_and_case_differences() {
        let parts = vec![
            "hello there World.".to_string(),
            "world how are you".to_string(),
        ];
        assert_eq!(
            join_transcripts(&parts, 2.0),
            "hello there World. how are you"
        );
    }

    #[test]
    fn three_parts_dedupe_each_boundary_independently() {
        let parts = vec![
            "a b c d".to_string(),
            "c d e f".to_string(),
            "e f g h".to_string(),
        ];
        assert_eq!(join_transcripts(&parts, 2.0), "a b c d e f g h");
    }

    #[test]
    fn overlap_shorter_than_search_window_is_still_found_exactly() {
        // max_words = ceil(0.5 * 6.0) = 3, well above the real 1-word overlap.
        let parts = vec!["a x y z".to_string(), "z b c".to_string()];
        assert_eq!(join_transcripts(&parts, 0.5), "a x y z b c");
    }
}

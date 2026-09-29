pub mod anthropic;
pub mod ollama;
pub mod openai;

use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize};

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// How (and whether) raw transcripts are cleaned up before paste.
///
/// Serializes to the snake_case strings shared with the TS prefs contract:
/// `none`, `api`, `local`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupMode {
    None,
    Api,
    Local,
}

/// How heavily the cleanup prompt rewrites the transcript.
/// Serializes to `light`, `standard`, `heavy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupAggressiveness {
    Light,
    Standard,
    Heavy,
}

/// Cloud cleanup provider (BYOK). Serializes to `open_ai`, `anthropic`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiProvider {
    OpenAi,
    Anthropic,
}

#[derive(Debug)]
pub enum CleanupError {
    NoKeyConfigured,
    KeyRejected(String),
    Unavailable(String),
    /// The provider is reachable but the configured model isn't installed.
    /// Only produced by the Ollama Test probe (`verify_ollama`), never by a
    /// `clean()` call — but it shares the raw-paste fallback path if it ever does.
    ModelNotFound,
    InvalidProvider,
}

impl CleanupError {
    /// Stable snake_case wire code (LLD §600, `test_api_key`).
    pub fn code(&self) -> String {
        match self {
            CleanupError::NoKeyConfigured => "no_key_configured",
            CleanupError::KeyRejected(_) => "key_rejected",
            CleanupError::Unavailable(_) => "unavailable",
            CleanupError::ModelNotFound => "model_not_found",
            CleanupError::InvalidProvider => "invalid_provider",
        }
        .to_string()
    }
}

impl std::fmt::Display for CleanupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CleanupError::NoKeyConfigured => write!(f, "no API key configured"),
            CleanupError::KeyRejected(e) => write!(f, "key rejected: {e}"),
            CleanupError::Unavailable(e) => write!(f, "provider unavailable: {e}"),
            CleanupError::ModelNotFound => write!(f, "model not found"),
            CleanupError::InvalidProvider => write!(f, "invalid provider"),
        }
    }
}

pub trait CleanupProvider: Send + Sync {
    fn clean<'a>(&'a self, text: &'a str) -> BoxFuture<'a, Result<String, CleanupError>>;
}

/// Resolved cleanup settings, held in `AppState` and rebuilt when prefs change.
/// `provider` is an `Arc` (not `Box`) so the pipeline can clone a cheap snapshot
/// out of the `RwLock` and drop the guard *before* awaiting (a `std::sync`
/// read guard isn't `Send` across an `.await`).
pub struct CleanupConfig {
    pub mode: CleanupMode,
    pub provider: Option<std::sync::Arc<dyn CleanupProvider>>,
}

/// The single cleanup entry point from the pipeline (LLD §3.4). Maps the
/// configured mode and provider presence onto a result:
/// only `Err(NoKeyConfigured)` hard-stops paste; every other `Err` is turned
/// into a raw-transcript fallback by the pipeline's `cleanup_outcome` mapper.
pub async fn dispatch_cleanup(
    raw: &str,
    mode: CleanupMode,
    provider: Option<&dyn CleanupProvider>,
) -> Result<String, CleanupError> {
    match mode {
        CleanupMode::None => Ok(raw.to_string()),
        CleanupMode::Api => match provider {
            Some(p) => p.clean(raw).await,
            None => Err(CleanupError::NoKeyConfigured), // cloud selected, no key stored
        },
        CleanupMode::Local => match provider {
            Some(p) => p.clean(raw).await,
            None => Ok(raw.to_string()), // misconfigured local: no-op, never block paste
        },
    }
}

/// Default cloud cleanup models (MVP — not user-selectable).
const OPENAI_CLEANUP_MODEL: &str = "gpt-4o-mini";
const ANTHROPIC_CLEANUP_MODEL: &str = "claude-haiku-4-5-20251001";
/// Production cloud base URLs (aisdk defaults; overridable only in tests).
const OPENAI_BASE_URL: &str = "https://api.openai.com";
const ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com/v1/";

/// Build the cloud provider for the selected `ApiProvider` with a stored key.
fn build_cloud_provider(
    provider: ApiProvider,
    key: String,
    aggressiveness: CleanupAggressiveness,
) -> std::sync::Arc<dyn CleanupProvider> {
    match provider {
        ApiProvider::OpenAi => std::sync::Arc::new(openai::OpenAiProvider {
            api_key: key,
            model: OPENAI_CLEANUP_MODEL.to_string(),
            base_url: OPENAI_BASE_URL.to_string(),
            aggressiveness,
        }),
        ApiProvider::Anthropic => std::sync::Arc::new(anthropic::AnthropicProvider {
            api_key: key,
            model: ANTHROPIC_CLEANUP_MODEL.to_string(),
            base_url: ANTHROPIC_BASE_URL.to_string(),
            aggressiveness,
        }),
    }
}

/// Build the [`CleanupConfig`] from prefs. `read_key` is injected so this is
/// unit-testable without the real Keychain (production passes
/// `|p| prefs::read_api_key(p).ok().flatten()`). A cloud provider is built
/// when a key exists; no stored key yields `provider: None`.
pub fn build_cleanup_config(
    prefs: &crate::prefs::Prefs,
    read_key: impl Fn(ApiProvider) -> Option<String>,
) -> CleanupConfig {
    let provider: Option<std::sync::Arc<dyn CleanupProvider>> = match prefs.cleanup_mode {
        CleanupMode::None => None,
        CleanupMode::Local => Some(std::sync::Arc::new(ollama::OllamaProvider {
            base_url: prefs.ollama_base_url.clone(),
            model: prefs.ollama_model.clone(),
            aggressiveness: prefs.cleanup_aggressiveness,
        })),
        CleanupMode::Api => read_key(prefs.api_provider)
            .map(|key| build_cloud_provider(prefs.api_provider, key, prefs.cleanup_aggressiveness)),
    };
    CleanupConfig {
        mode: prefs.cleanup_mode,
        provider,
    }
}

/// Fix-only cleanup: correct obvious transcription errors, leave wording alone.
const PROMPT_LIGHT: &str = "You are a transcription cleanup tool. Fix only obvious \
speech-to-text errors (misheard words, wrong homophones) in the user's text. Do not \
change wording, grammar, punctuation, or style. Preserve the speaker's exact phrasing. \
Output only the corrected text, nothing else.";

/// Standard cleanup: grammar, punctuation, and filler removal; meaning preserved.
const PROMPT_STANDARD: &str = "You are a transcription cleanup tool. Fix grammar, add \
correct punctuation and capitalization, and remove filler words (um, uh, like, you know) \
from the user's text. Preserve the speaker's meaning, tone, and voice — do not rewrite or \
summarize. Output only the cleaned text, nothing else.";

/// Heavy cleanup: standard plus tightening phrasing and structure; meaning preserved.
const PROMPT_HEAVY: &str = "You are a transcription cleanup tool. Fix grammar, punctuation, \
and capitalization, remove filler words and false starts, and tighten phrasing and sentence \
structure for clarity. Preserve the speaker's original meaning and intent — do not add new \
information or summarize away content. Output only the cleaned text, nothing else.";

/// The system-prompt instruction for a given aggressiveness (LLD §277 — three
/// fixed const prompts). The raw transcript is supplied separately as the user
/// message by each provider.
pub fn build_prompt(aggressiveness: CleanupAggressiveness) -> &'static str {
    match aggressiveness {
        CleanupAggressiveness::Light => PROMPT_LIGHT,
        CleanupAggressiveness::Standard => PROMPT_STANDARD,
        CleanupAggressiveness::Heavy => PROMPT_HEAVY,
    }
}

/// LLD §277: every provider call is bounded by a 5s timeout.
const CLEANUP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Map an `aisdk::Error` to a [`CleanupError`]. **Privacy:** never carries the
/// `details`/response body — only the numeric HTTP status or an opaque label.
/// 401/403 → `KeyRejected`; everything else (other statuses, network, parse,
/// provider-specific) → `Unavailable`.
pub(crate) fn map_aisdk_error(e: aisdk::Error) -> CleanupError {
    if let aisdk::Error::ApiError {
        status_code: Some(code),
        ..
    } = e
    {
        if code == reqwest::StatusCode::UNAUTHORIZED || code == reqwest::StatusCode::FORBIDDEN {
            return CleanupError::KeyRejected(code.as_u16().to_string());
        }
        return CleanupError::Unavailable(code.as_u16().to_string());
    }
    // Network/timeout/parse/`MissingField` (empty key) all collapse here. We
    // deliberately use `Unavailable` (→ raw fallback) rather than
    // `NoKeyConfigured` (→ hard stop): an empty key is unreachable in practice
    // (`build_cleanup_config` only builds a provider when a key exists), and
    // when in doubt we never block the paste.
    CleanupError::Unavailable("provider error".to_string())
}

/// Word-count-based bounds for [`estimate_max_output_tokens`]. Cleanup should
/// never expand a transcript by much (Heavy aggressiveness only tightens
/// phrasing), so the ceiling exists purely to stop a rambling/hallucinating
/// model (small local Ollama models are especially prone to this) from
/// running away to thousands of tokens of unrelated text.
const MIN_OUTPUT_TOKENS: u32 = 64;
const MAX_OUTPUT_TOKENS: u32 = 2048;

/// Estimate a generous-but-bounded `max_output_tokens` from the raw transcript,
/// since this codebase has no tokenizer: ~2 tokens/word plus fixed headroom,
/// clamped to `[MIN_OUTPUT_TOKENS, MAX_OUTPUT_TOKENS]`.
fn estimate_max_output_tokens(text: &str) -> u32 {
    let word_count = text.split_whitespace().count() as u32;
    (word_count.saturating_mul(2).saturating_add(64)).clamp(MIN_OUTPUT_TOKENS, MAX_OUTPUT_TOKENS)
}

/// A cleaned response whose word count exceeds [`MAX_LENGTH_RATIO`] times the
/// raw transcript's word count, plus this fixed slack, is treated as a
/// runaway/hallucinated generation rather than a legitimate rewrite, even
/// under Heavy aggressiveness. The fixed slack matters for very short inputs:
/// whisper can transcribe a fast/short utterance as very few (even one, if
/// speech runs together) whitespace-delimited "words", and a legitimate
/// cleanup correction that splits that into several separate words must not
/// trip a ratio-only guard.
const MAX_LENGTH_RATIO: usize = 4;
const RUNAWAY_LENGTH_FIXED_SLACK: usize = 8;

/// Run one cleanup generation against any aisdk language model under the 5s
/// timeout, mapping the outcome to a [`CleanupError`]. Shared by all three
/// providers (the only thing that differs between them is how the model is
/// built). Never logs the prompt, transcript, or response.
pub(crate) async fn run_cleanup<P>(
    model: P,
    instruction: &str,
    text: &str,
) -> Result<String, CleanupError>
where
    P: aisdk::core::LanguageModel + aisdk::core::capabilities::TextInputSupport + Send,
{
    let mut builder = aisdk::core::LanguageModelRequest::builder()
        .model(model)
        .system(instruction)
        .prompt(text);
    // No fluent setter exists in aisdk 0.5.2 for this option, but the field is
    // `pub` on `LanguageModelOptions` and the builder derefs to it at every
    // stage, so direct assignment reaches the wire request. Keep `cap` so the
    // truncation guard below compares against the exact value sent on the wire.
    let cap = estimate_max_output_tokens(text);
    builder.max_output_tokens = Some(cap);
    let mut req = builder.build();
    match tokio::time::timeout(CLEANUP_TIMEOUT, req.generate_text()).await {
        Err(_elapsed) => Err(CleanupError::Unavailable("timeout".to_string())),
        Ok(Err(e)) => Err(map_aisdk_error(e)),
        Ok(Ok(resp)) => {
            let cleaned = resp.text().unwrap_or_default();
            // A model that returns nothing (None, empty, or whitespace) must not
            // silently *replace* the dictation with an empty string — fall back
            // to pasting the raw transcript via the Unavailable arm.
            if cleaned.trim().is_empty() {
                Err(CleanupError::Unavailable("empty response".to_string()))
            } else if is_runaway_length(text, &cleaned) {
                // Second line of defense beyond the token cap above: a model
                // that ignores both the cap and the "output only the text"
                // instruction and rambles/hallucinates is treated the same as
                // an unavailable provider — fall back to the raw transcript.
                Err(CleanupError::Unavailable(
                    "runaway response length".to_string(),
                ))
            } else if is_truncated(resp.usage().output_tokens, cap) {
                // A legitimate long dictation whose cleaned output saturated the
                // token cap was almost certainly cut off mid-sentence. Pasting a
                // truncated clean is worse than the complete raw transcript, so
                // fall back the same way. Checked after `is_runaway_length` so a
                // rambling model that also hit the cap reports the more
                // informative "runaway" rather than "truncated".
                Err(CleanupError::Unavailable("truncated response".to_string()))
            } else {
                Ok(cleaned)
            }
        }
    }
}

/// True if `cleaned` is implausibly longer than `raw` (see [`MAX_LENGTH_RATIO`]
/// and [`RUNAWAY_LENGTH_FIXED_SLACK`]). Uses word counts, consistent with
/// [`estimate_max_output_tokens`].
fn is_runaway_length(raw: &str, cleaned: &str) -> bool {
    let raw_words = raw.split_whitespace().count();
    let cleaned_words = cleaned.split_whitespace().count();
    cleaned_words > raw_words * MAX_LENGTH_RATIO + RUNAWAY_LENGTH_FIXED_SLACK
}

/// True when the model's generated output reached the requested token cap,
/// meaning the cleaned text was almost certainly cut off mid-sentence. aisdk
/// 0.5.2 parses the provider's truncation signal (Anthropic
/// `stop_reason:"max_tokens"`, OpenAI Responses
/// `incomplete_details.reason:"max_output_tokens"`) but discards it before it
/// reaches us — the core `StopReason` it synthesizes has no length variant — so
/// token usage is the only truncation signal available. When a provider reports
/// no usage (`None`) we can't tell, so we do NOT flag: behaviour is then
/// unchanged (no false fallback to the raw transcript).
fn is_truncated(output_tokens: Option<usize>, max_output_tokens: u32) -> bool {
    matches!(output_tokens, Some(n) if n >= max_output_tokens as usize)
}

/// Verify a cloud cleanup API key by making the cheapest authenticated request
/// the provider offers and mapping the HTTP outcome to a `CleanupError`.
/// `base_url` has no trailing slash (e.g. `https://api.openai.com/v1`); tests
/// pass a mockito server URL. Never logs the key, request, or response body.
pub async fn verify_key(
    provider: ApiProvider,
    key: &str,
    base_url: &str,
) -> Result<(), CleanupError> {
    let client = reqwest::Client::new();
    let req = match provider {
        ApiProvider::OpenAi => client.get(format!("{base_url}/models")).bearer_auth(key),
        ApiProvider::Anthropic => client
            .post(format!("{base_url}/messages"))
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01")
            .json(&serde_json::json!({
                "model": "claude-3-5-haiku-latest",
                "max_tokens": 1,
                "messages": [{ "role": "user", "content": "hi" }]
            })),
    };
    let resp = req
        .send()
        .await
        .map_err(|e| CleanupError::Unavailable(e.to_string()))?;
    let status = resp.status();
    // Both probes (`GET /models`, `POST /messages`) return exactly 200 on
    // success — require that precisely, since this is the gate that authorizes
    // persisting the key. Any other 2xx is anomalous and treated as unavailable.
    if status == reqwest::StatusCode::OK {
        Ok(())
    } else if status == reqwest::StatusCode::UNAUTHORIZED
        || status == reqwest::StatusCode::FORBIDDEN
    {
        Err(CleanupError::KeyRejected(status.as_u16().to_string()))
    } else {
        // Other statuses (429 rate-limit, 5xx, redirects, 2xx-not-200) are
        // ambiguous: don't store on them — verification is inconclusive.
        Err(CleanupError::Unavailable(status.as_u16().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Canned-result provider — proves `dispatch_cleanup` routing without a network.
    struct StubProvider(std::sync::Mutex<Option<Result<String, CleanupError>>>);
    impl StubProvider {
        fn new(r: Result<String, CleanupError>) -> Self {
            StubProvider(std::sync::Mutex::new(Some(r)))
        }
    }
    impl CleanupProvider for StubProvider {
        fn clean<'a>(&'a self, _text: &'a str) -> BoxFuture<'a, Result<String, CleanupError>> {
            let r = self.0.lock().unwrap().take().expect("clean called once");
            Box::pin(async move { r })
        }
    }

    #[tokio::test]
    async fn dispatch_none_returns_raw_unchanged() {
        let r = dispatch_cleanup("raw text", CleanupMode::None, None).await;
        assert_eq!(r.unwrap(), "raw text");
    }

    #[tokio::test]
    async fn dispatch_api_is_available_without_license() {
        let p = StubProvider::new(Ok("clean".into()));
        let result = dispatch_cleanup("raw", CleanupMode::Api, Some(&p)).await;
        assert_eq!(result.unwrap(), "clean");
    }

    #[tokio::test]
    async fn dispatch_api_pro_without_provider_is_no_key_configured() {
        let err = dispatch_cleanup("raw", CleanupMode::Api, None)
            .await
            .unwrap_err();
        assert_eq!(err.code(), "no_key_configured");
    }

    #[tokio::test]
    async fn dispatch_api_pro_with_provider_returns_cleaned() {
        let p = StubProvider::new(Ok("CLEANED".into()));
        let r = dispatch_cleanup("raw", CleanupMode::Api, Some(&p)).await;
        assert_eq!(r.unwrap(), "CLEANED");
    }

    #[tokio::test]
    async fn dispatch_local_with_provider_returns_cleaned() {
        let p = StubProvider::new(Ok("CLEANED".into()));
        let r = dispatch_cleanup("raw", CleanupMode::Local, Some(&p)).await;
        assert_eq!(r.unwrap(), "CLEANED");
    }

    #[tokio::test]
    async fn dispatch_local_without_provider_falls_back_to_raw() {
        let r = dispatch_cleanup("raw", CleanupMode::Local, None).await;
        assert_eq!(r.unwrap(), "raw");
    }

    #[tokio::test]
    async fn dispatch_propagates_provider_error() {
        let p = StubProvider::new(Err(CleanupError::Unavailable("503".into())));
        let err = dispatch_cleanup("raw", CleanupMode::Local, Some(&p))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "unavailable");
    }

    #[test]
    fn estimate_max_output_tokens_applies_floor_for_short_input() {
        assert_eq!(estimate_max_output_tokens("hi"), 2 + 64);
        assert_eq!(estimate_max_output_tokens(""), MIN_OUTPUT_TOKENS);
    }

    #[test]
    fn estimate_max_output_tokens_scales_with_word_count() {
        let ten_words = "one two three four five six seven eight nine ten";
        assert_eq!(estimate_max_output_tokens(ten_words), 10 * 2 + 64);
    }

    #[test]
    fn estimate_max_output_tokens_applies_ceiling_for_long_input() {
        let many_words = "word ".repeat(5000);
        assert_eq!(estimate_max_output_tokens(&many_words), MAX_OUTPUT_TOKENS);
    }

    #[test]
    fn runaway_length_flags_hallucinated_expansion() {
        assert!(is_runaway_length(
            "turn off the lights",
            &"word ".repeat(50)
        ));
    }

    #[test]
    fn runaway_length_allows_legitimate_rewrite() {
        // Standard cleanup removing filler words / adding punctuation shouldn't
        // come close to tripping the ratio guard.
        assert!(!is_runaway_length(
            "um so like turn off the lights please",
            "Turn off the lights, please."
        ));
    }

    #[test]
    fn runaway_length_has_floor_for_tiny_input() {
        // A one-word input shouldn't allow an unbounded absolute expansion
        // just because the ratio is nominally satisfied by a small model.
        assert!(!is_runaway_length("hi", "Hi there!"));
        assert!(is_runaway_length("hi", &"word ".repeat(20)));
    }

    #[test]
    fn runaway_length_allows_expansion_of_short_glued_input() {
        // Whisper can transcribe a fast/short utterance as a single
        // whitespace-delimited "word" if speech runs together; a legitimate
        // cleanup correction that splits it into several real words must not
        // be discarded by a ratio-only guard.
        assert!(!is_runaway_length(
            "cantfindmykeysanywhere",
            "I can't find my keys anywhere."
        ));
    }

    #[test]
    fn truncated_flags_output_at_or_over_cap() {
        // Reaching the requested cap means the model was cut off mid-sentence.
        assert!(is_truncated(Some(2048), 2048));
        assert!(is_truncated(Some(2100), 2048));
    }

    #[test]
    fn truncated_allows_output_under_cap() {
        // A response that finished before the cap completed naturally.
        assert!(!is_truncated(Some(2047), 2048));
        assert!(!is_truncated(Some(0), 2048));
    }

    #[test]
    fn truncated_does_not_flag_when_usage_unknown() {
        // Providers that report no usage give no truncation signal; we must not
        // discard an otherwise-good clean on a guess.
        assert!(!is_truncated(None, 2048));
    }

    fn prefs_with_mode(mode: CleanupMode) -> crate::prefs::Prefs {
        crate::prefs::Prefs {
            cleanup_mode: mode,
            ..crate::prefs::Prefs::default()
        }
    }

    #[test]
    fn build_config_none_has_no_provider() {
        let cfg = build_cleanup_config(&prefs_with_mode(CleanupMode::None), |_| Some("k".into()));
        assert_eq!(cfg.mode, CleanupMode::None);
        assert!(cfg.provider.is_none());
    }

    #[test]
    fn build_config_local_builds_provider_without_key() {
        let cfg = build_cleanup_config(&prefs_with_mode(CleanupMode::Local), |_| None);
        assert_eq!(cfg.mode, CleanupMode::Local);
        assert!(cfg.provider.is_some());
    }

    #[test]
    fn build_config_api_with_key_builds_provider() {
        let cfg = build_cleanup_config(&prefs_with_mode(CleanupMode::Api), |_| {
            Some("sk-test".into())
        });
        assert_eq!(cfg.mode, CleanupMode::Api);
        assert!(cfg.provider.is_some());
    }

    #[test]
    fn build_config_api_without_key_has_no_provider() {
        let cfg = build_cleanup_config(&prefs_with_mode(CleanupMode::Api), |_| None);
        assert_eq!(cfg.mode, CleanupMode::Api);
        assert!(cfg.provider.is_none());
    }

    #[test]
    fn build_config_api_with_anthropic_provider_selected_builds_anthropic() {
        // `Prefs::default()` picks `ApiProvider::OpenAi`, so this must explicitly
        // override `api_provider` to reach `build_cloud_provider`'s Anthropic arm.
        let prefs = crate::prefs::Prefs {
            cleanup_mode: CleanupMode::Api,
            api_provider: ApiProvider::Anthropic,
            ..crate::prefs::Prefs::default()
        };
        let cfg = build_cleanup_config(&prefs, |_| Some("sk-ant-test".into()));
        assert_eq!(cfg.mode, CleanupMode::Api);
        assert!(cfg.provider.is_some());
    }

    #[test]
    fn cleanup_error_display_covers_every_variant() {
        assert_eq!(
            CleanupError::NoKeyConfigured.to_string(),
            "no API key configured"
        );
        assert_eq!(
            CleanupError::KeyRejected("401".to_string()).to_string(),
            "key rejected: 401"
        );
        assert_eq!(
            CleanupError::Unavailable("503".to_string()).to_string(),
            "provider unavailable: 503"
        );
        assert_eq!(CleanupError::ModelNotFound.to_string(), "model not found");
        assert_eq!(
            CleanupError::InvalidProvider.to_string(),
            "invalid provider"
        );
    }

    #[test]
    fn build_prompt_is_distinct_and_nonempty_per_aggressiveness() {
        let light = build_prompt(CleanupAggressiveness::Light);
        let standard = build_prompt(CleanupAggressiveness::Standard);
        let heavy = build_prompt(CleanupAggressiveness::Heavy);
        for p in [light, standard, heavy] {
            assert!(!p.is_empty());
        }
        assert_ne!(light, standard);
        assert_ne!(standard, heavy);
        assert_ne!(light, heavy);
    }

    #[tokio::test]
    async fn openai_200_verifies() {
        let mut server = mockito::Server::new_async().await;
        // Guard the auth-header contract, not just path+method.
        let m = server
            .mock("GET", "/models")
            .match_header("authorization", "Bearer sk-test")
            .with_status(200)
            .create_async()
            .await;
        let r = verify_key(ApiProvider::OpenAi, "sk-test", &server.url()).await;
        assert!(r.is_ok());
        m.assert_async().await;
    }

    #[tokio::test]
    async fn openai_401_is_key_rejected() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/models")
            .with_status(401)
            .create_async()
            .await;
        let err = verify_key(ApiProvider::OpenAi, "bad", &server.url())
            .await
            .unwrap_err();
        assert_eq!(err.code(), "key_rejected");
    }

    #[tokio::test]
    async fn openai_403_is_key_rejected() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/models")
            .with_status(403)
            .create_async()
            .await;
        let err = verify_key(ApiProvider::OpenAi, "bad", &server.url())
            .await
            .unwrap_err();
        assert_eq!(err.code(), "key_rejected");
    }

    #[tokio::test]
    async fn openai_500_is_unavailable() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/models")
            .with_status(500)
            .create_async()
            .await;
        let err = verify_key(ApiProvider::OpenAi, "sk", &server.url())
            .await
            .unwrap_err();
        assert_eq!(err.code(), "unavailable");
    }

    #[tokio::test]
    async fn rate_limit_429_is_unavailable_not_rejected() {
        // A 429 means the key is likely valid but the check is inconclusive —
        // deliberately `unavailable` (don't store), never `key_rejected`.
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/models")
            .with_status(429)
            .create_async()
            .await;
        let err = verify_key(ApiProvider::OpenAi, "sk", &server.url())
            .await
            .unwrap_err();
        assert_eq!(err.code(), "unavailable");
    }

    #[tokio::test]
    async fn anthropic_200_verifies() {
        let mut server = mockito::Server::new_async().await;
        // Guard the auth + version headers, not just path+method.
        let m = server
            .mock("POST", "/messages")
            .match_header("x-api-key", "sk-ant")
            .match_header("anthropic-version", "2023-06-01")
            .with_status(200)
            .create_async()
            .await;
        let r = verify_key(ApiProvider::Anthropic, "sk-ant", &server.url()).await;
        assert!(r.is_ok());
        m.assert_async().await;
    }

    #[tokio::test]
    async fn anthropic_401_is_key_rejected() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("POST", "/messages")
            .with_status(401)
            .create_async()
            .await;
        let err = verify_key(ApiProvider::Anthropic, "bad", &server.url())
            .await
            .unwrap_err();
        assert_eq!(err.code(), "key_rejected");
    }

    #[tokio::test]
    async fn network_error_is_unavailable() {
        // Port 1 is unbound → connection refused → reqwest send error.
        let err = verify_key(ApiProvider::OpenAi, "sk", "http://127.0.0.1:1")
            .await
            .unwrap_err();
        assert_eq!(err.code(), "unavailable");
    }
}

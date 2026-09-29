//! Local Ollama cleanup provider (OpenAI-compatible, no key).
//!
//! Built with aisdk's `OpenAICompatible` provider pointed at the user's Ollama
//! `base_url` (from prefs). Ollama needs no auth, but aisdk's `build()` rejects
//! an empty key, so a constant `"ollama"` placeholder is passed. Driven through
//! the shared `run_cleanup` helper (5s timeout); with no auth, errors collapse
//! to `Unavailable` (connection refused / timeout / bad response).

use std::future::Future;
use std::pin::Pin;

use aisdk::core::DynamicModel;
use aisdk::providers::OpenAICompatible;

use super::{
    build_prompt, map_aisdk_error, run_cleanup, CleanupAggressiveness, CleanupError,
    CleanupProvider,
};

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Ollama has no API key; aisdk's builder requires a non-empty one, so we pass
/// this inert placeholder.
const OLLAMA_PLACEHOLDER_KEY: &str = "ollama";

/// Local cleanup via an Ollama server speaking the OpenAI-compatible API. Never
/// logs the transcript.
pub struct OllamaProvider {
    pub base_url: String,
    pub model: String,
    pub aggressiveness: CleanupAggressiveness,
}

impl CleanupProvider for OllamaProvider {
    fn clean<'a>(&'a self, text: &'a str) -> BoxFuture<'a, Result<String, CleanupError>> {
        Box::pin(async move {
            let model = OpenAICompatible::<DynamicModel>::builder()
                .model_name(&self.model)
                .api_key(OLLAMA_PLACEHOLDER_KEY)
                .base_url(&self.base_url)
                .build()
                .map_err(map_aisdk_error)?;
            run_cleanup(model, build_prompt(self.aggressiveness), text).await
        })
    }
}

/// Bound the Test probe so an unresponsive server fails fast instead of hanging
/// the Preferences button.
const VERIFY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Verify an Ollama setup for the Preferences **Test** button: the server is
/// reachable **and** the configured model is installed. Probes the
/// OpenAI-compatible `GET {base}/models` list (which Ollama implements) and
/// checks the model is present. Never logs anything sensitive — there's nothing
/// to log but the public model list.
///
/// - Connection refused / timeout / send error / non-200 → [`CleanupError::Unavailable`].
/// - 200 but the model isn't in the list → [`CleanupError::ModelNotFound`].
/// - 200 and the model is present → `Ok(())`.
///
/// Ollama tags models as `llama3:latest` while users typically type `llama3`, so
/// a bare name matches any `name` or `name:<tag>` entry.
pub async fn verify_ollama(base_url: &str, model: &str) -> Result<(), CleanupError> {
    // The default pref base URL carries a trailing slash (`.../v1/`); trim it so
    // we don't produce a `//models` path.
    let base = base_url.trim_end_matches('/');
    let resp = reqwest::Client::new()
        .get(format!("{base}/models"))
        .timeout(VERIFY_TIMEOUT)
        .send()
        .await
        .map_err(|e| CleanupError::Unavailable(e.to_string()))?;
    let status = resp.status();
    if status != reqwest::StatusCode::OK {
        return Err(CleanupError::Unavailable(status.as_u16().to_string()));
    }
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| CleanupError::Unavailable(e.to_string()))?;
    let present = body["data"]
        .as_array()
        .map(|models| {
            models.iter().any(|m| {
                m["id"]
                    .as_str()
                    .is_some_and(|id| id == model || id.starts_with(&format!("{model}:")))
            })
        })
        .unwrap_or(false);
    if present {
        Ok(())
    } else {
        Err(CleanupError::ModelNotFound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(base_url: String) -> OllamaProvider {
        OllamaProvider {
            base_url,
            model: "llama3".to_string(),
            aggressiveness: CleanupAggressiveness::Standard,
        }
    }

    /// 200 success exercises aisdk's real OpenAI-compatible `/chat/completions`
    /// request→parse→`.text()` against an Ollama-shaped response.
    #[tokio::test]
    async fn ok_200_returns_cleaned_text() {
        let mut server = mockito::Server::new_async().await;
        let body = r#"{"id":"x","object":"chat.completion","created":0,"model":"llama3","choices":[{"index":0,"message":{"role":"assistant","content":"CLEANED OLL"}}]}"#;
        server
            .mock("POST", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;
        let out = provider(server.url()).clean("messy input").await.unwrap();
        assert_eq!(out, "CLEANED OLL");
    }

    /// Proves the token cap computed in `run_cleanup` actually reaches the wire
    /// request, not just the in-memory `LanguageModelOptions` struct: the mock
    /// only matches if the body contains `max_completion_tokens`, so an
    /// unmatched request (and thus a non-2xx/connection response) would fail
    /// this test with `unavailable` instead of a successful clean.
    #[tokio::test]
    async fn request_includes_max_completion_tokens() {
        let mut server = mockito::Server::new_async().await;
        let body = r#"{"id":"x","object":"chat.completion","created":0,"model":"llama3","choices":[{"index":0,"message":{"role":"assistant","content":"CLEANED OLL"}}]}"#;
        let m = server
            .mock("POST", mockito::Matcher::Any)
            .match_body(mockito::Matcher::Regex("\"max_completion_tokens\"".into()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;
        let out = provider(server.url()).clean("messy input").await.unwrap();
        assert_eq!(out, "CLEANED OLL");
        m.assert_async().await;
    }

    /// A model that ignores the token cap and the "output only the text"
    /// instruction and rambles far beyond the input length must not be pasted
    /// verbatim — `run_cleanup`'s length-ratio guard should fall it back to
    /// `Unavailable` (→ raw transcript, via the pipeline).
    #[tokio::test]
    async fn runaway_response_falls_back_to_unavailable() {
        let mut server = mockito::Server::new_async().await;
        let rambling: String = "word ".repeat(50);
        let body = serde_json::json!({
            "id": "x", "object": "chat.completion", "created": 0, "model": "llama3",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": rambling}}]
        })
        .to_string();
        server
            .mock("POST", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;
        let err = provider(server.url())
            .clean("turn off the lights")
            .await
            .unwrap_err();
        assert_eq!(err.code(), "unavailable");
    }

    #[tokio::test]
    async fn server_error_is_unavailable() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("POST", mockito::Matcher::Any)
            .with_status(500)
            .create_async()
            .await;
        let err = provider(server.url()).clean("hi").await.unwrap_err();
        assert_eq!(err.code(), "unavailable");
    }

    #[tokio::test]
    async fn connection_refused_is_unavailable() {
        // Ollama not running (unbound port) → connection refused → Unavailable.
        let err = provider("http://127.0.0.1:1".to_string())
            .clean("hi")
            .await
            .unwrap_err();
        assert_eq!(err.code(), "unavailable");
    }

    /// `GET /models` body shaped like Ollama's OpenAI-compatible model list.
    fn models_body(ids: &[&str]) -> String {
        let data: Vec<_> = ids
            .iter()
            .map(|id| serde_json::json!({ "id": id, "object": "model" }))
            .collect();
        serde_json::json!({ "object": "list", "data": data }).to_string()
    }

    async fn mock_models(server: &mut mockito::Server, body: String) {
        server
            .mock("GET", "/models")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;
    }

    #[tokio::test]
    async fn verify_ok_when_model_present() {
        let mut server = mockito::Server::new_async().await;
        mock_models(&mut server, models_body(&["llama3", "qwen2"])).await;
        assert!(verify_ollama(&server.url(), "llama3").await.is_ok());
    }

    #[tokio::test]
    async fn verify_ok_when_model_matches_tag() {
        // User types `llama3`; Ollama lists the tagged `llama3:latest`.
        let mut server = mockito::Server::new_async().await;
        mock_models(&mut server, models_body(&["llama3:latest"])).await;
        assert!(verify_ollama(&server.url(), "llama3").await.is_ok());
    }

    #[tokio::test]
    async fn verify_model_not_found_when_absent() {
        let mut server = mockito::Server::new_async().await;
        mock_models(&mut server, models_body(&["qwen2", "mistral"])).await;
        let err = verify_ollama(&server.url(), "llama3").await.unwrap_err();
        assert_eq!(err.code(), "model_not_found");
    }

    #[tokio::test]
    async fn verify_server_error_is_unavailable() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/models")
            .with_status(500)
            .create_async()
            .await;
        let err = verify_ollama(&server.url(), "llama3").await.unwrap_err();
        assert_eq!(err.code(), "unavailable");
    }

    #[tokio::test]
    async fn verify_connection_refused_is_unavailable() {
        // Ollama not running (unbound port) → connection refused → Unavailable.
        let err = verify_ollama("http://127.0.0.1:1", "llama3")
            .await
            .unwrap_err();
        assert_eq!(err.code(), "unavailable");
    }

    #[tokio::test]
    async fn verify_trims_trailing_slash_in_base_url() {
        // The default pref base URL ends in `/`; the probe must still hit `/models`.
        let mut server = mockito::Server::new_async().await;
        mock_models(&mut server, models_body(&["llama3"])).await;
        let base = format!("{}/", server.url());
        assert!(verify_ollama(&base, "llama3").await.is_ok());
    }
}

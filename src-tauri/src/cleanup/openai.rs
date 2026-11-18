//! OpenAI cloud cleanup provider (BYOK).
//!
//! aisdk API (resolved against aisdk 0.5.2 source): the provider is built with
//! `OpenAI::<DynamicModel>::builder().model_name(..).api_key(..).base_url(..).build()`
//! (`build()` errors if the key is empty), then driven through the shared
//! `run_cleanup` helper which issues a `LanguageModelRequest` (system =
//! aggressiveness prompt, user = transcript), bounds it at 5s, and maps the
//! result. The OpenAI provider targets the `/v1/responses` API.

use std::future::Future;
use std::pin::Pin;

use aisdk::core::DynamicModel;
use aisdk::providers::OpenAI;

use super::{
    build_prompt, map_aisdk_error, run_cleanup, CleanupAggressiveness, CleanupError,
    CleanupProvider,
};

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Cloud cleanup via OpenAI. `base_url` is overridable so tests point it at a
/// mockito server; production passes the OpenAI default. Never logs the key or
/// transcript.
pub struct OpenAiProvider {
    pub api_key: String,
    pub model: String,
    pub base_url: String,
    pub aggressiveness: CleanupAggressiveness,
}

impl CleanupProvider for OpenAiProvider {
    fn identity(&self) -> Option<(&str, &str)> {
        Some(("open_ai", &self.model))
    }

    fn clean<'a>(&'a self, text: &'a str) -> BoxFuture<'a, Result<String, CleanupError>> {
        Box::pin(async move {
            if self.model == "gpt-6-luna" {
                return self.clean_luna(text).await;
            }
            let model = OpenAI::<DynamicModel>::builder()
                .model_name(&self.model)
                .api_key(&self.api_key)
                .base_url(&self.base_url)
                .build()
                .map_err(map_aisdk_error)?;
            run_cleanup(model, build_prompt(self.aggressiveness), text, false).await
        })
    }
}

impl OpenAiProvider {
    async fn clean_luna(&self, text: &str) -> Result<String, CleanupError> {
        use super::{estimate_max_output_tokens, usage, validate_output, CLEANUP_TIMEOUT};

        let cap = estimate_max_output_tokens(text);
        let client = reqwest::Client::builder()
            .timeout(CLEANUP_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| CleanupError::Unavailable("client error".into()))?;
        let response = client
            .post(format!(
                "{}/v1/responses",
                self.base_url.trim_end_matches('/')
            ))
            .bearer_auth(&self.api_key)
            .json(&serde_json::json!({
                "model": self.model,
                "instructions": build_prompt(self.aggressiveness),
                "input": text,
                "max_output_tokens": cap,
                "reasoning": {"effort": "none"},
                "store": false
            }))
            .send()
            .await
            .map_err(|_| CleanupError::Unavailable("network or timeout".into()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(super::http_error(status));
        }
        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|_| CleanupError::Unavailable("invalid response".into()))?;
        let input_tokens = body["usage"]["input_tokens"].as_u64();
        let output_tokens = body["usage"]["output_tokens"].as_u64();
        usage::report(usage::TokenCounts {
            input_tokens,
            output_tokens,
        });
        if body["status"] == "incomplete" {
            return Err(CleanupError::Unavailable("incomplete response".into()));
        }
        let cleaned = body["output"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|item| item["type"] == "message")
            .flat_map(|item| item["content"].as_array().into_iter().flatten())
            .filter(|part| part["type"] == "output_text")
            .filter_map(|part| part["text"].as_str())
            .collect::<String>();
        validate_output(
            text,
            cleaned,
            output_tokens.and_then(|count| usize::try_from(count).ok()),
            cap,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn luna_reports_tokens_even_when_output_is_rejected() {
        let mut server = mockito::Server::new_async().await;
        let request = server
            .mock("POST", "/v1/responses")
            .match_body(mockito::Matcher::PartialJson(serde_json::json!({
                "model": "gpt-6-luna", "reasoning": {"effort": "none"}, "store": false
            })))
            .with_status(200)
            .with_body(r#"{"status":"incomplete","usage":{"input_tokens":20,"output_tokens":68}}"#)
            .create_async()
            .await;
        let mut provider = provider(server.url());
        provider.model = "gpt-6-luna".into();
        let (result, attempt) = super::super::usage::measure(&provider, "messy input").await;
        assert!(result.is_err());
        let counts = attempt.unwrap().counts;
        assert_eq!(counts.input_tokens, Some(20));
        assert_eq!(counts.output_tokens, Some(68));
        request.assert_async().await;
    }

    fn provider(base_url: String) -> OpenAiProvider {
        OpenAiProvider {
            api_key: "sk-test".to_string(),
            model: "gpt-4o-mini".to_string(),
            base_url,
            aggressiveness: CleanupAggressiveness::Standard,
        }
    }

    // The privacy-critical logic we own is the *error mapping*; these tests pin
    // it with `Matcher::Any` (no response-body parsing — the `/v1/responses`
    // success shape is aisdk-internal). The raw→cleaned success path is asserted
    // end-to-end at the dispatch/pipeline layer with a stub provider.

    /// 200 success exercises aisdk's real `/v1/responses` request→parse→`.text()`
    /// path (a minimal valid Responses body), so a future aisdk codec change is
    /// caught in CI rather than only by manual smoke.
    #[tokio::test]
    async fn ok_200_returns_cleaned_text() {
        let mut server = mockito::Server::new_async().await;
        let body = r#"{"output":[{"type":"message","role":"assistant","content":[{"type":"output_text","annotations":[],"logprobs":[],"text":"CLEANED OAI"}]}]}"#;
        server
            .mock("POST", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;
        let out = provider(server.url()).clean("messy input").await.unwrap();
        assert_eq!(out, "CLEANED OAI");
    }

    /// An empty model completion must NOT replace the transcript — it maps to
    /// `Unavailable` so the pipeline falls back to pasting the raw text. Covers
    /// the shared `run_cleanup` empty-guard for all three providers.
    #[tokio::test]
    async fn empty_completion_is_unavailable_not_empty_ok() {
        let mut server = mockito::Server::new_async().await;
        let body = r#"{"output":[{"type":"message","role":"assistant","content":[{"type":"output_text","annotations":[],"logprobs":[],"text":"   "}]}]}"#;
        server
            .mock("POST", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;
        let err = provider(server.url())
            .clean("messy input")
            .await
            .unwrap_err();
        assert_eq!(err.code(), "unavailable");
    }

    /// A completion whose reported `usage.output_tokens` reached the request's
    /// output-token cap is treated as truncated → `Unavailable`, so the pipeline
    /// pastes the raw transcript instead of a mid-sentence fragment. Exercises
    /// the shared `run_cleanup` truncation guard end-to-end (usage is parsed off
    /// the real `/v1/responses` body). The cap for the 2-word input
    /// "messy input" is `estimate_max_output_tokens` = 2*2+64 = 68; the short
    /// "CLEANED OAI" text stays under the runaway guard so truncation is the arm
    /// under test.
    #[tokio::test]
    async fn cap_saturating_completion_is_unavailable() {
        let mut server = mockito::Server::new_async().await;
        let body = r#"{"output":[{"type":"message","role":"assistant","content":[{"type":"output_text","annotations":[],"logprobs":[],"text":"CLEANED OAI"}]}],"usage":{"input_tokens":10,"input_tokens_details":{"cached_tokens":0},"output_tokens":68,"output_tokens_details":{"reasoning_tokens":0},"total_tokens":78}}"#;
        server
            .mock("POST", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;
        let err = provider(server.url())
            .clean("messy input")
            .await
            .unwrap_err();
        assert_eq!(err.code(), "unavailable");
    }

    #[tokio::test]
    async fn unauthorized_401_is_key_rejected() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("POST", mockito::Matcher::Any)
            .with_status(401)
            .create_async()
            .await;
        let err = provider(server.url()).clean("hi").await.unwrap_err();
        assert_eq!(err.code(), "key_rejected");
    }

    #[tokio::test]
    async fn forbidden_403_is_key_rejected() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("POST", mockito::Matcher::Any)
            .with_status(403)
            .create_async()
            .await;
        let err = provider(server.url()).clean("hi").await.unwrap_err();
        assert_eq!(err.code(), "key_rejected");
    }

    #[tokio::test]
    async fn server_error_500_is_unavailable() {
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
        // Unbound port → connection refused → reqwest send error → Unavailable.
        let err = provider("http://127.0.0.1:1".to_string())
            .clean("hi")
            .await
            .unwrap_err();
        assert_eq!(err.code(), "unavailable");
    }
}

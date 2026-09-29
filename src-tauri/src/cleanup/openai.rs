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
    fn clean<'a>(&'a self, text: &'a str) -> BoxFuture<'a, Result<String, CleanupError>> {
        Box::pin(async move {
            let model = OpenAI::<DynamicModel>::builder()
                .model_name(&self.model)
                .api_key(&self.api_key)
                .base_url(&self.base_url)
                .build()
                .map_err(map_aisdk_error)?;
            run_cleanup(model, build_prompt(self.aggressiveness), text).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

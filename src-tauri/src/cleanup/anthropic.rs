//! Anthropic cloud cleanup provider (BYOK).
//!
//! Same shape as [`super::openai`]: built with
//! `Anthropic::<DynamicModel>::builder().model_name(..).api_key(..).base_url(..).build()`
//! then driven through the shared `run_cleanup` helper (5s timeout, identical
//! error mapping — 401/403 → `KeyRejected`, else → `Unavailable`).

use std::future::Future;
use std::pin::Pin;

use aisdk::core::DynamicModel;
use aisdk::providers::Anthropic;

use super::{
    build_prompt, map_aisdk_error, run_cleanup, CleanupAggressiveness, CleanupError,
    CleanupProvider,
};

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Cloud cleanup via Anthropic. `base_url` is overridable for tests. Never logs
/// the key or transcript.
pub struct AnthropicProvider {
    pub api_key: String,
    pub model: String,
    pub base_url: String,
    pub aggressiveness: CleanupAggressiveness,
}

impl CleanupProvider for AnthropicProvider {
    fn clean<'a>(&'a self, text: &'a str) -> BoxFuture<'a, Result<String, CleanupError>> {
        Box::pin(async move {
            let model = Anthropic::<DynamicModel>::builder()
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

    fn provider(base_url: String) -> AnthropicProvider {
        AnthropicProvider {
            api_key: "sk-ant".to_string(),
            model: "claude-haiku-4-5-20251001".to_string(),
            base_url,
            aggressiveness: CleanupAggressiveness::Standard,
        }
    }

    // Error mapping is the logic we own (the success body shape is aisdk-internal
    // — asserted at the dispatch/pipeline layer with a stub provider).

    /// 200 success exercises aisdk's real `/v1/messages` request→parse→`.text()`.
    #[tokio::test]
    async fn ok_200_returns_cleaned_text() {
        let mut server = mockito::Server::new_async().await;
        let body = r#"{"id":"msg_1","content":[{"type":"text","text":"CLEANED ANT"}],"model":"claude-haiku-4-5-20251001","stop_reason":"end_turn","type":"message","usage":{"cache_creation":{"ephemeral_1h_input_tokens":0,"ephemeral_5m_input_tokens":0},"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"input_tokens":1,"output_tokens":1,"service_tier":"standard"}}"#;
        server
            .mock("POST", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;
        let out = provider(server.url()).clean("messy input").await.unwrap();
        assert_eq!(out, "CLEANED ANT");
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
        let err = provider("http://127.0.0.1:1".to_string())
            .clean("hi")
            .await
            .unwrap_err();
        assert_eq!(err.code(), "unavailable");
    }
}

/// Shared logger construction for shipping builds and privacy regression tests.
pub(crate) fn builder() -> tauri_plugin_log::Builder {
    // aisdk logs raw provider error bodies before our error mapper runs. Those
    // bodies can echo keys or transcripts; keep SDK diagnostics out of every
    // sink, in debug and release. Our own status-only diagnostics remain enabled.
    tauri_plugin_log::Builder::default()
        .level(log::LevelFilter::Info)
        .level_for("aisdk", log::LevelFilter::Off)
}

#[cfg(test)]
mod tests {
    use crate::cleanup::{openai::OpenAiProvider, CleanupAggressiveness, CleanupProvider};
    use tauri_plugin_log::{Target, TargetKind};

    #[tokio::test]
    async fn sdk_error_bodies_never_reach_the_application_log() {
        let dir = tempfile::tempdir().unwrap();
        let app = tauri::test::mock_app();
        let (_, level, logger) = super::builder()
            .clear_targets()
            .target(Target::new(TargetKind::Folder {
                path: dir.path().to_owned(),
                file_name: Some("privacy".into()),
            }))
            .split(app.handle())
            .unwrap();
        tauri_plugin_log::attach_logger(level, logger).unwrap();

        let mut server = mockito::Server::new_async().await;
        let secret = "SENTINEL_KEY_NOT_FOR_LOGS";
        let transcript = "SENTINEL_TRANSCRIPT_NOT_FOR_LOGS";
        let request = server
            .mock("POST", mockito::Matcher::Any)
            .with_status(401)
            .with_body(format!("{secret}: {transcript}"))
            .create_async()
            .await;
        let provider = OpenAiProvider {
            api_key: secret.into(),
            model: "gpt-4o-mini".into(),
            base_url: server.url(),
            aggressiveness: CleanupAggressiveness::Standard,
        };
        let error = provider.clean(transcript).await.unwrap_err();
        assert_eq!(error.code(), "key_rejected");
        let wire_error = serde_json::to_string(&crate::error::AppError::from(error)).unwrap();
        assert!(!wire_error.contains(secret));
        assert!(!wire_error.contains(transcript));
        request.assert_async().await;

        log::info!("privacy-control-marker");
        log::logger().flush();
        let logged = std::fs::read_to_string(dir.path().join("privacy.log")).unwrap();
        assert!(
            logged.contains("privacy-control-marker"),
            "the real log sink must be exercised"
        );
        assert!(
            !logged.contains(secret),
            "SDK response leaked credential material"
        );
        assert!(
            !logged.contains(transcript),
            "SDK response leaked transcript material"
        );
    }
}

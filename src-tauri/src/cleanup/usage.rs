use serde::{Deserialize, Serialize};
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenCounts {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupAttempt {
    pub provider: String,
    pub model: String,
    pub counts: TokenCounts,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    latest: Option<CleanupAttempt>,
    input_tokens: u64,
    output_tokens: u64,
}

tokio::task_local! {
    pub(super) static COUNTS: Cell<TokenCounts>;
}

pub(super) fn report(counts: TokenCounts) {
    let _ = COUNTS.try_with(|slot| slot.set(counts));
}

pub async fn measure(
    provider: &dyn super::CleanupProvider,
    raw: &str,
) -> (Result<String, super::CleanupError>, Option<CleanupAttempt>) {
    let result = COUNTS
        .scope(Cell::new(TokenCounts::default()), async {
            let result = provider.clean(raw).await;
            (result, COUNTS.with(Cell::get))
        })
        .await;
    let attempt = provider.identity().map(|(provider, model)| CleanupAttempt {
        provider: provider.into(),
        model: model.into(),
        counts: result.1,
    });
    (result.0, attempt)
}

fn path() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    Some(crate::profile::data_root(&home).join("cleanup-usage.json"))
}

fn read_from(path: &Path) -> std::io::Result<UsageSummary> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(UsageSummary::default()),
        Err(error) => Err(error),
    }
}

fn summary() -> &'static Mutex<std::io::Result<UsageSummary>> {
    static SUMMARY: OnceLock<Mutex<std::io::Result<UsageSummary>>> = OnceLock::new();
    SUMMARY.get_or_init(|| {
        let loaded = path()
            .ok_or_else(|| std::io::Error::other("Missing home directory"))
            .and_then(|path| read_from(&path));
        Mutex::new(loaded)
    })
}

pub fn load() -> std::io::Result<UsageSummary> {
    summary()
        .lock()
        .unwrap()
        .as_ref()
        .cloned()
        .map_err(|error| std::io::Error::from(error.kind()))
}

pub fn record(attempt: CleanupAttempt) {
    let mut loaded = summary().lock().unwrap();
    // Keep the load failure for this session so saving cannot replace unreadable data.
    let Ok(summary) = loaded.as_mut() else {
        return;
    };
    summary.input_tokens = summary
        .input_tokens
        .saturating_add(attempt.counts.input_tokens.unwrap_or(0));
    summary.output_tokens = summary
        .output_tokens
        .saturating_add(attempt.counts.output_tokens.unwrap_or(0));
    summary.latest = Some(attempt);
}

pub fn persist() -> std::io::Result<()> {
    static WRITER: Mutex<()> = Mutex::new(());
    let _guard = WRITER.lock().unwrap();
    let path = path().ok_or_else(|| std::io::Error::other("Missing home directory"))?;
    write_to(&path, &load()?)
}

fn write_to(path: &Path, summary: &UsageSummary) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, serde_json::to_vec(summary)?)?;
    if let Err(error) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(temporary);
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_survives_reload_and_reports_unreadable_data() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("usage.json");
        assert!(read_from(&path).unwrap().latest.is_none());
        let summary = UsageSummary {
            latest: Some(CleanupAttempt {
                provider: "ollama".into(),
                model: "llama3".into(),
                counts: TokenCounts {
                    input_tokens: Some(12),
                    output_tokens: None,
                },
            }),
            input_tokens: 24,
            output_tokens: 5,
        };
        write_to(&path, &summary).unwrap();
        let loaded = read_from(&path).unwrap();
        assert_eq!(loaded.input_tokens, 24);
        assert_eq!(loaded.output_tokens, 5);
        assert_eq!(loaded.latest.unwrap().counts.output_tokens, None);
        std::fs::write(&path, b"invalid json").unwrap();
        assert!(read_from(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"invalid json");
    }
}

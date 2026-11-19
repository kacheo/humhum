use chrono::{Local, Months, NaiveDate};
use serde::{Deserialize, Serialize};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenCounts {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

impl TokenCounts {
    fn add(&mut self, counts: Self) {
        fn add_reported(total: &mut Option<u64>, count: Option<u64>) {
            if let Some(count) = count {
                *total = Some(total.unwrap_or(0).saturating_add(count));
            }
        }

        add_reported(&mut self.input_tokens, counts.input_tokens);
        add_reported(&mut self.output_tokens, counts.output_tokens);
    }
}

pub struct CleanupAttempt {
    date: NaiveDate,
    pub(super) counts: TokenCounts,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageSummary {
    input_tokens: u64,
    output_tokens: u64,
    #[serde(default)]
    days: BTreeMap<NaiveDate, TokenCounts>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReportedCounts {
    input_tokens: Option<String>,
    output_tokens: Option<String>,
}

impl From<TokenCounts> for ReportedCounts {
    fn from(counts: TokenCounts) -> Self {
        Self {
            input_tokens: counts.input_tokens.map(|count| count.to_string()),
            output_tokens: counts.output_tokens.map(|count| count.to_string()),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LifetimeCounts {
    input_tokens: String,
    output_tokens: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonthlyUsage {
    month: String,
    current_month: String,
    first_month: Option<String>,
    days: BTreeMap<NaiveDate, ReportedCounts>,
    totals: ReportedCounts,
    lifetime: LifetimeCounts,
}

fn month_key(date: NaiveDate) -> String {
    date.format("%Y-%m").to_string()
}

impl UsageSummary {
    fn record(&mut self, date: NaiveDate, counts: TokenCounts) {
        self.input_tokens = self
            .input_tokens
            .saturating_add(counts.input_tokens.unwrap_or(0));
        self.output_tokens = self
            .output_tokens
            .saturating_add(counts.output_tokens.unwrap_or(0));
        self.days.entry(date).or_default().add(counts);
    }

    fn for_month(&self, month: Option<&str>, today: NaiveDate) -> std::io::Result<MonthlyUsage> {
        let current_month = month_key(today);
        let month = month.unwrap_or(&current_month);
        let start = NaiveDate::parse_from_str(&format!("{month}-01"), "%Y-%m-%d")
            .ok()
            .filter(|date| month.len() == 7 && month_key(*date) == month)
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "Expected YYYY-MM")
            })?;
        let end = start.checked_add_months(Months::new(1)).unwrap();
        let mut totals = TokenCounts::default();
        let days = self
            .days
            .range(start..end)
            .map(|(&date, &counts)| {
                totals.add(counts);
                (date, counts.into())
            })
            .collect();
        Ok(MonthlyUsage {
            month: month.into(),
            current_month,
            first_month: self
                .days
                .first_key_value()
                .map(|(&date, _)| month_key(date)),
            days,
            totals: totals.into(),
            lifetime: LifetimeCounts {
                input_tokens: self.input_tokens.to_string(),
                output_tokens: self.output_tokens.to_string(),
            },
        })
    }
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
    COUNTS
        .scope(Cell::new(TokenCounts::default()), async {
            let result = provider.clean(raw).await;
            let attempt = CleanupAttempt {
                date: Local::now().date_naive(),
                counts: COUNTS.with(Cell::get),
            };
            (result, Some(attempt))
        })
        .await
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

pub fn load(month: Option<&str>) -> std::io::Result<MonthlyUsage> {
    summary()
        .lock()
        .unwrap()
        .as_ref()
        .map_err(|error| std::io::Error::from(error.kind()))?
        .for_month(month, Local::now().date_naive())
}

pub fn record(attempt: CleanupAttempt) {
    let mut loaded = summary().lock().unwrap();
    // Keep the load failure for this session so saving cannot replace unreadable data.
    if let Ok(summary) = loaded.as_mut() {
        summary.record(attempt.date, attempt.counts);
    }
}

pub fn persist() -> std::io::Result<()> {
    let path = path().ok_or_else(|| std::io::Error::other("Missing home directory"))?;
    static WRITER: Mutex<()> = Mutex::new(());
    let _guard = WRITER.lock().unwrap();
    let snapshot = summary()
        .lock()
        .unwrap()
        .as_ref()
        .cloned()
        .map_err(|error| std::io::Error::from(error.kind()))?;
    write_to(&path, &snapshot)
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
    fn legacy_totals_stay_all_time_and_daily_aggregates_survive_restart() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("usage.json");
        std::fs::write(
            &path,
            br#"{"inputTokens":20,"outputTokens":10,"latest":{"provider":"ollama","model":"llama3","counts":{"inputTokens":20,"outputTokens":10}}}"#,
        )
        .unwrap();
        let mut summary = read_from(&path).unwrap();
        let leap_day = "2024-02-29".parse().unwrap();
        let march = "2024-03-01".parse().unwrap();
        let current = "2025-01-01".parse().unwrap();
        assert!(summary
            .for_month(None, current)
            .unwrap()
            .first_month
            .is_none());
        summary.record(
            leap_day,
            TokenCounts {
                input_tokens: Some(0),
                output_tokens: None,
            },
        );
        summary.record(
            leap_day,
            TokenCounts {
                input_tokens: None,
                output_tokens: None,
            },
        );
        summary.record(march, TokenCounts::default());
        write_to(&path, &summary).unwrap();

        let summary = read_from(&path).unwrap();
        let leap =
            serde_json::to_value(summary.for_month(Some("2024-02"), current).unwrap()).unwrap();
        assert_eq!(leap["firstMonth"], "2024-02");
        assert_eq!(leap["currentMonth"], "2025-01");
        assert_eq!(leap["days"]["2024-02-29"]["inputTokens"], "0");
        assert!(leap["days"]["2024-02-29"]["outputTokens"].is_null());
        assert!(leap["days"].get("2024-03-01").is_none());
        assert_eq!(leap["totals"]["inputTokens"], "0");
        assert_eq!(leap["lifetime"]["inputTokens"], "20");
        assert_eq!(leap["lifetime"]["outputTokens"], "10");
        assert!(leap.get("latest").is_none());
        let march =
            serde_json::to_value(summary.for_month(Some("2024-03"), current).unwrap()).unwrap();
        assert_eq!(
            march["days"]["2024-03-01"],
            serde_json::json!({"inputTokens": null, "outputTokens": null})
        );
        let gap = summary.for_month(Some("2024-12"), current).unwrap();
        assert!(gap.days.is_empty());
        assert!(summary.for_month(None, current).unwrap().days.is_empty());
        for invalid in ["2024-2", "2024-13", "2024-00", "2024-02-29", "abcd-02"] {
            assert!(summary.for_month(Some(invalid), current).is_err());
        }
    }

    #[test]
    fn local_dates_are_fixed_and_large_aggregates_saturate() {
        let mut summary = UsageSummary::default();
        for timestamp in ["2024-12-31T16:30:00-08:00", "2025-01-01T09:30:00+09:00"] {
            let finished = chrono::DateTime::parse_from_rfc3339(timestamp).unwrap();
            let counts = TokenCounts {
                input_tokens: Some(u64::MAX),
                output_tokens: Some(1),
            };
            summary.record(finished.date_naive(), counts);
        }
        let current = "2025-01-02".parse().unwrap();
        summary.record(
            "2025-01-01".parse().unwrap(),
            TokenCounts {
                input_tokens: Some(1),
                output_tokens: Some(u64::MAX),
            },
        );
        summary.record(
            current,
            TokenCounts {
                input_tokens: Some(u64::MAX),
                output_tokens: Some(2),
            },
        );
        let january = serde_json::to_value(summary.for_month(None, current).unwrap()).unwrap();
        assert_eq!(january["month"], "2025-01");
        assert_eq!(
            january["days"]["2025-01-01"]["inputTokens"],
            u64::MAX.to_string()
        );
        assert_eq!(january["totals"]["inputTokens"], u64::MAX.to_string());
        assert_eq!(january["totals"]["outputTokens"], u64::MAX.to_string());
        assert_eq!(january["lifetime"]["outputTokens"], u64::MAX.to_string());
        let december =
            serde_json::to_value(summary.for_month(Some("2024-12"), current).unwrap()).unwrap();
        assert_eq!(december["days"]["2024-12-31"]["outputTokens"], "1");
    }

    #[test]
    fn unreadable_usage_is_reported_and_preserved() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("usage.json");
        assert!(read_from(&path).unwrap().days.is_empty());
        std::fs::write(&path, b"invalid json").unwrap();
        assert!(read_from(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"invalid json");
    }
}

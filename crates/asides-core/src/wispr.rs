//! Wispr Flow's dictation history, read from the app's local `flow.sqlite` with the system's
//! `sqlite3`, read-only. The schema is Wispr Flow's own and changes between its versions, so
//! columns are discovered at run time and the optional ones may be missing.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use chrono::{DateTime, Duration, NaiveDateTime, TimeZone, Utc};
use serde_json::Value;

const SQLITE3: &str = "/usr/bin/sqlite3";
/// The most recent rows looked at each time; far more than anyone dictates between two changes.
const RECENT_ROWS: u32 = 50;
/// A dictation's row is written when it starts and finished later, so rows stay candidates for a while.
const WINDOW: Duration = Duration::minutes(15);
/// Rows with text Wispr Flow delivered: formatted, or raw when formatting was skipped.
const DONE: &[&str] = &["formatted", "raw_transcript"];

/// `~/Library/Application Support/Wispr Flow/flow.sqlite`
pub fn default_db() -> PathBuf {
    std::env::home_dir().expect("HOME is not set").join("Library/Application Support/Wispr Flow/flow.sqlite")
}

#[derive(Debug, Clone, PartialEq)]
pub struct Dictation {
    pub id: String,
    /// When the dictation started.
    pub at: DateTime<Utc>,
    /// The app it was dictated into (a name or a bundle ID), when Wispr Flow recorded one.
    pub app: Option<String>,
    /// What Wispr Flow typed: its formatted text, or the raw transcript when there's none.
    pub text: String,
}

/// New dictations in Wispr Flow's history, oldest first.
pub struct History {
    db: PathBuf,
    since: DateTime<Utc>,
    seen: HashMap<String, DateTime<Utc>>,
    columns: Option<Vec<String>>,
    files: Option<Vec<Option<SystemTime>>>,
}

impl History {
    /// Dictations that started before `since` are never reported.
    pub fn new(db: PathBuf, since: DateTime<Utc>) -> History {
        History { db, since, seen: HashMap::new(), columns: None, files: None }
    }

    /// Dictations finished since the last call (made as its files change). The database is only
    /// queried when they have.
    pub fn read_new(&mut self) -> Result<Vec<Dictation>, String> {
        if !self.db.is_file() {
            return Err(format!(
                "Wispr Flow's history isn't at {}. Is Wispr Flow installed, and does it keep history on this Mac?",
                crate::paths::tilde(&self.db)
            ));
        }
        let files = self.file_times();
        if self.files.as_ref() == Some(&files) {
            return Ok(Vec::new());
        }
        let rows = match self.query() {
            Ok(rows) => rows,
            Err(err) => {
                // Perhaps a Wispr Flow update changed the table: look at its columns again next time.
                self.columns = None;
                return Err(err);
            }
        };
        self.files = Some(files);
        let now = Utc::now();
        let floor = self.since.max(now - WINDOW);
        self.seen.retain(|_, at| *at >= floor);
        let mut found: Vec<Dictation> =
            rows.into_iter().filter(|d| d.at >= floor && !self.seen.contains_key(&d.id)).collect();
        found.sort_by_key(|d| d.at);
        for d in &found {
            self.seen.insert(d.id.clone(), d.at);
        }
        Ok(found)
    }

    fn file_times(&self) -> Vec<Option<SystemTime>> {
        ["", "-wal"]
            .iter()
            .map(|suffix| {
                let mut path = self.db.clone().into_os_string();
                path.push(suffix);
                std::fs::metadata(PathBuf::from(path)).and_then(|m| m.modified()).ok()
            })
            .collect()
    }

    /// Finished dictations among the most recent rows.
    fn query(&mut self) -> Result<Vec<Dictation>, String> {
        if self.columns.is_none() {
            let rows = sqlite(&self.db, "SELECT name FROM pragma_table_info('History')")?;
            let names: Vec<String> = rows.iter().filter_map(|r| r.get("name")?.as_str().map(String::from)).collect();
            self.columns = Some(names);
        }
        let columns = self.columns.as_deref().unwrap_or_default();
        let has = |name: &str| columns.iter().any(|c| c == name);
        let col = |name: &str| if has(name) { format!("\"{name}\"") } else { "NULL".into() };
        if !has("timestamp") || !(has("formattedText") || has("asrText")) {
            self.columns = None;
            return Err("Wispr Flow's history has a format asides doesn't know. Please report it.".into());
        }
        let id = if has("transcriptEntityId") { col("transcriptEntityId") } else { "rowid".into() };
        let sql = format!(
            "SELECT {id} AS id, \"timestamp\" AS ts, {} AS app, {} AS formatted, {} AS asr, {} AS status, \
             {} AS archived FROM History ORDER BY rowid DESC LIMIT {RECENT_ROWS}",
            col("app"),
            col("formattedText"),
            col("asrText"),
            col("status"),
            col("isArchived"),
        );
        let rows = sqlite(&self.db, &sql)?;
        Ok(rows.iter().filter_map(dictation).collect())
    }
}

/// Whether a dictation into `app` should be added, given the configured app filters.
pub fn app_matches(app: Option<&str>, filters: &[String]) -> bool {
    let filters: Vec<String> = filters.iter().map(|f| f.trim().to_lowercase()).filter(|f| !f.is_empty()).collect();
    if filters.is_empty() {
        return true;
    }
    let Some(app) = app.map(str::to_lowercase) else { return false };
    filters.iter().any(|f| app.contains(f.as_str()) || f.contains(app.as_str()))
}

fn dictation(row: &serde_json::Map<String, Value>) -> Option<Dictation> {
    let text = |key: &str| row.get(key).and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty());
    if let Some(status) = text("status")
        && !DONE.contains(&status)
    {
        return None;
    }
    let archived = match row.get("archived") {
        Some(Value::Number(n)) => n.as_i64() == Some(1),
        Some(Value::String(s)) => s == "1" || s == "true",
        Some(Value::Bool(b)) => *b,
        _ => false,
    };
    if archived {
        return None;
    }
    let id = match row.get("id")? {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    Some(Dictation {
        id,
        at: parse_timestamp(row.get("ts")?)?,
        app: text("app").map(String::from),
        text: text("formatted").or_else(|| text("asr"))?.to_string(),
    })
}

/// Wispr Flow stores Sequelize timestamps such as `2026-06-05 07:34:51.566 +00:00`; a few other
/// shapes are accepted in case that changes.
fn parse_timestamp(value: &Value) -> Option<DateTime<Utc>> {
    if let Some(ms) = value.as_i64() {
        return Utc.timestamp_millis_opt(ms).single();
    }
    let s = value.as_str()?.trim();
    for format in ["%Y-%m-%d %H:%M:%S%.f %:z", "%Y-%m-%d %H:%M:%S%.f %z"] {
        if let Ok(at) = DateTime::parse_from_str(s, format) {
            return Some(at.with_timezone(&Utc));
        }
    }
    if let Ok(at) = DateTime::parse_from_rfc3339(s) {
        return Some(at.with_timezone(&Utc));
    }
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f").ok().map(|at| at.and_utc())
}

/// Runs one read-only query; rows come back as JSON objects.
fn sqlite(db: &Path, sql: &str) -> Result<Vec<serde_json::Map<String, Value>>, String> {
    let out = Command::new(SQLITE3)
        .args(["-readonly", "-json", "-cmd", ".timeout 3000"])
        .arg(db)
        .arg(sql)
        .output()
        .map_err(|e| format!("could not run sqlite3: {e}"))?;
    if !out.status.success() {
        return Err(format!("could not read Wispr Flow's history: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(&text).map_err(|e| format!("could not read Wispr Flow's history: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCHEMA: &str = "PRAGMA journal_mode=WAL; CREATE TABLE History (transcriptEntityId UUID PRIMARY KEY, \
        \"timestamp\" DATETIME, app VARCHAR(255), url TEXT, status VARCHAR(255), formattedText TEXT, asrText TEXT, \
        isArchived TINYINT(1) DEFAULT 0, audio BLOB);";

    fn exec(db: &Path, sql: &str) {
        let ok = Command::new(SQLITE3).arg(db).arg(sql).status().unwrap().success();
        assert!(ok, "sqlite3 failed: {sql}");
    }

    fn stamp(at: DateTime<Utc>) -> String {
        at.format("%Y-%m-%d %H:%M:%S%.3f +00:00").to_string()
    }

    fn insert(db: &Path, id: &str, at: DateTime<Utc>, status: &str, formatted: &str, asr: &str) {
        exec(
            db,
            &format!(
                "INSERT INTO History (transcriptEntityId, \"timestamp\", app, status, formattedText, asrText, audio) \
                 VALUES ('{id}', '{}', 'com.apple.Safari', '{status}', '{formatted}', '{asr}', x'00ff');",
                stamp(at)
            ),
        );
    }

    fn texts(found: Vec<Dictation>) -> Vec<String> {
        found.into_iter().map(|d| d.text).collect()
    }

    #[test]
    fn reports_each_finished_dictation_once() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("flow.sqlite");
        exec(&db, SCHEMA);
        let start = Utc::now();
        insert(&db, "old", start - Duration::seconds(30), "formatted", "Before asides watched.", "");
        let mut history = History::new(db.clone(), start);
        assert_eq!(texts(history.read_new().unwrap()), Vec::<String>::new());

        insert(
            &db,
            "a",
            start + Duration::seconds(1),
            "formatted",
            "The card needs padding.",
            "the card needs padding",
        );
        insert(&db, "b", start + Duration::seconds(2), "raw_transcript", "", "raw only");
        insert(&db, "c", start + Duration::seconds(3), "dismissed", "Dismissed.", "");
        insert(&db, "d", start + Duration::seconds(4), "formatted", "  ", "  ");
        insert(&db, "e", start + Duration::seconds(5), "formatted", "Archived.", "");
        exec(&db, "UPDATE History SET isArchived = 1 WHERE transcriptEntityId = 'e'");
        insert(&db, "f", start + Duration::seconds(6), "processing", "", "");
        let found = history.read_new().unwrap();
        assert_eq!(found[0].app.as_deref(), Some("com.apple.Safari"));
        assert_eq!(texts(found), ["The card needs padding.", "raw only"]);
        assert!(history.read_new().unwrap().is_empty());

        // A row finished after it was first seen.
        exec(
            &db,
            "UPDATE History SET status = 'formatted', formattedText = 'It''s done now.' WHERE transcriptEntityId = 'f'",
        );
        assert_eq!(texts(history.read_new().unwrap()), ["It's done now."]);
        assert!(history.read_new().unwrap().is_empty());
    }

    #[test]
    fn copes_with_fewer_columns() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("flow.sqlite");
        exec(&db, "CREATE TABLE History (\"timestamp\" TEXT, asrText TEXT);");
        let start = Utc::now();
        let mut history = History::new(db.clone(), start - Duration::seconds(1));
        exec(&db, &format!("INSERT INTO History VALUES ('{}', 'Just words.')", stamp(start)));
        let found = history.read_new().unwrap();
        assert_eq!((found[0].id.as_str(), found[0].app.as_deref(), found[0].text.as_str()), ("1", None, "Just words."));

        let unknown = tmp.path().join("other.sqlite");
        exec(&unknown, "CREATE TABLE History (\"when\" TEXT, words TEXT);");
        assert!(History::new(unknown, start).read_new().unwrap_err().contains("format"));
        let missing = History::new(tmp.path().join("missing.sqlite"), start).read_new().unwrap_err();
        assert!(missing.contains("Is Wispr Flow installed"), "{missing}");
    }

    #[test]
    fn parses_timestamps() {
        let at = Utc.with_ymd_and_hms(2026, 6, 5, 7, 34, 51).unwrap() + Duration::milliseconds(566);
        for s in ["2026-06-05 07:34:51.566 +00:00", "2026-06-05 10:34:51.566 +03:00", "2026-06-05T07:34:51.566Z"] {
            assert_eq!(parse_timestamp(&Value::from(s)), Some(at), "{s}");
        }
        assert_eq!(parse_timestamp(&Value::from("2026-06-05 07:34:51.566")), Some(at));
        assert_eq!(parse_timestamp(&Value::from(at.timestamp_millis())), Some(at));
        assert_eq!(parse_timestamp(&Value::from("yesterday")), None);
    }

    #[test]
    fn filters_apps() {
        let apps = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(app_matches(Some("Slack"), &[]));
        assert!(app_matches(None, &apps(&[" "])));
        assert!(app_matches(Some("com.apple.Safari"), &apps(&["safari"])));
        assert!(app_matches(Some("Google Chrome"), &apps(&["Safari", "Chrome"])));
        assert!(!app_matches(Some("Slack"), &apps(&["Safari"])));
        assert!(!app_matches(None, &apps(&["Safari"])));
    }
}

use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    learning::{self, Source, Turn},
    provider::LearningMode,
};

pub const MAX_SESSIONS: usize = 100;
pub const MAX_FILE_BYTES: usize = 10 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 80_000;
const MAX_TURNS: usize = 200;
const MAX_DETAIL_BYTES: usize = 20_000;
const MAX_LABEL_BYTES: usize = 200;
const MAX_ACTION_BYTES: usize = 100;
const MAX_SOURCES: usize = 100;
const MAX_SOURCE_TITLE_BYTES: usize = 500;
const MAX_SOURCE_URL_BYTES: usize = 8_192;
const CLOCK_SKEW_SECONDS: u64 = 5 * 60;
const VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub problem: String,
    pub mode: LearningMode,
    pub web_search: bool,
    pub turns: Vec<StoredTurn>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredTurn {
    pub label: String,
    pub action: String,
    pub detail: Option<String>,
    pub content: String,
    pub sources: Vec<StoredSource>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StoredSource {
    pub title: String,
    pub url: String,
}

#[derive(Deserialize, Serialize)]
struct SessionFile {
    version: u32,
    sessions: Vec<Session>,
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn load() -> Result<Vec<Session>, String> {
    load_from_path(&path()?)
}

fn load_from_path(path: &Path) -> Result<Vec<Session>, String> {
    match load_exact(path) {
        Ok(sessions) => Ok(sessions),
        Err(main_error) => {
            let backup = backup_path(path);
            match load_exact(&backup) {
                Ok(sessions) => Ok(sessions),
                Err(_) if !path.exists() && !backup.exists() => Ok(Vec::new()),
                Err(_) => Err(main_error),
            }
        }
    }
}

fn load_exact(path: &Path) -> Result<Vec<Session>, String> {
    if !path.exists() {
        return Err("Session history is missing".into());
    }
    let metadata = fs::metadata(path)
        .map_err(|error| format!("Could not inspect session history: {error}"))?;
    if metadata.len() > MAX_FILE_BYTES as u64 {
        return Err("Session history exceeds the 10 MiB safety limit".into());
    }
    let contents =
        fs::read(path).map_err(|error| format!("Could not read session history: {error}"))?;
    let file: SessionFile = serde_json::from_slice(&contents)
        .map_err(|error| format!("Session history is invalid; the file was preserved: {error}"))?;
    if file.version != VERSION {
        return Err(format!(
            "Session history version {} is unsupported; the file was preserved",
            file.version
        ));
    }
    if file.sessions.len() > MAX_SESSIONS {
        return Err("Session history contains too many sessions; the file was preserved".into());
    }
    let mut ids = std::collections::HashSet::new();
    file.sessions
        .into_iter()
        .map(|session| {
            let session = validate_session(session)?;
            if !ids.insert(session.id.clone()) {
                return Err(
                    "Session history contains duplicate session IDs; the file was preserved".into(),
                );
            }
            Ok(session)
        })
        .collect()
}

pub fn upsert(session: Session) -> Result<(), String> {
    let path = path()?;
    upsert_from_path(&path, session)
}

fn upsert_from_path(path: &Path, session: Session) -> Result<(), String> {
    // A corrupt main file remains read-only even when its backup can be displayed.
    let sessions = if path.exists() {
        load_exact(path)?
    } else {
        load_from_path(path)?
    };
    upsert_at(path, sessions, session)
}

fn upsert_at(path: &Path, mut sessions: Vec<Session>, session: Session) -> Result<(), String> {
    let session = validate_session(session)?;
    sessions.retain(|item| item.id != session.id);
    sessions.push(session);
    sessions.sort_by_key(|item| std::cmp::Reverse(item.updated_at));
    sessions.truncate(MAX_SESSIONS);

    while serialized(&sessions)?.len() > MAX_FILE_BYTES && sessions.len() > 1 {
        sessions.pop();
    }
    let contents = serialized(&sessions)?;
    if contents.len() > MAX_FILE_BYTES {
        return Err("Session is too large to store safely".into());
    }
    atomic_write(path, &contents)
}

fn serialized(sessions: &[Session]) -> Result<Vec<u8>, String> {
    serde_json::to_vec_pretty(&SessionFile {
        version: VERSION,
        sessions: sessions.to_vec(),
    })
    .map_err(|error| format!("Could not serialize session history: {error}"))
}

fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), String> {
    atomic_write_with(path, contents, || Ok(()))
}

fn atomic_write_with(
    path: &Path,
    contents: &[u8],
    before_replace: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Session history path has no parent directory".to_owned())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create the session history directory: {error}"))?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("sessions"),
        uuid::Uuid::new_v4()
    ));
    let backup = backup_path(path);
    let mut file = File::options()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| format!("Could not create temporary session history: {error}"))?;
    let write_result = file.write_all(contents).and_then(|()| file.sync_all());
    drop(file);
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temporary);
        return Err(format!(
            "Could not write temporary session history: {error}"
        ));
    }

    if path.exists() {
        if backup.exists()
            && let Err(error) = fs::remove_file(&backup)
        {
            let _ = fs::remove_file(&temporary);
            return Err(format!("Could not retain session history backup: {error}"));
        }
        if let Err(error) = fs::copy(path, &backup).and_then(|_| {
            let backup_file = File::options().write(true).open(&backup)?;
            backup_file.sync_all()
        }) {
            let _ = fs::remove_file(&temporary);
            return Err(format!("Could not retain session history backup: {error}"));
        }
        if let Err(error) = fs::remove_file(path) {
            let _ = fs::remove_file(&temporary);
            return Err(format!("Could not prepare session history update: {error}"));
        }
    }
    if let Err(error) = before_replace() {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(format!("Could not replace session history: {error}"));
    }
    Ok(())
}

fn backup_path(path: &Path) -> PathBuf {
    path.with_extension("json.bak")
}

pub fn from_app(
    id: &str,
    created_at: u64,
    problem: &str,
    mode: LearningMode,
    web_search: bool,
    turns: &[Turn],
) -> Option<Session> {
    let problem = clean_text(problem, MAX_TEXT_BYTES);
    let turns = turns
        .iter()
        .take(MAX_TURNS)
        .filter_map(stored_turn)
        .collect::<Vec<_>>();
    if problem.trim().is_empty() || turns.is_empty() {
        return None;
    }
    Some(Session {
        id: clean_line(id, 128),
        created_at,
        updated_at: now(),
        problem,
        mode,
        web_search,
        turns,
    })
}

pub fn into_turns(session: &Session) -> Vec<Turn> {
    session
        .turns
        .iter()
        .map(|turn| Turn {
            label: turn.label.clone(),
            detail: turn.detail.clone(),
            content: turn.content.clone(),
            sources: turn
                .sources
                .iter()
                .map(|source| Source {
                    title: source.title.clone(),
                    url: source.url.clone(),
                })
                .collect(),
        })
        .collect()
}

pub fn title(session: &Session) -> String {
    clean_line(&session.problem, 72)
}

fn stored_turn(turn: &Turn) -> Option<StoredTurn> {
    let content = clean_text(&learning::visible_content(&turn.content), MAX_TEXT_BYTES);
    if content.trim().is_empty() {
        return None;
    }
    let label = clean_line(&turn.label, 200);
    Some(StoredTurn {
        action: action_for_label(&label).into(),
        label,
        detail: turn
            .detail
            .as_deref()
            .map(|value| clean_text(value, 20_000)),
        content,
        sources: turn
            .sources
            .iter()
            .take(100)
            .filter_map(|source| safe_source(&source.title, &source.url))
            .collect(),
    })
}

fn validate_session(session: Session) -> Result<Session, String> {
    let now = now();
    if session.id.is_empty()
        || session.id.len() > 128
        || session.id.trim() != session.id
        || !session
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        || session.turns.is_empty()
        || session.turns.len() > MAX_TURNS
        || session.created_at > session.updated_at
        || session.updated_at > now.saturating_add(CLOCK_SKEW_SECONDS)
        || session.problem.len() > MAX_TEXT_BYTES
        || session.problem.trim().is_empty()
        || session.problem != clean_text(&session.problem, MAX_TEXT_BYTES)
    {
        return Err("Session history contains an invalid session".into());
    }
    for turn in &session.turns {
        if turn.label.is_empty()
            || turn.label.len() > MAX_LABEL_BYTES
            || turn.label != clean_line(&turn.label, MAX_LABEL_BYTES)
            || turn.action.is_empty()
            || turn.action.len() > MAX_ACTION_BYTES
            || turn.action != action_for_label(&turn.label)
            || turn.content.len() > MAX_TEXT_BYTES
            || turn.content.trim().is_empty()
            || turn.content != clean_text(&learning::visible_content(&turn.content), MAX_TEXT_BYTES)
            || turn.detail.as_ref().is_some_and(|detail| {
                detail.len() > MAX_DETAIL_BYTES || detail != &clean_text(detail, MAX_DETAIL_BYTES)
            })
            || turn.sources.len() > MAX_SOURCES
            || turn.sources.iter().any(|source| {
                source.title.len() > MAX_SOURCE_TITLE_BYTES
                    || source.title != clean_line(&source.title, MAX_SOURCE_TITLE_BYTES)
                    || source.url.len() > MAX_SOURCE_URL_BYTES
                    || safe_source(&source.title, &source.url).is_none()
            })
        {
            return Err("Session history contains an invalid turn; the file was preserved".into());
        }
    }
    Ok(session)
}

fn safe_source(title: &str, value: &str) -> Option<StoredSource> {
    let url = Url::parse(value).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    Some(StoredSource {
        title: clean_line(title, 500),
        url: url.into(),
    })
}

fn clean_text(value: &str, limit: usize) -> String {
    let clean = learning::strip_controls(value);
    if clean.len() <= limit {
        return clean;
    }
    let mut end = limit;
    while !clean.is_char_boundary(end) {
        end -= 1;
    }
    clean[..end].to_owned()
}

fn clean_line(value: &str, limit: usize) -> String {
    clean_text(value, limit)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn action_for_label(label: &str) -> &'static str {
    match label {
        "Worked example" | "First question" => "initial",
        "Next question" => "socratic_response",
        "Follow-up" => "follow_up",
        "Term explanation" => "explain_term",
        "Guiding question" => "another_hint",
        "Step explanation" => "explain_step",
        "Attempt feedback" => "check_attempt",
        "Target solution" => "reveal_solution",
        _ => "unknown",
    }
}

fn path() -> Result<PathBuf, String> {
    ProjectDirs::from("com", "workedexamples", "desktop")
        .map(|dirs| dirs.config_dir().join("sessions.json"))
        .ok_or_else(|| "Could not locate the session history directory".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn temporary_path() -> PathBuf {
        std::env::temp_dir().join(format!("unfold-sessions-{}.json", Uuid::new_v4()))
    }

    fn remove_history(path: &Path) {
        let _ = fs::remove_file(path);
        let _ = fs::remove_file(backup_path(path));
    }

    fn session(id: &str, updated_at: u64) -> Session {
        Session {
            id: id.into(),
            created_at: 1,
            updated_at,
            problem: format!("problem {id}"),
            mode: LearningMode::Socratic,
            web_search: false,
            turns: vec![StoredTurn {
                label: "First question".into(),
                action: "initial".into(),
                detail: None,
                content: "visible".into(),
                sources: Vec::new(),
            }],
        }
    }

    #[test]
    fn malformed_history_is_preserved_and_not_overwritten() {
        let path = temporary_path();
        fs::write(&path, b"not json").unwrap();
        assert!(load_from_path(&path).unwrap_err().contains("preserved"));
        assert_eq!(fs::read(&path).unwrap(), b"not json");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn upsert_replaces_identity_and_orders_recent_first() {
        let path = temporary_path();
        upsert_at(&path, Vec::new(), session("a", 1)).unwrap();
        let existing = load_from_path(&path).unwrap();
        upsert_at(&path, existing, session("b", 3)).unwrap();
        let existing = load_from_path(&path).unwrap();
        upsert_at(&path, existing, session("a", 4)).unwrap();
        let loaded = load_from_path(&path).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].id, "a");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn persistence_hides_metadata_controls_and_unsafe_sources() {
        let turn = Turn {
            label: "First question\x1b[2J".into(),
            content: "answer<!--SUGGESTIONS secret-->\n<reasoning>hidden</reasoning>".into(),
            detail: Some("learner\x1b[2J".into()),
            sources: vec![
                Source {
                    title: "ok".into(),
                    url: "https://example.test/?query=kept".into(),
                },
                Source {
                    title: "bad".into(),
                    url: "https://user:pass@example.test/".into(),
                },
            ],
        };
        let saved = from_app("id", 1, "problem", LearningMode::Socratic, true, &[turn]).unwrap();
        let json = serde_json::to_string(&saved).unwrap();
        assert!(json.contains("query=kept"));
        assert!(!json.contains("secret"));
        assert!(!json.contains("hidden"));
        assert!(!json.contains("pass"));
        assert!(!json.contains("\\u001b"));
    }

    #[test]
    fn every_known_turn_label_has_a_stable_persisted_action() {
        let cases = [
            ("Worked example", "initial"),
            ("First question", "initial"),
            ("Next question", "socratic_response"),
            ("Follow-up", "follow_up"),
            ("Term explanation", "explain_term"),
            ("Guiding question", "another_hint"),
            ("Step explanation", "explain_step"),
            ("Attempt feedback", "check_attempt"),
            ("Target solution", "reveal_solution"),
            ("Custom label", "unknown"),
        ];

        for (label, expected) in cases {
            assert_eq!(action_for_label(label), expected);
        }
    }

    #[test]
    fn loads_backup_when_main_is_missing_or_corrupt_without_changing_artifacts() {
        let path = temporary_path();
        let backup = backup_path(&path);
        fs::write(&backup, serialized(&[session("backup", 2)]).unwrap()).unwrap();
        assert_eq!(load_from_path(&path).unwrap()[0].id, "backup");

        fs::write(&path, b"corrupt main").unwrap();
        assert_eq!(load_from_path(&path).unwrap()[0].id, "backup");
        assert_eq!(fs::read(&path).unwrap(), b"corrupt main");
        assert!(upsert_from_path(&path, session("new", 3)).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"corrupt main");
        remove_history(&path);
    }

    #[test]
    fn failed_replacement_leaves_the_previous_copy_in_backup() {
        let path = temporary_path();
        let old = serialized(&[session("old", 1)]).unwrap();
        fs::write(&path, &old).unwrap();

        let error = atomic_write_with(&path, b"new", || Err("injected replacement failure".into()))
            .unwrap_err();

        assert!(error.contains("injected"));
        assert!(!path.exists());
        assert_eq!(fs::read(backup_path(&path)).unwrap(), old);
        assert_eq!(load_from_path(&path).unwrap()[0].id, "old");
        remove_history(&path);
    }

    #[test]
    fn invalid_records_are_rejected_instead_of_normalized() {
        let mut cases = Vec::new();
        let mut invalid = session(" bad", 2);
        cases.push(invalid.clone());
        invalid = session("ok", 2);
        invalid.created_at = 3;
        cases.push(invalid.clone());
        invalid = session("ok", now() + CLOCK_SKEW_SECONDS + 1);
        cases.push(invalid.clone());
        invalid = session("ok", 2);
        invalid.turns[0].label = " First question ".into();
        cases.push(invalid.clone());
        invalid = session("ok", 2);
        invalid.turns[0].action = "follow_up".into();
        cases.push(invalid.clone());
        invalid = session("ok", 2);
        invalid.turns.clear();
        cases.push(invalid);

        for value in cases {
            assert!(validate_session(value).is_err());
        }

        let path = temporary_path();
        let duplicate = serialized(&[session("same", 1), session("same", 2)]).unwrap();
        fs::write(&path, duplicate).unwrap();
        assert!(load_exact(&path).unwrap_err().contains("duplicate"));
        remove_history(&path);
    }

    #[test]
    fn upsert_prunes_session_count_and_approximate_file_bytes() {
        let path = temporary_path();
        let many = (0..=MAX_SESSIONS)
            .map(|index| session(&format!("id-{index}"), index as u64 + 1))
            .collect();
        upsert_at(&path, many, session("latest", 500)).unwrap();
        assert_eq!(load_exact(&path).unwrap().len(), MAX_SESSIONS);

        remove_history(&path);
        let mut large = session("large", 600);
        large.turns = (0..75)
            .map(|_| StoredTurn {
                label: "First question".into(),
                action: "initial".into(),
                detail: None,
                content: "x".repeat(MAX_TEXT_BYTES),
                sources: Vec::new(),
            })
            .collect();
        upsert_at(&path, Vec::new(), large.clone()).unwrap();
        large.id = "new-large".into();
        large.updated_at += 1;
        let existing = load_exact(&path).unwrap();
        upsert_at(&path, existing, large).unwrap();
        let bytes = fs::metadata(&path).unwrap().len();
        assert!(bytes <= MAX_FILE_BYTES as u64);
        assert_eq!(load_exact(&path).unwrap().len(), 1);
        remove_history(&path);
    }
}

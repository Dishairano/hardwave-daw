//! Facts a bug report needs, gathered so the dialog can show the user exactly
//! what it is about to send.
//!
//! The report itself is posted from the frontend, which is deliberate: the
//! dialog holds the literal payload, so the text shown next to the "attach my
//! log" checkbox is the text that goes out, not a description of it.

use serde::Serialize;

/// Hard cap on the log tail. The bug API keeps `context` as at most 8000
/// characters of JSON, and a payload over that is cut without warning, so the
/// log is trimmed here with room left for the rest of the object.
const LOG_TAIL_LIMIT: usize = 6000;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BugReportEnv {
    /// App version, for the report's `version` field.
    pub version: String,
    /// Short OS name. The API caps `os` at 20 characters and wants
    /// "Windows 11" rather than a full uname string.
    pub os: String,
    /// True when there is a session log that could be attached.
    pub log_available: bool,
}

#[tauri::command]
pub fn bug_report_env() -> BugReportEnv {
    BugReportEnv {
        version: env!("CARGO_PKG_VERSION").to_string(),
        os: friendly_os_name(),
        log_available: crate::diagnostics::current_session_log().is_some(),
    }
}

/// The tail of this session's log, trimmed to fit the API's context limit.
///
/// The tail rather than the head: what went wrong is what happened last. An
/// empty string when there is no log, so the dialog can say so instead of
/// offering an attachment that does not exist.
#[tauri::command]
pub fn session_log_tail() -> String {
    let Some(path) = crate::diagnostics::current_session_log() else {
        return String::new();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return String::new();
    };
    tail_chars(&text, LOG_TAIL_LIMIT)
}

/// Last `limit` characters, cut at a line boundary so the attachment never
/// starts mid-word, with a marker saying it was shortened.
fn tail_chars(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let skip = text.chars().count() - limit;
    let tail: String = text.chars().skip(skip).collect();
    let from_line_start = match tail.find('\n') {
        Some(i) => &tail[i + 1..],
        None => tail.as_str(),
    };
    format!("[earlier lines trimmed]\n{from_line_start}")
}

/// Short, recognisable OS name within the API's 20-character limit.
fn friendly_os_name() -> String {
    match std::env::consts::OS {
        "windows" => "Windows".to_string(),
        "macos" => "macOS".to_string(),
        "linux" => "Linux".to_string(),
        other => other.chars().take(20).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_log_is_sent_whole() {
        let log = "line one\nline two\n";
        assert_eq!(tail_chars(log, 6000), log);
    }

    #[test]
    fn a_long_log_keeps_its_end_and_says_it_was_trimmed() {
        let log: String = (0..2000).map(|i| format!("line {i}\n")).collect();
        let tail = tail_chars(&log, 200);

        assert!(tail.starts_with("[earlier lines trimmed]\n"));
        assert!(tail.ends_with("line 1999\n"), "the end is what matters");
        // Comfortably inside the API's 8000-character context budget.
        assert!(tail.chars().count() <= 240, "{}", tail.chars().count());
    }

    #[test]
    fn a_trimmed_log_starts_at_a_line_not_mid_word() {
        let log: String = (0..500)
            .map(|i| format!("timestamp {i} something happened\n"))
            .collect();
        let tail = tail_chars(&log, 100);
        let first_real_line = tail.lines().nth(1).unwrap_or("");
        assert!(
            first_real_line.starts_with("timestamp "),
            "cut mid-line: {first_real_line}"
        );
    }

    #[test]
    fn the_os_name_fits_the_api_limit() {
        assert!(friendly_os_name().chars().count() <= 20);
    }
}

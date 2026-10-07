//! The queue behind stem separation.
//!
//! A DAW sends a song; if the account behind it has Pro, the song
//! joins a line and the separator runs on it when its turn comes, one
//! song at a time. The DAW asks how far along it is, and when it is
//! done fetches the four parts: drums, bass, vocals and the rest.
//!
//! One at a time is deliberate. The service shares a small machine
//! with other work, and two separations at once would each take twice
//! as long while using twice the memory. A person waiting sees where
//! they are in the line instead.
//!
//! Jobs live in memory. A restart forgets them and clears their files,
//! and a finished job's parts are kept for a day so a DAW that was
//! closed can still collect them.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use parking_lot::Mutex;
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::sync::Notify;

/// The parts a song comes back as, in the order the model makes them.
pub const STEMS: [&str; 4] = ["drums", "bass", "other", "vocals"];

pub struct Config {
    /// Where accounts are checked.
    pub site: String,
    /// Where songs and their parts are kept while a job lives.
    pub work_dir: PathBuf,
    /// The separator: a program and its first arguments. The song and
    /// the folder for the parts are added after them.
    pub command: Vec<String>,
    /// The largest song accepted, in bytes.
    pub max_upload: usize,
    /// How long a finished job's parts are kept.
    pub keep_for: Duration,
    /// How long one separation may run before it is stopped.
    pub longest_run: Duration,
}

impl Config {
    pub fn from_environment() -> Self {
        let command = std::env::var("STEMS_COMMAND")
            .unwrap_or_else(|_| {
                "/opt/hardwave-stems/venv/bin/python /opt/hardwave-stems/separate.py".into()
            })
            .split_whitespace()
            .map(String::from)
            .collect();
        Config {
            site: hardwave_account::site_from_environment(),
            work_dir: std::env::var("STEMS_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("/var/lib/hardwave-stems")),
            command,
            max_upload: 200 * 1024 * 1024,
            keep_for: Duration::from_secs(24 * 60 * 60),
            longest_run: Duration::from_secs(2 * 60 * 60),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Phase {
    Queued,
    Running { progress: f32 },
    Done,
    Failed { reason: String },
}

struct Job {
    owner: String,
    phase: Phase,
    finished: Option<Instant>,
    cancelled: bool,
}

struct Line {
    config: Config,
    jobs: Mutex<HashMap<String, Job>>,
    waiting: Mutex<VecDeque<String>>,
    wake: Notify,
}

type Shared = Arc<Line>;

/// What a DAW sees of its job.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobView {
    pub id: String,
    /// queued, running, done or failed.
    pub state: &'static str,
    /// Songs ahead of this one, counting the one being separated.
    pub ahead: usize,
    pub progress: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub stems: Vec<&'static str>,
}

/// A request turned away, with the reason a person reads.
struct Refusal {
    status: StatusCode,
    reason: String,
}

impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(serde_json::json!({ "error": self.reason })),
        )
            .into_response()
    }
}

fn refusal(status: StatusCode, reason: impl Into<String>) -> Refusal {
    Refusal {
        status,
        reason: reason.into(),
    }
}

fn refuse(status: StatusCode, reason: impl Into<String>) -> Response {
    refusal(status, reason).into_response()
}

/// Who is asking, if they may use the service at all.
async fn account(line: &Line, headers: &HeaderMap) -> Result<hardwave_account::Who, Refusal> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("")
        .trim();
    if token.is_empty() {
        return Err(refusal(
            StatusCode::UNAUTHORIZED,
            "sign in to separate stems",
        ));
    }
    let who = hardwave_account::identify(&line.config.site, token)
        .await
        .map_err(|reason| refusal(StatusCode::UNAUTHORIZED, reason))?;
    if !who.subscribed {
        return Err(refusal(
            StatusCode::FORBIDDEN,
            "Stem separation is part of Hardwave Pro.",
        ));
    }
    Ok(who)
}

/// How a job looks from outside. `jobs` is the locked job table, so
/// the count of songs ahead is read in the same moment as the job.
fn view(jobs: &HashMap<String, Job>, waiting: &VecDeque<String>, id: &str) -> Option<JobView> {
    let job = jobs.get(id)?;
    let (state, progress, error) = match &job.phase {
        Phase::Queued => ("queued", 0.0, None),
        Phase::Running { progress } => ("running", *progress, None),
        Phase::Done => ("done", 1.0, None),
        Phase::Failed { reason } => ("failed", 0.0, Some(reason.clone())),
    };
    let ahead = if job.phase == Phase::Queued {
        let running = jobs
            .values()
            .filter(|j| matches!(j.phase, Phase::Running { .. }))
            .count();
        waiting.iter().position(|w| w == id).unwrap_or(0) + running
    } else {
        0
    };
    Some(JobView {
        id: id.to_string(),
        state,
        ahead,
        progress,
        error,
        stems: if job.phase == Phase::Done {
            STEMS.to_vec()
        } else {
            Vec::new()
        },
    })
}

fn job_dir(line: &Line, id: &str) -> PathBuf {
    line.config.work_dir.join(id)
}

/// Keep only what can be part of a file name's extension, so a name
/// sent by a client cannot reach outside the job's folder.
fn extension(file_name: &str) -> String {
    let ext: String = std::path::Path::new(file_name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(8)
        .collect::<String>()
        .to_ascii_lowercase();
    if ext.is_empty() {
        "audio".into()
    } else {
        ext
    }
}

/// A song joins the line.
async fn submit(State(line): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    let who = match account(&line, &headers).await {
        Ok(who) => who,
        Err(refused) => return refused.into_response(),
    };
    if body.is_empty() {
        return refuse(StatusCode::BAD_REQUEST, "the song arrived empty");
    }
    {
        let jobs = line.jobs.lock();
        let busy = jobs.values().any(|j| {
            j.owner == who.user_id && matches!(j.phase, Phase::Queued | Phase::Running { .. })
        });
        if busy {
            return refuse(
                StatusCode::CONFLICT,
                "One song at a time: the one you sent is still being separated.",
            );
        }
    }

    let id = uuid::Uuid::new_v4().to_string();
    let file_name = headers
        .get("x-file-name")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("song");
    let dir = job_dir(&line, &id);
    let input = dir.join(format!("input.{}", extension(file_name)));
    let stored = async {
        tokio::fs::create_dir_all(&dir).await?;
        tokio::fs::write(&input, &body).await
    }
    .await;
    if let Err(e) = stored {
        tracing::error!("could not store a song for {id}: {e}");
        return refuse(
            StatusCode::INSUFFICIENT_STORAGE,
            "the server could not store the song",
        );
    }

    let jobs_view = {
        let mut jobs = line.jobs.lock();
        let mut waiting = line.waiting.lock();
        jobs.insert(
            id.clone(),
            Job {
                owner: who.user_id.clone(),
                phase: Phase::Queued,
                finished: None,
                cancelled: false,
            },
        );
        waiting.push_back(id.clone());
        view(&jobs, &waiting, &id)
    };
    line.wake.notify_one();
    tracing::info!("song {id} queued for {}", who.user_id);
    (StatusCode::ACCEPTED, Json(jobs_view)).into_response()
}

/// The job, if it exists and belongs to whoever is asking. Someone
/// else's job answers the same as one that does not exist.
async fn owned(line: &Line, headers: &HeaderMap, id: &str) -> Result<String, Refusal> {
    let who = account(line, headers).await?;
    let jobs = line.jobs.lock();
    match jobs.get(id) {
        Some(job) if job.owner == who.user_id => Ok(who.user_id),
        _ => Err(refusal(StatusCode::NOT_FOUND, "no such job")),
    }
}

async fn status(
    State(line): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(refused) = owned(&line, &headers, &id).await {
        return refused.into_response();
    }
    let jobs = line.jobs.lock();
    let waiting = line.waiting.lock();
    match view(&jobs, &waiting, &id) {
        Some(v) => Json(v).into_response(),
        None => refuse(StatusCode::NOT_FOUND, "no such job"),
    }
}

async fn stem(
    State(line): State<Shared>,
    headers: HeaderMap,
    Path((id, name)): Path<(String, String)>,
) -> Response {
    if let Err(refused) = owned(&line, &headers, &id).await {
        return refused.into_response();
    }
    let Some(name) = STEMS.iter().find(|s| **s == name) else {
        return refuse(StatusCode::NOT_FOUND, "no such part");
    };
    if line.jobs.lock().get(&id).map(|j| j.phase.clone()) != Some(Phase::Done) {
        return refuse(StatusCode::CONFLICT, "not separated yet");
    }
    let path = job_dir(&line, &id).join(format!("{name}.flac"));
    match tokio::fs::File::open(&path).await {
        Ok(file) => (
            [("content-type", "audio/flac")],
            Body::from_stream(tokio_util::io::ReaderStream::new(file)),
        )
            .into_response(),
        Err(_) => refuse(StatusCode::GONE, "the parts are no longer on the server"),
    }
}

/// Take a song out of the line, or stop it if it is running.
async fn cancel(
    State(line): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(refused) = owned(&line, &headers, &id).await {
        return refused.into_response();
    }
    let was_queued = {
        let mut jobs = line.jobs.lock();
        let mut waiting = line.waiting.lock();
        waiting.retain(|w| w != &id);
        match jobs.get_mut(&id) {
            Some(job) if job.phase == Phase::Queued => {
                jobs.remove(&id);
                true
            }
            Some(job) => {
                job.cancelled = true;
                false
            }
            None => false,
        }
    };
    if was_queued {
        let _ = tokio::fs::remove_dir_all(job_dir(&line, &id)).await;
    }
    StatusCode::NO_CONTENT.into_response()
}

/// What the separator said last on its error output, if it is
/// something a person can read. A traceback is logged, not shown.
fn readable_failure(stderr: &str) -> String {
    let last = stderr
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    if last.is_empty() || last.contains("Error") || last.contains("Traceback") || last.len() > 160 {
        "the separation failed on the server".into()
    } else {
        last.to_string()
    }
}

/// Run one job to the end, keeping its progress up to date.
async fn run(line: &Shared, id: &str) -> Phase {
    let dir = job_dir(line, id);
    let input = match std::fs::read_dir(&dir).ok().and_then(|mut d| {
        d.find_map(|e| {
            let p = e.ok()?.path();
            p.file_stem().is_some_and(|s| s == "input").then_some(p)
        })
    }) {
        Some(p) => p,
        None => {
            return Phase::Failed {
                reason: "the song was not on the server any more".into(),
            }
        }
    };
    let Some((program, args)) = line.config.command.split_first() else {
        return Phase::Failed {
            reason: "the server has no separator set up".into(),
        };
    };
    let child = tokio::process::Command::new(program)
        .args(args)
        .arg(&input)
        .arg(&dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("could not start the separator: {e}");
            return Phase::Failed {
                reason: "the separator could not start".into(),
            };
        }
    };
    let mut lines = BufReader::new(child.stdout.take().expect("piped")).lines();
    let mut stderr = child.stderr.take().expect("piped");
    let collect_stderr = tokio::spawn(async move {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text).await;
        text
    });

    let deadline = Instant::now() + line.config.longest_run;
    let mut said_done = false;
    loop {
        if line.jobs.lock().get(id).is_none_or(|j| j.cancelled) {
            let _ = child.kill().await;
            return Phase::Failed {
                reason: "stopped".into(),
            };
        }
        if Instant::now() > deadline {
            let _ = child.kill().await;
            return Phase::Failed {
                reason: "the separation took too long and was stopped".into(),
            };
        }
        match tokio::time::timeout(Duration::from_millis(500), lines.next_line()).await {
            Ok(Ok(Some(text))) => {
                if let Some(value) = text.strip_prefix("progress ") {
                    if let Ok(progress) = value.trim().parse::<f32>() {
                        if let Some(job) = line.jobs.lock().get_mut(id) {
                            job.phase = Phase::Running {
                                progress: progress.clamp(0.0, 1.0),
                            };
                        }
                    }
                } else if text.trim() == "done" {
                    said_done = true;
                }
            }
            Ok(Ok(None)) | Ok(Err(_)) => break,
            Err(_) => continue,
        }
    }
    let exit = child.wait().await;
    let stderr = collect_stderr.await.unwrap_or_default();
    let all_there = STEMS
        .iter()
        .all(|s| dir.join(format!("{s}.flac")).is_file());
    match exit {
        Ok(status) if status.success() && said_done && all_there => {
            let _ = tokio::fs::remove_file(&input).await;
            Phase::Done
        }
        _ => {
            tracing::warn!("separation {id} failed: {stderr}");
            Phase::Failed {
                reason: readable_failure(&stderr),
            }
        }
    }
}

/// Take songs from the line one at a time, for as long as the service runs.
async fn worker(line: Shared) {
    loop {
        let next = {
            let mut jobs = line.jobs.lock();
            let mut waiting = line.waiting.lock();
            let id = waiting.pop_front();
            if let Some(job) = id.as_ref().and_then(|id| jobs.get_mut(id)) {
                job.phase = Phase::Running { progress: 0.0 };
            }
            id
        };
        let Some(id) = next else {
            line.wake.notified().await;
            continue;
        };
        tracing::info!("separating {id}");
        let started = Instant::now();
        let phase = run(&line, &id).await;
        tracing::info!("{id} {:?} after {:?}", phase, started.elapsed());
        let failed = matches!(phase, Phase::Failed { .. });
        if let Some(job) = line.jobs.lock().get_mut(&id) {
            job.phase = phase;
            job.finished = Some(Instant::now());
        }
        if failed {
            let dir = job_dir(&line, &id);
            for s in STEMS {
                let _ = tokio::fs::remove_file(dir.join(format!("{s}.flac"))).await;
            }
        }
    }
}

/// Forget finished jobs once they have been kept long enough.
async fn sweeper(line: Shared) {
    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
        let expired: Vec<String> = {
            let mut jobs = line.jobs.lock();
            let old: Vec<String> = jobs
                .iter()
                .filter(|(_, j)| {
                    j.finished
                        .is_some_and(|f| f.elapsed() > line.config.keep_for)
                })
                .map(|(id, _)| id.clone())
                .collect();
            for id in &old {
                jobs.remove(id);
            }
            old
        };
        for id in expired {
            let _ = tokio::fs::remove_dir_all(job_dir(&line, &id)).await;
        }
    }
}

/// Start the service: the worker, the sweeper, and the routes.
///
/// Anything left in the work folder belongs to jobs a previous run
/// forgot, so it is cleared first. The folder itself stays: on the
/// server it is one systemd made, and may not be ours to remove.
pub fn start(config: Config) -> Router {
    let _ = std::fs::create_dir_all(&config.work_dir);
    if let Ok(entries) = std::fs::read_dir(&config.work_dir) {
        for entry in entries.flatten() {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
    let max_upload = config.max_upload;
    let line: Shared = Arc::new(Line {
        config,
        jobs: Mutex::new(HashMap::new()),
        waiting: Mutex::new(VecDeque::new()),
        wake: Notify::new(),
    });
    tokio::spawn(worker(line.clone()));
    tokio::spawn(sweeper(line.clone()));
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/jobs", axum::routing::post(submit))
        .route("/jobs/{id}", get(status).delete(cancel))
        .route("/jobs/{id}/{stem}", get(stem))
        .layer(DefaultBodyLimit::max(max_upload))
        .with_state(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_name_cannot_walk_out_of_its_folder() {
        assert_eq!(extension("song.MP3"), "mp3");
        assert_eq!(extension("../../etc/passwd"), "audio");
        assert_eq!(extension("x.wav/../../a"), "audio");
        assert_eq!(extension("a.fl/ac"), "audio");
        assert_eq!(extension("weird.w@v"), "wv");
    }

    #[test]
    fn a_traceback_is_not_shown_to_a_person() {
        assert_eq!(
            readable_failure("songs up to 15 minutes can be separated\n"),
            "songs up to 15 minutes can be separated"
        );
        assert_eq!(
            readable_failure("Traceback (most recent call last):\n  File x\nRuntimeError: boom\n"),
            "the separation failed on the server"
        );
        assert_eq!(readable_failure(""), "the separation failed on the server");
    }
}

//! The stems queue over real HTTP, with a stand-in site and a
//! stand-in separator.
//!
//! The separator is a few lines of shell that behave the way the real
//! one does: progress lines, "done", four files. That keeps these tests
//! about the service (who may use it, the line, the hand-back) and
//! quick, while the real separator is measured on the server.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use axum::routing::get;
use axum::{Json, Router};

/// Tokens beginning with `pro-` have Pro; anything else is a free
/// account; "bad" is not a sign-in at all.
async fn fake_site() -> SocketAddr {
    fn token(headers: &axum::http::HeaderMap) -> String {
        headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .trim_start_matches("Bearer ")
            .to_string()
    }
    let app = Router::new()
        .route(
            "/api/auth/me",
            get(|headers: axum::http::HeaderMap| async move {
                let t = token(&headers);
                if t == "bad" {
                    return Err(axum::http::StatusCode::UNAUTHORIZED);
                }
                Ok(Json(serde_json::json!({ "id": t })))
            }),
        )
        .route(
            "/api/subscription",
            get(|headers: axum::http::HeaderMap| async move {
                Json(serde_json::json!({ "hasSubscription": token(&headers).starts_with("pro-") }))
            }),
        );
    serve(app).await
}

async fn serve(app: Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    address
}

/// A separator that writes each part as the text "<part> of <song
/// bytes>", so a test can tell the parts came from the song it sent.
fn fake_separator(dir: &std::path::Path, body: &str) -> Vec<String> {
    let script = dir.join("separate.sh");
    std::fs::write(&script, body).unwrap();
    vec!["sh".into(), script.display().to_string()]
}

const WORKS: &str = r#"
song="$1"; out="$2"
echo "progress 0.5"
sleep 0.2
for p in drums bass other vocals; do printf "%s of %s" "$p" "$(cat "$song")" > "$out/$p.flac"; done
echo "progress 1.0"
echo done
"#;

async fn service(command: Vec<String>, work: PathBuf) -> String {
    let site = fake_site().await;
    let app = hardwave_stems_server::start(hardwave_stems_server::Config {
        site: format!("http://{site}"),
        work_dir: work,
        command,
        max_upload: 1024 * 1024,
        keep_for: Duration::from_secs(3600),
        longest_run: Duration::from_secs(30),
    });
    format!("http://{}", serve(app).await)
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hw-stems-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

async fn send(base: &str, token: &str, song: &'static str) -> reqwest::Response {
    reqwest::Client::new()
        .post(format!("{base}/jobs"))
        .bearer_auth(token)
        .header("x-file-name", "My Song.wav")
        .body(song)
        .send()
        .await
        .unwrap()
}

async fn status(base: &str, token: &str, id: &str) -> serde_json::Value {
    reqwest::Client::new()
        .get(format!("{base}/jobs/{id}"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

async fn until_finished(base: &str, token: &str, id: &str) -> serde_json::Value {
    for _ in 0..100 {
        let s = status(base, token, id).await;
        if s["state"] == "done" || s["state"] == "failed" {
            return s;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the job never finished");
}

#[tokio::test]
async fn only_a_pro_account_can_send_a_song() {
    let dir = scratch("door");
    let base = service(fake_separator(&dir, WORKS), dir.join("work")).await;

    let none = reqwest::Client::new()
        .post(format!("{base}/jobs"))
        .body("x")
        .send()
        .await
        .unwrap();
    assert_eq!(none.status(), 401);
    let bad = send(&base, "bad", "x").await;
    assert_eq!(bad.status(), 401);
    let free = send(&base, "free-anna", "x").await;
    assert_eq!(free.status(), 403);
    let reason: serde_json::Value = free.json().await.unwrap();
    assert_eq!(reason["error"], "Stem separation is part of Hardwave Pro.");
}

#[tokio::test]
async fn a_song_comes_back_as_four_parts_to_the_one_who_sent_it() {
    let dir = scratch("roundtrip");
    let base = service(fake_separator(&dir, WORKS), dir.join("work")).await;

    let sent = send(&base, "pro-anna", "kick and a screech").await;
    assert_eq!(sent.status(), 202);
    let id = sent.json::<serde_json::Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Someone else cannot see it, or even learn it exists.
    let other = reqwest::Client::new()
        .get(format!("{base}/jobs/{id}"))
        .bearer_auth("pro-bert")
        .send()
        .await
        .unwrap();
    assert_eq!(other.status(), 404);

    let done = until_finished(&base, "pro-anna", &id).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(
        done["stems"],
        serde_json::json!(["drums", "bass", "other", "vocals"])
    );
    for part in ["drums", "bass", "other", "vocals"] {
        let body = reqwest::Client::new()
            .get(format!("{base}/jobs/{id}/{part}"))
            .bearer_auth("pro-anna")
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(body, format!("{part} of kick and a screech"));
    }
    let stranger = reqwest::Client::new()
        .get(format!("{base}/jobs/{id}/vocals"))
        .bearer_auth("pro-bert")
        .send()
        .await
        .unwrap();
    assert_eq!(stranger.status(), 404);
    let made_up = reqwest::Client::new()
        .get(format!("{base}/jobs/{id}/..%2Finput.wav"))
        .bearer_auth("pro-anna")
        .send()
        .await
        .unwrap();
    assert_eq!(made_up.status(), 404, "only the four parts can be fetched");
}

#[tokio::test]
async fn the_line_says_who_is_ahead_and_one_song_each() {
    let dir = scratch("line");
    let slow = r#"
song="$1"; out="$2"
sleep 1
for p in drums bass other vocals; do echo x > "$out/$p.flac"; done
echo done
"#;
    let base = service(fake_separator(&dir, slow), dir.join("work")).await;

    let first = send(&base, "pro-anna", "one")
        .await
        .json::<serde_json::Value>()
        .await
        .unwrap();
    let again = send(&base, "pro-anna", "two").await;
    assert_eq!(again.status(), 409, "one song at a time per person");

    tokio::time::sleep(Duration::from_millis(200)).await;
    let second = send(&base, "pro-bert", "three")
        .await
        .json::<serde_json::Value>()
        .await
        .unwrap();
    let id = second["id"].as_str().unwrap();
    let waiting = status(&base, "pro-bert", id).await;
    assert_eq!(waiting["state"], "queued");
    assert_eq!(
        waiting["ahead"], 1,
        "Anna's song is being separated: {waiting}"
    );

    let first_id = first["id"].as_str().unwrap();
    assert_eq!(
        until_finished(&base, "pro-anna", first_id).await["state"],
        "done"
    );
    assert_eq!(until_finished(&base, "pro-bert", id).await["state"], "done");
}

#[tokio::test]
async fn a_failure_says_why_in_words_and_a_crash_does_not_leak() {
    let dir = scratch("fail");
    let too_long = "echo 'songs up to 15 minutes can be separated' >&2; exit 1";
    let base = service(fake_separator(&dir, too_long), dir.join("work")).await;
    let id = send(&base, "pro-anna", "long")
        .await
        .json::<serde_json::Value>()
        .await
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let failed = until_finished(&base, "pro-anna", &id).await;
    assert_eq!(failed["state"], "failed");
    assert_eq!(failed["error"], "songs up to 15 minutes can be separated");

    let dir = scratch("crash");
    let crash = "echo 'Traceback (most recent call last):' >&2; echo 'RuntimeError: /opt/secret/path' >&2; exit 1";
    let base = service(fake_separator(&dir, crash), dir.join("work")).await;
    let id = send(&base, "pro-anna", "x")
        .await
        .json::<serde_json::Value>()
        .await
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let failed = until_finished(&base, "pro-anna", &id).await;
    assert_eq!(failed["error"], "the separation failed on the server");
}

#[tokio::test]
async fn a_song_can_be_taken_out_of_the_line() {
    let dir = scratch("cancel");
    let slow = "sleep 5; echo done";
    let base = service(fake_separator(&dir, slow), dir.join("work")).await;
    let first = send(&base, "pro-anna", "one")
        .await
        .json::<serde_json::Value>()
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let second = send(&base, "pro-bert", "two")
        .await
        .json::<serde_json::Value>()
        .await
        .unwrap();
    let second_id = second["id"].as_str().unwrap();

    let gone = reqwest::Client::new()
        .delete(format!("{base}/jobs/{second_id}"))
        .bearer_auth("pro-bert")
        .send()
        .await
        .unwrap();
    assert_eq!(gone.status(), 204);
    let after = reqwest::Client::new()
        .get(format!("{base}/jobs/{second_id}"))
        .bearer_auth("pro-bert")
        .send()
        .await
        .unwrap();
    assert_eq!(after.status(), 404);

    // Stopping the running one ends it straight away, not after its 5 s.
    let first_id = first["id"].as_str().unwrap();
    let started = std::time::Instant::now();
    reqwest::Client::new()
        .delete(format!("{base}/jobs/{first_id}"))
        .bearer_auth("pro-anna")
        .send()
        .await
        .unwrap();
    let stopped = until_finished(&base, "pro-anna", first_id).await;
    assert_eq!(stopped["error"], "stopped");
    assert!(started.elapsed() < Duration::from_secs(3));
}

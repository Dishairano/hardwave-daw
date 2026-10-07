//! Talking to the stem separation service.
//!
//! The service lives beside the rooms on HQ. The DAW sends a song,
//! asks how it is going every couple of seconds, and fetches the four
//! parts when it is done. Refusals come back as sentences written for
//! a person (sign in, Pro, one song at a time), so they are shown as
//! they are.

use serde::Deserialize;
use std::path::Path;
use std::time::Duration;

/// The parts a song comes back as, in the order the service lists them.
pub const STEMS: [&str; 4] = ["drums", "bass", "other", "vocals"];

pub fn base_url() -> String {
    std::env::var("HARDWAVE_STEMS_URL")
        .unwrap_or_else(|_| "https://rooms.hardwavestudios.com/stems".to_string())
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobView {
    pub id: String,
    /// queued, running, done or failed.
    pub state: String,
    #[serde(default)]
    pub ahead: usize,
    #[serde(default)]
    pub progress: f32,
    #[serde(default)]
    pub error: Option<String>,
}

pub struct Client {
    http: reqwest::Client,
    base: String,
    token: String,
}

/// The service's own sentence when it sent one, otherwise what the
/// status means to a producer.
async fn explain(response: reqwest::Response, during: &str) -> String {
    let status = response.status();
    if let Ok(body) = response.json::<serde_json::Value>().await {
        if let Some(reason) = body.get("error").and_then(|e| e.as_str()) {
            return reason.to_string();
        }
    }
    match status.as_u16() {
        401 => "your Hardwave sign-in has run out; sign in again in any plug-in".into(),
        413 => "that song is too large to send; the limit is 200 MB".into(),
        502..=504 => "the stems service is not answering; try again in a minute".into(),
        _ => format!("the stems service answered {status} while {during}"),
    }
}

impl Client {
    pub fn new(token: String) -> Self {
        Self::with_base(base_url(), token)
    }

    pub fn with_base(base: String, token: String) -> Self {
        Self {
            // No overall timeout: a long song on a slow upload takes as
            // long as it takes. Each call below sets its own.
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(15))
                .build()
                .unwrap_or_default(),
            base,
            token,
        }
    }

    pub async fn submit(&self, file_name: &str, song: Vec<u8>) -> Result<JobView, String> {
        let response = self
            .http
            .post(format!("{}/jobs", self.base))
            .bearer_auth(&self.token)
            .header("x-file-name", file_name)
            .timeout(Duration::from_secs(30 * 60))
            .body(song)
            .send()
            .await
            .map_err(|_| "could not reach the stems service".to_string())?;
        if !response.status().is_success() {
            return Err(explain(response, "sending the song").await);
        }
        response
            .json()
            .await
            .map_err(|_| "the stems service answered something we could not read".into())
    }

    pub async fn status(&self, id: &str) -> Result<JobView, String> {
        let response = self
            .http
            .get(format!("{}/jobs/{id}", self.base))
            .bearer_auth(&self.token)
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(|_| "lost the stems service while waiting".to_string())?;
        if !response.status().is_success() {
            return Err(explain(response, "checking on the song").await);
        }
        response
            .json()
            .await
            .map_err(|_| "the stems service answered something we could not read".into())
    }

    /// Fetch one part into a file, writing as it arrives.
    pub async fn download(&self, id: &str, stem: &str, to: &Path) -> Result<(), String> {
        use futures_util::StreamExt;
        use tokio::io::AsyncWriteExt;
        let response = self
            .http
            .get(format!("{}/jobs/{id}/{stem}", self.base))
            .bearer_auth(&self.token)
            .timeout(Duration::from_secs(30 * 60))
            .send()
            .await
            .map_err(|_| "lost the stems service while bringing the parts down".to_string())?;
        if !response.status().is_success() {
            return Err(explain(response, "bringing the parts down").await);
        }
        let partial = to.with_extension("part");
        let mut file = tokio::fs::File::create(&partial)
            .await
            .map_err(|e| format!("could not write {}: {e}", partial.display()))?;
        let mut body = response.bytes_stream();
        while let Some(chunk) = body.next().await {
            let chunk = chunk.map_err(|_| "the download broke off; try again".to_string())?;
            file.write_all(&chunk)
                .await
                .map_err(|e| format!("could not write {}: {e}", partial.display()))?;
        }
        file.flush().await.map_err(|e| e.to_string())?;
        drop(file);
        tokio::fs::rename(&partial, to)
            .await
            .map_err(|e| format!("could not put {} in place: {e}", to.display()))
    }

    pub async fn cancel(&self, id: &str) {
        let _ = self
            .http
            .delete(format!("{}/jobs/{id}", self.base))
            .bearer_auth(&self.token)
            .timeout(Duration::from_secs(15))
            .send()
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    async fn serve(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{address}")
    }

    /// A service that queues, runs, finishes, and hands back each part
    /// as its own name, the way the real one answers.
    #[tokio::test]
    async fn a_song_goes_up_and_four_parts_come_down() {
        let asked = Arc::new(AtomicUsize::new(0));
        let polls = asked.clone();
        let app = Router::new()
            .route(
                "/jobs",
                post(|headers: axum::http::HeaderMap, body: axum::body::Bytes| async move {
                    assert_eq!(headers["x-file-name"], "Raw Drop.wav");
                    assert_eq!(headers["authorization"], "Bearer pro-token");
                    assert_eq!(&body[..], b"the song");
                    (
                        axum::http::StatusCode::ACCEPTED,
                        Json(serde_json::json!({"id": "j1", "state": "queued", "ahead": 1, "progress": 0.0, "stems": []})),
                    )
                }),
            )
            .route(
                "/jobs/{id}",
                get(move || {
                    let n = polls.fetch_add(1, Ordering::SeqCst);
                    async move {
                        let state = if n == 0 { "running" } else { "done" };
                        Json(serde_json::json!({"id": "j1", "state": state, "ahead": 0, "progress": 0.5, "stems": []}))
                    }
                }),
            )
            .route(
                "/jobs/{id}/{stem}",
                get(|axum::extract::Path((_, stem)): axum::extract::Path<(String, String)>| async move { stem }),
            );
        let client = Client::with_base(serve(app).await, "pro-token".into());

        let job = client
            .submit("Raw Drop.wav", b"the song".to_vec())
            .await
            .unwrap();
        assert_eq!(
            (job.id.as_str(), job.state.as_str(), job.ahead),
            ("j1", "queued", 1)
        );
        assert_eq!(client.status("j1").await.unwrap().state, "running");
        assert_eq!(client.status("j1").await.unwrap().state, "done");

        let dir = std::env::temp_dir().join(format!("hw-stems-client-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for stem in STEMS {
            let to = dir.join(format!("{stem}.flac"));
            client.download("j1", stem, &to).await.unwrap();
            assert_eq!(std::fs::read_to_string(&to).unwrap(), stem);
            assert!(
                !to.with_extension("part").exists(),
                "no half-written file left behind"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn the_service_sentence_reaches_the_person() {
        let app = Router::new().route(
            "/jobs",
            post(|| async {
                (
                    axum::http::StatusCode::FORBIDDEN,
                    Json(serde_json::json!({"error": "Stem separation is part of Hardwave Pro."})),
                )
            }),
        );
        let client = Client::with_base(serve(app).await, "free".into());
        let refused = client.submit("a.wav", vec![1]).await.unwrap_err();
        assert_eq!(refused, "Stem separation is part of Hardwave Pro.");
    }

    #[tokio::test]
    async fn a_proxy_error_is_explained_not_dumped() {
        let app = Router::new().route(
            "/jobs/{id}",
            get(|| async { (axum::http::StatusCode::BAD_GATEWAY, "<html>nginx</html>") }),
        );
        let client = Client::with_base(serve(app).await, "t".into());
        let failed = client.status("j").await.unwrap_err();
        assert_eq!(
            failed,
            "the stems service is not answering; try again in a minute"
        );
    }
}

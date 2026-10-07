//! The stem separation service, as it runs on the server.

use std::net::SocketAddr;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let port: u16 = std::env::var("HARDWAVE_STEMS_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8789);
    // Loopback only: nginx puts TLS and a hostname in front of it.
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .expect("the stems service could not take its port");
    tracing::info!("stems service listening on {address}");
    let app = hardwave_stems_server::start(hardwave_stems_server::Config::from_environment());
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .expect("the stems service stopped badly");
}

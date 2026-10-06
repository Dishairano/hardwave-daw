//! Running the room service.
//!
//! The service itself is in the library beside this, so the rules and
//! the socket can both be tested without a port being taken.

use std::net::SocketAddr;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let port: u16 = std::env::var("HARDWAVE_ROOM_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8787);
    // Loopback only: nginx puts TLS and a hostname in front of it, the
    // same way the plug-in windows are served.
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .expect("the room service could not take its port");
    tracing::info!("room service listening on {address}");
    axum::serve(listener, hardwave_room_server::router())
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .expect("the room service stopped badly");
}

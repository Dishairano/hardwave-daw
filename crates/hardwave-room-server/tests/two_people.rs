//! Two DAWs in one room, over a real socket.
//!
//! The rules have their own tests in `hardwave-room`. What is proved
//! here is the part that only shows up once there is a network: that
//! a host gets a code, that a second person can use it, that what one
//! sends arrives at the other and not back at themselves, and that
//! the account check is the site's answer rather than the client's.

use std::net::SocketAddr;

use axum::routing::get;
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use hardwave_project::multiplayer::{MixerSync, SyncKind, SyncMessage};
use tokio_tungstenite::tungstenite::Message;

/// A stand-in for the site: it says who a token belongs to and
/// whether that account has Pro, which is all the service asks.
async fn fake_site(subscribed: bool) -> SocketAddr {
    let app = Router::new()
        .route(
            "/api/auth/me",
            get(move |headers: axum::http::HeaderMap| async move {
                let token = headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .trim_start_matches("Bearer ")
                    .to_string();
                Json(serde_json::json!({
                    "id": token,
                    "display_name": format!("User {token}"),
                }))
            }),
        )
        .route(
            "/api/subscription",
            get(move || async move { Json(serde_json::json!({ "hasSubscription": subscribed })) }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    address
}

async fn start_service(site: SocketAddr) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            hardwave_room_server::router_for_site(format!("http://{site}")),
        )
        .await
        .unwrap();
    });
    address
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect(service: SocketAddr, query: &str) -> (Socket, serde_json::Value) {
    let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://{service}/room?{query}"))
        .await
        .expect("the service should accept the connection");
    let hello = socket.next().await.expect("a hello").expect("not an error");
    let text = hello.into_text().expect("hello is text");
    (socket, serde_json::from_str(&text).expect("hello is json"))
}

fn an_edit() -> SyncMessage {
    SyncMessage {
        sender_user_id: "whoever".into(),
        logical_clock: 1,
        kind: SyncKind::Mixer(MixerSync {
            track_id: "kick".into(),
            volume_db: Some(-4.0),
            pan: None,
            muted: None,
        }),
    }
}

#[tokio::test]
async fn two_people_hear_each_other_and_not_themselves() {
    let site = fake_site(true).await;
    let service = start_service(site).await;

    let (mut host, hello) = connect(service, "token=host-1").await;
    assert_eq!(hello["type"], "joined");
    let room = hello["room_id"].as_str().unwrap().to_string();
    let code = hello["invite_code"].as_str().unwrap().to_string();

    let (mut guest, joined) =
        connect(service, &format!("token=guest-1&room={room}&code={code}")).await;
    assert_eq!(joined["type"], "joined", "{joined}");

    // The host moves a fader.
    host.send(Message::text(serde_json::to_string(&an_edit()).unwrap()))
        .await
        .unwrap();

    let arrived = tokio::time::timeout(std::time::Duration::from_secs(5), guest.next())
        .await
        .expect("the guest should hear it")
        .expect("a message")
        .expect("not an error");
    let message: SyncMessage = serde_json::from_str(&arrived.into_text().unwrap()).unwrap();
    assert_eq!(
        message.sender_user_id, "host-1",
        "the sender is who the socket says, not who the message claimed"
    );
    assert!(matches!(message.kind, SyncKind::Mixer(_)));

    // And it does not come back to the person who sent it.
    let echo = tokio::time::timeout(std::time::Duration::from_millis(400), host.next()).await;
    assert!(echo.is_err(), "an edit must not echo to its sender");
}

#[tokio::test]
async fn without_pro_the_door_stays_shut() {
    let site = fake_site(false).await;
    let service = start_service(site).await;

    let (_socket, hello) = connect(service, "token=host-1").await;
    assert_eq!(hello["type"], "refused");
    assert!(
        hello["reason"].as_str().unwrap().contains("Hardwave Pro"),
        "it should say why: {hello}"
    );
}

#[tokio::test]
async fn a_wrong_code_does_not_get_in() {
    let site = fake_site(true).await;
    let service = start_service(site).await;

    let (_host, hello) = connect(service, "token=host-1").await;
    let room = hello["room_id"].as_str().unwrap().to_string();

    let (_guest, refused) = connect(
        service,
        &format!("token=guest-1&room={room}&code=NOT-THE-CODE"),
    )
    .await;
    assert_eq!(refused["type"], "refused");
    assert!(refused["reason"].as_str().unwrap().contains("invite code"));
}

#[tokio::test]
async fn a_room_that_was_never_opened_is_said_so() {
    let site = fake_site(true).await;
    let service = start_service(site).await;

    let (_socket, refused) = connect(service, "token=host-1&room=room-nope&code=X").await;
    assert_eq!(refused["type"], "refused");
    assert!(refused["reason"].as_str().unwrap().contains("not open"));
}

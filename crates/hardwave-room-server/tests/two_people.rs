//! Two DAWs in one room, over a real socket.
//!
//! The rules have their own tests in `hardwave-room`. What is proved
//! here is the part that only shows up once there is a network: that
//! a host gets a code, that a second person can use it once the host
//! says yes, that what one sends arrives at the other and not back at
//! themselves, that the account check is the site's answer rather than
//! the client's, and that the service holds up against someone trying
//! to knock it over.

use std::net::SocketAddr;

use axum::routing::get;
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use hardwave_project::multiplayer::{MixerSync, SyncKind, SyncMessage};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

/// A stand-in for the site: it says who a token belongs to and
/// whether that account has Pro, which is all the service asks.
///
/// Tokens beginning with `pro-` have a subscription, so one site can
/// answer for a paying host and a free guest in the same test.
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
            get(move |headers: axum::http::HeaderMap| async move {
                let token = headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .trim_start_matches("Bearer ");
                Json(serde_json::json!({
                    "hasSubscription": subscribed || token.starts_with("pro-"),
                }))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    address
}

async fn start_service(site: SocketAddr) -> SocketAddr {
    start_with(site, hardwave_room_server::Limits::default()).await
}

async fn start_with(site: SocketAddr, limits: hardwave_room_server::Limits) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            hardwave_room_server::router_with(format!("http://{site}"), limits),
        )
        .await
        .unwrap();
    });
    address
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

const WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// Connect with the token in the Authorization header, the way the DAW
/// does, and read the first thing the service says.
async fn connect(
    service: SocketAddr,
    token: Option<&str>,
    query: &str,
) -> (Socket, serde_json::Value) {
    // The way the DAW does it: the code goes in a header, not the address.
    let code = query
        .split('&')
        .find_map(|p| p.strip_prefix("code="))
        .map(str::to_string);
    let query: Vec<&str> = query
        .split('&')
        .filter(|p| !p.starts_with("code="))
        .collect();
    let mut request = format!("ws://{service}/room?{}", query.join("&"))
        .into_client_request()
        .unwrap();
    if let Some(code) = code {
        request
            .headers_mut()
            .insert("x-invite-code", code.parse().unwrap());
    }
    if let Some(token) = token {
        request
            .headers_mut()
            .insert("authorization", format!("Bearer {token}").parse().unwrap());
    }
    let (mut socket, _) = tokio_tungstenite::connect_async(request)
        .await
        .expect("the service should accept the connection");
    let hello = read_json(&mut socket).await.expect("a hello");
    (socket, hello)
}

async fn read_json(socket: &mut Socket) -> Option<serde_json::Value> {
    loop {
        let frame = tokio::time::timeout(WAIT, socket.next())
            .await
            .ok()??
            .ok()?;
        if let Message::Text(text) = frame {
            return serde_json::from_str(&text).ok();
        }
    }
}

/// The next message that is not the room saying who is in it, or
/// nothing within the time given.
async fn next_edit(socket: &mut Socket, within: std::time::Duration) -> Option<SyncMessage> {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        let frame = tokio::time::timeout(left, socket.next())
            .await
            .ok()??
            .ok()?;
        let Message::Text(text) = frame else { continue };
        let message: SyncMessage = serde_json::from_str(&text).ok()?;
        if !matches!(message.kind, SyncKind::MembersChanged { .. }) {
            return Some(message);
        }
    }
}

/// A host with a room open: the socket, the room and its code.
async fn open(service: SocketAddr, token: &str) -> (Socket, String, String) {
    let (host, hello) = connect(service, Some(token), "").await;
    assert_eq!(hello["type"], "joined", "{hello}");
    (
        host,
        hello["room_id"].as_str().unwrap().to_string(),
        hello["invite_code"].as_str().unwrap().to_string(),
    )
}

fn send(kind: SyncKind) -> Message {
    Message::text(
        serde_json::to_string(&SyncMessage {
            sender_user_id: "whoever".into(),
            logical_clock: 1,
            kind,
        })
        .unwrap(),
    )
}

/// A guest knocks; the host answers. Returns the guest's socket and
/// what the service finally told the guest.
async fn knock(
    service: SocketAddr,
    host: &mut Socket,
    token: &str,
    room: &str,
    code: &str,
    admit: bool,
) -> (Socket, serde_json::Value) {
    let (mut guest, first) =
        connect(service, Some(token), &format!("room={room}&code={code}")).await;
    if first["type"] != "waiting" {
        return (guest, first);
    }
    let request = loop {
        let message = next_edit(host, WAIT).await.expect("the host is asked");
        if let SyncKind::JoinRequest { request_id, name } = message.kind {
            assert_eq!(name, format!("User {token}"), "the host sees who wants in");
            break request_id;
        }
    };
    host.send(send(SyncKind::JoinAnswer {
        request_id: request,
        admit,
    }))
    .await
    .unwrap();
    let last = read_json(&mut guest).await.expect("an answer");
    (guest, last)
}

fn an_edit() -> SyncKind {
    SyncKind::Mixer(MixerSync {
        track_id: "kick".into(),
        volume_db: Some(-4.0),
        pan: None,
        muted: None,
    })
}

#[tokio::test]
async fn two_people_hear_each_other_and_not_themselves() {
    let service = start_service(fake_site(true).await).await;
    let (mut host, room, code) = open(service, "host-1").await;
    let (mut guest, joined) = knock(service, &mut host, "guest-1", &room, &code, true).await;
    assert_eq!(joined["type"], "joined", "{joined}");

    host.send(send(an_edit())).await.unwrap();
    let message = next_edit(&mut guest, WAIT)
        .await
        .expect("the guest should hear it");
    assert_eq!(
        message.sender_user_id, "host-1",
        "the sender is who the socket says"
    );
    assert!(matches!(message.kind, SyncKind::Mixer(_)));

    let echo = next_edit(&mut host, std::time::Duration::from_millis(400)).await;
    assert!(
        echo.is_none(),
        "an edit must not echo to its sender: {echo:?}"
    );
}

#[tokio::test]
async fn a_guest_without_pro_can_still_join_a_paid_room() {
    let service = start_service(fake_site(false).await).await;
    let (mut host, room, code) = open(service, "pro-host").await;
    let (_guest, joined) = knock(service, &mut host, "free-guest", &room, &code, true).await;
    assert_eq!(
        joined["type"], "joined",
        "one subscription brings a second producer in: {joined}"
    );
}

#[tokio::test]
async fn opening_a_room_without_pro_is_refused() {
    let service = start_service(fake_site(false).await).await;
    let (_socket, hello) = connect(service, Some("host-1"), "").await;
    assert_eq!(hello["type"], "refused");
    assert!(
        hello["reason"].as_str().unwrap().contains("Hardwave Pro"),
        "{hello}"
    );
}

#[tokio::test]
async fn the_right_code_is_not_enough_without_the_host() {
    let service = start_service(fake_site(true).await).await;
    let (mut host, room, code) = open(service, "host-1").await;
    let (_guest, refused) = knock(service, &mut host, "guest-1", &room, &code, false).await;
    assert_eq!(refused["type"], "refused");
    assert!(
        refused["reason"]
            .as_str()
            .unwrap()
            .contains("did not let you in"),
        "{refused}"
    );
}

#[tokio::test]
async fn a_wrong_code_does_not_get_in() {
    let service = start_service(fake_site(true).await).await;
    let (_host, room, _code) = open(service, "host-1").await;
    let (_guest, refused) = connect(
        service,
        Some("guest-1"),
        &format!("room={room}&code=AAAAAAAAAA"),
    )
    .await;
    assert_eq!(refused["type"], "refused");
    assert!(refused["reason"].as_str().unwrap().contains("invite code"));
}

#[tokio::test]
async fn the_host_hears_when_the_guest_comes_in() {
    let service = start_service(fake_site(true).await).await;
    let (mut host, room, code) = open(service, "host-1").await;
    let (_guest, joined) = knock(service, &mut host, "guest-1", &room, &code, true).await;
    assert_eq!(joined["type"], "joined");
    loop {
        let frame = tokio::time::timeout(WAIT, host.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let message: SyncMessage = serde_json::from_str(&frame.into_text().unwrap()).unwrap();
        if let SyncKind::MembersChanged { names } = message.kind {
            if names.len() == 2 {
                assert_eq!(
                    names,
                    vec!["User host-1".to_string(), "User guest-1".to_string()]
                );
                break;
            }
        }
    }
}

#[tokio::test]
async fn connecting_without_signing_in_is_told_why() {
    let service = start_service(fake_site(true).await).await;
    let (_socket, refused) = connect(service, None, "").await;
    assert_eq!(refused["type"], "refused");
    assert!(
        refused["reason"].as_str().unwrap().contains("sign in"),
        "{refused}"
    );
}

#[tokio::test]
async fn a_token_in_the_address_is_not_accepted() {
    // The address ends up in logs; the token never belongs there.
    let service = start_service(fake_site(true).await).await;
    let (_socket, refused) = connect(service, None, "token=host-1").await;
    assert_eq!(refused["type"], "refused");
}

#[tokio::test]
async fn a_room_that_was_never_opened_is_said_so() {
    let service = start_service(fake_site(true).await).await;
    let (_socket, refused) = connect(service, Some("host-1"), "room=room-nope&code=X").await;
    assert_eq!(refused["type"], "refused");
    assert!(refused["reason"].as_str().unwrap().contains("not open"));
}

#[tokio::test]
async fn a_kicked_guest_is_disconnected_and_the_host_gets_a_new_code() {
    let service = start_service(fake_site(true).await).await;
    let (mut host, room, code) = open(service, "host-1").await;
    let (mut guest, _) = knock(service, &mut host, "guest-1", &room, &code, true).await;

    host.send(send(SyncKind::Kick {
        target_user_id: "guest-1".into(),
    }))
    .await
    .unwrap();

    // The guest's connection ends.
    let ended = tokio::time::timeout(WAIT, async {
        loop {
            match guest.next().await {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                _ => continue,
            }
        }
    })
    .await;
    assert!(ended.is_ok(), "the kicked guest stays connected");

    let new_code = loop {
        let message = next_edit(&mut host, WAIT)
            .await
            .expect("the host hears the new code");
        if let SyncKind::CodeChanged { invite_code } = message.kind {
            break invite_code;
        }
    };
    assert_ne!(new_code, code);
    let (_again, refused) = connect(
        service,
        Some("guest-1"),
        &format!("room={room}&code={code}"),
    )
    .await;
    assert_eq!(
        refused["type"], "refused",
        "the old code does not get them back in"
    );
}

#[tokio::test]
async fn a_song_nobody_asked_for_is_not_passed_on() {
    let service = start_service(fake_site(true).await).await;
    let (mut host, room, code) = open(service, "host-1").await;
    let (mut guest, _) = knock(service, &mut host, "guest-1", &room, &code, true).await;
    guest
        .send(send(SyncKind::ProjectOffer {
            name: "x".into(),
            blob: vec![1, 2, 3],
        }))
        .await
        .unwrap();
    let got = next_edit(&mut host, std::time::Duration::from_millis(500)).await;
    assert!(
        got.is_none(),
        "an unasked-for song reached the host: {got:?}"
    );
}

#[tokio::test]
async fn opening_a_second_room_closes_the_first() {
    let service = start_service(fake_site(true).await).await;
    let (mut first, _, _) = open(service, "host-1").await;
    let (_second, _, _) = open(service, "host-1").await;
    let ended = tokio::time::timeout(WAIT, async {
        loop {
            match first.next().await {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                _ => continue,
            }
        }
    })
    .await;
    assert!(ended.is_ok(), "one room per host");
}

#[tokio::test]
async fn a_quiet_connection_is_closed() {
    let limits = hardwave_room_server::Limits {
        idle: std::time::Duration::from_millis(300),
        ..Default::default()
    };
    let service = start_with(fake_site(true).await, limits).await;
    let (mut host, _, _) = open(service, "host-1").await;
    let ended = tokio::time::timeout(WAIT, async {
        loop {
            match host.next().await {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                _ => continue,
            }
        }
    })
    .await;
    assert!(ended.is_ok(), "a silent connection stays open forever");
}

#[tokio::test]
async fn a_message_too_large_ends_that_connection_only() {
    let limits = hardwave_room_server::Limits {
        max_message: 64 * 1024,
        ..Default::default()
    };
    let service = start_with(fake_site(true).await, limits).await;
    let (mut host, _, _) = open(service, "host-1").await;
    let _ = host.send(Message::text("x".repeat(200 * 1024))).await;
    let ended = tokio::time::timeout(WAIT, async {
        loop {
            match host.next().await {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                _ => continue,
            }
        }
    })
    .await;
    assert!(ended.is_ok());
    // And the service is still there for everyone else.
    let (_other, hello) = connect(service, Some("host-2"), "").await;
    assert_eq!(hello["type"], "joined");
}

#[tokio::test]
async fn past_its_connection_limit_the_service_says_so() {
    let limits = hardwave_room_server::Limits {
        max_sockets: 2,
        ..Default::default()
    };
    let service = start_with(fake_site(true).await, limits).await;
    let _a = open(service, "host-1").await;
    let _b = open(service, "host-2").await;
    let request = format!("ws://{service}/room")
        .into_client_request()
        .unwrap();
    let third = tokio_tungstenite::connect_async(request).await;
    assert!(
        third.is_err(),
        "a third connection was let in past the limit"
    );
}

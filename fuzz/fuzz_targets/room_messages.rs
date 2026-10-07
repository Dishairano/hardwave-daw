#![no_main]
//! Messages a room member sends, through the room's rules.
use hardwave_project::multiplayer::SyncMessage;
use hardwave_room::LiveRoom;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut room = LiveRoom::open("host", 0);
    let code = room.invite_code.clone();
    if let Ok(hardwave_room::Knock::AskHost { request_id }) = room.knock("guest", "Guest", &code, true) {
        let _ = room.answer(&request_id, true);
    }
    for line in data.split(|b| *b == b'\n') {
        if let Ok(mut message) = serde_json::from_slice::<SyncMessage>(line) {
            message.sender_user_id = if line.len() % 2 == 0 { "host".into() } else { "guest".into() };
            let _ = room.handle(&message);
        }
    }
    let _ = room.catch_up(0);
    let _ = room.member_names();
});

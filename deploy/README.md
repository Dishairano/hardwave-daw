# Putting the room service on HQ

The service is one binary with no state. Rooms live in memory and a
room that empties is forgotten, so a restart costs the two people in
a room their connection and nothing else: they press join again.

    cargo build --release -p hardwave-room-server
    install -m755 target/release/hardwave-room-server /usr/local/bin/
    install -m644 deploy/hardwave-room.service /etc/systemd/system/
    systemctl daemon-reload && systemctl enable --now hardwave-room

    # DNS: rooms.hardwavestudios.com -> HQ, then
    certbot --nginx -d rooms.hardwavestudios.com
    install -m644 deploy/hardwave-room.nginx /etc/nginx/sites-available/hardwave-room
    ln -sf /etc/nginx/sites-available/hardwave-room /etc/nginx/sites-enabled/
    nginx -t && systemctl reload nginx

Check it answers:

    curl -s https://rooms.hardwavestudios.com/healthz   # ok

The DAW looks for `wss://rooms.hardwavestudios.com/room` unless
`HARDWAVE_ROOM_URL` says otherwise, which is how a tester points at a
service running somewhere else.

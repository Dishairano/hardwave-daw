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

DNS needs both records, not just the A. The zone has a wildcard that
sends every name's IPv6 at the load balancer, so a client that
prefers IPv6, which most do, lands on the wrong machine and is served
the wrong certificate:

    rooms.hardwavestudios.com  A     178.104.2.34
    rooms.hardwavestudios.com  AAAA  2a01:4f8:1c19:b39d::1

Check it answers:

    curl -s https://rooms.hardwavestudios.com/healthz   # ok

The DAW looks for `wss://rooms.hardwavestudios.com/room` unless
`HARDWAVE_ROOM_URL` says otherwise, which is how a tester points at a
service running somewhere else.

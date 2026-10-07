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

# Putting the stems service on HQ

Stem separation (Pro) runs on HQ beside the rooms, reached at
`https://rooms.hardwavestudios.com/stems/`. Two parts: the queue
(`hardwave-stems-server`, Rust, one song at a time) and the separator
it runs (`services/stems/separate.py`, Demucs htdemucs, MIT).

    python3 -m venv /opt/hardwave-stems/venv
    /opt/hardwave-stems/venv/bin/pip install torch torchaudio --index-url https://download.pytorch.org/whl/cpu
    /opt/hardwave-stems/venv/bin/pip install demucs soundfile
    install -m644 services/stems/separate.py /opt/hardwave-stems/separate.py
    # The model is fetched once, by hand, and never while running:
    HF_HOME=/opt/hardwave-stems/models /opt/hardwave-stems/venv/bin/python -c \
        "from demucs.pretrained import get_model; get_model('htdemucs')"
    chmod -R a+rX /opt/hardwave-stems

    cargo build --release -p hardwave-stems-server
    install -m755 target/release/hardwave-stems-server /usr/local/bin/
    install -m644 deploy/hardwave-stems.service /etc/systemd/system/
    systemctl daemon-reload && systemctl enable --now hardwave-stems
    # The /stems/ route is in deploy/hardwave-room.nginx.

HQ also serves the plug-in windows, so the service is the one that
gives way: lowest CPU and disk priority, 1.5 GB of memory at most.
Measured on HQ: 90 seconds of a song took 4 to 6 minutes while the
machine was busy, with memory flat at 1.25 GB because the separator
works in half-minute chunks. Parts are kept a day, then removed.

Check it answers:

    curl -s https://rooms.hardwavestudios.com/stems/healthz   # ok

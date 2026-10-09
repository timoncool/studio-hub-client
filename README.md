# studio-hub-client

Telemetry and in-app notices for timoncool's desktop studios (YuE2, MiniMax Music3, ACE-Step, Dub Studio and the
next ones), talking to Studio Hub directly or through its RU proxy and keeping the last feed for offline starts.

```rust
let hub = studio_hub_client::Hub::new(HubConfig {
    app: "yue2".into(), version: env!("CARGO_PKG_VERSION").into(), data_dir, http, os_label, gpu, ui_lang, urls: vec![],
})?;
hub.spawn();                          // feed every 6 h, reports a minute after start and every 6 h
let app = app.merge(hub.router());    // /v1/hub/* for the window
hub.count("songs", 1);                // what users do: counts only
```

## What is sent

Only after the start screen has shown its checkbox, and only while it stays checked:

- a random install id (UUID v4) made on this computer, not tied to hardware or an account; Settings can reset it;
- the program, its version, the OS name and version, the window language;
- the graphics card as vendor, a VRAM bucket (≤8, 12, 16, 24+ GB) and the backend (cuda, vulkan, cpu);
- per day: how many times things were done (songs made, covers, failures) and which models were used;
- which notices were shown, clicked or closed.

Never: lyrics, prompts, audio, file names or paths, the IP address (the hub does not store it), anything personal.
`DO_NOT_TRACK=1` or `STUDIO_TELEMETRY=0` turns telemetry off entirely; then no id exists and nothing is counted.
Notices still arrive: the feed is an anonymous GET with no id. `STUDIO_HUB_TEST=1` receives notices marked as test,
`STUDIO_HUB_URL` points the client at another hub (a local `wrangler dev`).

## Routes for the window

| Route | |
|---|---|
| `GET /v1/hub/state` | telemetry choice, feed source, notices that fit now with what the user did with them |
| `POST /v1/hub/notices/{id}/{shown\|clicked\|dismissed}` | remembered always, counted with telemetry on |
| `POST /v1/hub/telemetry` `{enabled, acknowledge}` | the start screen sends `acknowledge: true` |
| `GET /v1/hub/telemetry/preview` | exactly what today's report would carry |
| `POST /v1/hub/telemetry/reset` | a new install id |
| `POST /v1/hub/refresh` | fetch the feed now |
| `GET /v1/hub/media/{key}` | a notice image, cached in the data folder |

The window draws the notices in its own style: bars as a stack across the top (at most three, a colour per place
unless the notice names one), one popup per launch, news in the News list.

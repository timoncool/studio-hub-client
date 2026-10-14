# studio-hub-client

Telemetry and in-app notices for timoncool's desktop studios (YuE2, MiniMax Music3, ACE-Step, Dub Studio and the
next ones), talking to Studio Hub directly or through its RU proxy and keeping the last feed for offline starts.

```rust
let hub = studio_hub_client::Hub::new(HubConfig {
    app: "yue2".into(), version: env!("CARGO_PKG_VERSION").into(), data_dir, http, os_label, gpu, ui_lang, urls: vec![],
})?;
hub.spawn();                          // feed every 6 h, reports a minute after start and every 6 h
let app = hub.track(app.merge(hub.router()));  // /v1/hub/* for the window; every API call counted by route
hub.count("songs", 1);                // results the routes cannot see: songs made
hub.setup_ready();                    // models downloaded (or hub.setup_failed(reason))
hub.worked("song", seconds, work);    // seconds of result in the time the engine had to itself: speed by card
```

## What is sent

Only after the start screen has shown its checkbox, and only while it stays checked:

- a random install id (UUID v4) made on this computer, not tied to hardware or an account; Settings can reset it;
- the program, its version, the OS name and version, the window language;
- the graphics card as vendor, a VRAM bucket (≤8, 12, 16, 24+ GB) and the backend (cuda, vulkan, cpu);
- per day: how many times things were done (songs made, covers, failures) and which models were used;
- per day: which routes of the studio's own local API were called and how many answered with an error, by route
  template (`POST /api/songs/{id}/stems`), never with the ids, the request or the answer: the features people use;
- which notices were shown, clicked or closed.

Never: lyrics, prompts, audio, file names or paths, the IP address, anything personal. The hub keeps the country
Cloudflare reports for a direct connection (none through the proxy), not the address.
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

## Experiments

A notice can be one variant of an A/B test (`experiment: {id, weight}`). The hub sends variants apart from the other
notices (`variants` in the feed), so a client that predates experiments shows none of them. Each install sees one
variant of each experiment, picked by weight from a bucket of a random seed kept on this computer (never sent) and
the experiment id: the same variant at every start, unrelated between experiments. The hub's site embed computes
the same bucket (`feed::bucket`, FNV-1a and the MurmurHash3 finaliser).

The window draws the notices in its own style: bars as a stack across the top (at most three, a colour per place
unless the notice names one), one popup per launch, news in the News list.

# Contract of timoncool's programs

What every program (YuE2 Studio, MiniMax Music3 Studio, ACE-Step Studio, Dub Studio, Dungeon Ultimate, Bulka and
every new one) does the same way. A change to anything here is made in every program in the same release, and this
file changes with it.

## Program ids

The id a program registers with in Studio Hub, in `studios.json` and in "taken from ...":

| id | Name |
|---|---|
| `yue2` | YuE2 Studio |
| `minimax` | MiniMax Music3 Studio |
| `ace` | ACE-Step Studio |
| `dub` | Dub Studio |
| `dungeon` | Dungeon Ultimate |
| `bulka` | Bulka |
| `videosos` | VideoSOS |

A new program gets an id here and in the hub (`hub_app_save`) before its first release.

## Studio Hub: what a program wires in

1. `Hub::new(HubConfig { app, version, data_dir, ... })` at start, then `hub.spawn()` inside the tokio runtime.
   `data_dir` is the program's data folder: the hub keeps `studio-hub.json` there, and `studios.json` points other
   programs to it for models.
2. `hub.track(router.merge(hub.router()))` after the last route of the program's own API and before the MCP tools
   take the router: every call is counted by route template, an agent's calls included. No counter is added by hand
   for a feature that has a route.
3. The day's report leaves when the program closes: `POST /v1/hub/flush` from the desktop shell (`hub.flush(8 s)`).
4. Nothing is counted or sent before the first-run screen showed the statistics checkbox (`acknowledge`).
5. The "What is sent" text of the program lists everything the report carries, in every interface language: counts,
   functions used by name, models, version, OS, graphics card class, failure reasons, the computer number. A new
   field in the report is a new line there.

## Counters

- Finished work: `<kind>_done`, `<kind>_failed` (with `hub.failed(kind, reason)`), `<kind>_cancelled`.
- Models downloaded: `hub.setup_ready()`; a download that failed: `hub.setup_failed(reason)` with a reason in no
  interface language (an error code), so one cause is one row.
- Speed by card: `hub.worked(kind, result_seconds, work)` only for work that had the engine to itself: time spent
  behind another job would read as a slow card. The hub reads `<kind>_media_s` / `<kind>_work_s`.
- Model sets: `hub.used_models(set, components, count)`.

## People: the computer id

`machine::id()` is the same in every program on a computer: an HMAC-SHA256 of the system's machine id (Windows
MachineGuid, Linux machine-id, macOS IOPlatformUUID) over the constant `timoncool-studios/machine/v1`. Every report
carries it; the hub counts people across programs with it. The constant never changes: that would make every computer
a new person. A program that is not built on this crate (a web page) sends none.

## The shared folder

`%LOCALAPPDATA%\timoncool` on Windows, `~/Library/Application Support/timoncool` on macOS,
`$XDG_DATA_HOME/timoncool` elsewhere (`machine::shared_dir`; `TIMONCOOL_SHARED_DIR` replaces it in tests):

- `studios.json`: one line per program (id, version, data folder, last start), written by `Hub::new` under a lock.
- `device-id`: the computer id where the system gives none.

## Models: taken from another program before downloading

- A model file is identified by its file name and its exact size. A program looks for it in the data folders of the
  other programs (`neighbors::current()`) before downloading.
- On the setup screen a file found on the same drive is linked in (a hard link: one file on disk for both programs).
  On install it is taken by copy too when it is on another drive. Only what no program has is downloaded.
- What has a checksum goes through the same check as a download.
- The setup screen says "taken from <program name>" for a model taken in this run.
- A model file is never written in place: downloads and updates go to `.part` and replace the file by rename, so a
  file linked into another program is never changed under it.

## Rollout

The hub's report schema is strict: a report with a field the hub does not know is refused and that day is lost.
A new report field ships in this order, in one go: the hub migration and deploy first, then the programs' releases.

## Reporting problems

Every program has the GitHub issue forms (`bug`, `idea`, `question`) and a "Report a problem" link that opens them
with the version, OS and graphics card filled in from what the program knows. The text fields' `id`s are the query
parameters.

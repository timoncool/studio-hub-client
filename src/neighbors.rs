//! The other programs of timoncool's on this computer, so a studio can take a model another one already downloaded
//! instead of downloading it again. Each program writes its own line (its id, version and data folder) into
//! `studios.json` in the folder they all share ([`crate::machine::shared_dir`]) when it starts; nothing of it is sent
//! anywhere. A portable copy registers the same way, from wherever it was unpacked.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Neighbor {
    /// The program id: yue2, minimax, ace, dub, dungeon...
    pub app: String,
    pub version: String,
    /// Its data folder, where its models are.
    pub data_dir: PathBuf,
    /// Unix seconds of its last start.
    pub seen: i64,
}

#[derive(Default, Deserialize, Serialize)]
struct Registry {
    studios: Vec<Neighbor>,
}

fn registry_file() -> Option<PathBuf> {
    crate::machine::shared_dir().map(|dir| dir.join("studios.json"))
}

fn read(file: &Path) -> Registry {
    match std::fs::read(file) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            tracing::warn!("studio hub: {} is unreadable ({e}), the programs write it anew", file.display());
            Registry::default()
        }),
        Err(_) => Registry::default(),
    }
}

/// Held while one program rewrites the file, so two starting at once do not drop each other's line. A lock older
/// than a few seconds belongs to a program that died holding it and is taken over.
struct Lock(PathBuf);

impl Lock {
    fn take(file: &Path) -> Option<Lock> {
        let path = file.with_extension("lock");
        for _ in 0..100 {
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(_) => return Some(Lock(path)),
                Err(_) => {
                    let stale = std::fs::metadata(&path).and_then(|meta| meta.modified()).is_ok_and(|at| at.elapsed().is_ok_and(|age| age.as_secs() > 5));
                    if stale {
                        let _ = std::fs::remove_file(&path);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
            }
        }
        tracing::warn!("studio hub: {} stayed locked, this program's line was not written", file.display());
        None
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// This program's data folder, once it registered: [`current`] reads the others from anywhere in the program.
static OWN: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// The other programs on this computer, as seen by this one ([`register`] first; none before).
pub fn current() -> Vec<Neighbor> {
    OWN.get().map(|own| others(own)).unwrap_or_default()
}

/// Writes this program's line; the lines of programs whose folder is gone are dropped.
pub fn register(app: &str, version: &str, data_dir: &Path) {
    let _ = OWN.set(data_dir.to_path_buf());
    let Some(file) = registry_file() else { return };
    if let Some(parent) = file.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            tracing::warn!("studio hub: {}: {e}", parent.display());
            return;
        }
    }
    let Some(_held) = Lock::take(&file) else { return };
    let mut registry = read(&file);
    registry.studios.retain(|known| known.data_dir != data_dir && known.data_dir.is_dir());
    registry.studios.push(Neighbor { app: app.into(), version: version.into(), data_dir: data_dir.to_path_buf(), seen: chrono::Utc::now().timestamp() });
    let partial = file.with_extension("tmp");
    let written = std::fs::write(&partial, serde_json::to_vec_pretty(&registry).unwrap_or_default()).and_then(|_| std::fs::rename(&partial, &file));
    if let Err(e) = written {
        tracing::warn!("studio hub: {} was not written: {e}", file.display());
    }
}

/// How deep a neighbour's data folder is searched: models sit a few folders down (`karaoke/models/whisper/...`).
const SEARCH_DEPTH: usize = 6;
/// The neighbours' files are walked at most this often: setup screens ask every second or two.
const WALK_EVERY: std::time::Duration = std::time::Duration::from_secs(60);

fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for path in entries.flatten().map(|entry| entry.path()) {
        if path.is_file() {
            out.push(path);
        } else if depth > 0 && path.is_dir() && !path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with('.')) {
            walk(&path, depth - 1, out);
        }
    }
}

fn neighbor_files() -> Vec<(PathBuf, String)> {
    type Walked = Option<(std::time::Instant, Vec<(PathBuf, String)>)>;
    static WALKED: std::sync::Mutex<Walked> = std::sync::Mutex::new(None);
    let mut walked = WALKED.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((_, files)) = walked.as_ref().filter(|(at, _)| at.elapsed() < WALK_EVERY) {
        return files.clone();
    }
    let mut files = Vec::new();
    for neighbor in current() {
        let mut found = Vec::new();
        walk(&neighbor.data_dir, SEARCH_DEPTH, &mut found);
        files.extend(found.into_iter().map(|path| (path, neighbor.app.clone())));
    }
    *walked = Some((std::time::Instant::now(), files.clone()));
    files
}

/// A model file another program already has: the same file name and exactly this size (a download in progress still
/// carries its `.part` name, so what matches is whole), with that program's id.
pub fn find_file(name: &std::ffi::OsStr, bytes: u64) -> Option<(PathBuf, String)> {
    if bytes == 0 {
        return None;
    }
    neighbor_files().into_iter().find(|(path, _)| path.file_name() == Some(name) && std::fs::metadata(path).is_ok_and(|meta| meta.len() == bytes))
}

/// Puts a neighbour's file at `target`: a hard link (one file on disk for both programs) or, with `copy`, a copy where
/// a link cannot be (another drive). It is staged under `<target>.neighbor`, a name no download writes into, and
/// published by rename only once `accept` passes on it (a checksum, where the program has one); whatever fails, the
/// staged file is removed, so the neighbour's file is never left linked under a name something writes to.
pub fn take_file(source: &Path, target: &Path, copy: bool, accept: impl FnOnce(&Path) -> bool) -> std::io::Result<()> {
    let staged = PathBuf::from(format!("{}.neighbor", target.display()));
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _ = std::fs::remove_file(&staged);
    let placed = match std::fs::hard_link(source, &staged) {
        Ok(()) => Ok(()),
        Err(e) if !copy => Err(e),
        Err(_) => std::fs::copy(source, &staged).map(|_| ()),
    };
    let published = placed.and_then(|()| {
        if accept(&staged) {
            std::fs::rename(&staged, target)
        } else {
            Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{} did not pass the check", source.display())))
        }
    });
    if published.is_err() {
        let _ = std::fs::remove_file(&staged);
    }
    published
}

/// The other programs on this computer whose data folder still exists, the most recently started first.
pub fn others(own_data_dir: &Path) -> Vec<Neighbor> {
    let Some(file) = registry_file() else { return Vec::new() };
    let mut found: Vec<Neighbor> = read(&file).studios.into_iter().filter(|known| known.data_dir != own_data_dir && known.data_dir.is_dir()).collect();
    found.sort_by_key(|known| std::cmp::Reverse(known.seen));
    found
}

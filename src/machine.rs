//! One id per computer, the same in every program of timoncool's, so the hub counts people and not only installs: a
//! person with three studios is one person. It is derived from the system's own machine id (Windows MachineGuid,
//! Linux machine-id, macOS IOPlatformUUID) the way systemd derives an application id from it: an HMAC-SHA256 keyed by
//! that id over a constant of ours, shaped as a UUID. The system's id never leaves the computer and cannot be
//! recovered from ours. Where the system gives no id, a random one is kept in a folder of ours that all the
//! programs share, as Microsoft's DevDeviceId does for its developer tools.

use std::path::PathBuf;

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

/// The same in every program, so their ids match; changing it makes every computer a new one.
const APP_ID: &[u8] = b"timoncool-studios/machine/v1";

/// This computer's id as the programs send it, or none when neither the system id nor the shared folder is available.
pub fn id() -> Option<String> {
    match machine_uid::get() {
        Ok(system) if !system.trim().is_empty() => Some(derive(system.trim())),
        Ok(_) => shared_random(),
        Err(e) => {
            tracing::warn!("studio hub: the system gave no machine id ({e}), the shared one is used");
            shared_random()
        }
    }
}

/// [`id`] where the system gives one, and none otherwise: it writes nothing, so a program may ask before the person
/// chose about statistics (a shell handing the id to its own web page).
pub fn system_id() -> Option<String> {
    machine_uid::get().ok().map(|system| system.trim().to_string()).filter(|system| !system.is_empty()).map(|system| derive(&system))
}

/// HMAC-SHA256 keyed by the system id over [`APP_ID`], its first 16 bytes as a version 4 UUID (RFC 9562), as
/// `sd_id128_get_machine_app_specific` does.
pub(crate) fn derive(system: &str) -> String {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(system.as_bytes()).expect("HMAC takes a key of any length");
    mac.update(APP_ID);
    let digest = mac.finalize().into_bytes();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(bytes).to_string()
}

/// The folder all of timoncool's programs share on this computer: `%LOCALAPPDATA%\timoncool` on Windows,
/// `~/Library/Application Support/timoncool` on macOS, `$XDG_DATA_HOME/timoncool` (or `~/.local/share`) elsewhere.
/// `TIMONCOOL_SHARED_DIR` replaces it (tests, a second profile).
pub fn shared_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("TIMONCOOL_SHARED_DIR") {
        return Some(PathBuf::from(dir));
    }
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library").join("Application Support"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local").join("share")))
    };
    base.map(|base| base.join("timoncool"))
}

fn shared_random() -> Option<String> {
    let file = shared_dir()?.join("device-id");
    if let Ok(text) = std::fs::read_to_string(&file) {
        if let Ok(id) = uuid::Uuid::parse_str(text.trim()) {
            return Some(id.to_string());
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    let written = file.parent().map(std::fs::create_dir_all).transpose().and_then(|_| {
        let partial = file.with_extension("tmp");
        std::fs::write(&partial, &id)?;
        std::fs::rename(&partial, &file)
    });
    match written {
        Ok(()) => Some(id),
        Err(e) => {
            tracing::warn!("studio hub: the shared machine id was not kept in {}: {e}", file.display());
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_system_id_gives_the_same_uuid_and_nothing_of_it_shows() {
        let a = derive("4c4c4544-0042-3510-8051-b4c04f4e3332");
        assert_eq!(a, derive("4c4c4544-0042-3510-8051-b4c04f4e3332"));
        assert_ne!(a, derive("4c4c4544-0042-3510-8051-b4c04f4e3333"));
        let parsed = uuid::Uuid::parse_str(&a).unwrap();
        assert_eq!(parsed.get_version_num(), 4);
        assert!(!a.contains("4c4c4544"));
        assert_eq!(derive("a"), "041bb722-6c3b-4ebe-be3a-7ed023dab1b9");
    }

    #[test]
    fn this_computer_has_an_id() {
        assert!(id().is_some_and(|id| uuid::Uuid::parse_str(&id).is_ok()));
        assert_eq!(system_id(), id(), "where the system gives an id both agree");
    }
}

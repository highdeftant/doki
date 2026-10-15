//! Read-only session-bus metadata. Only ListNames, Get and GetAll are called.
//! A single worker replaces a single snapshot; rendering never waits for D-Bus.

use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::Value;

const EMPTY: &str = "No MPRIS player";
const PREFIX: &str = "org.mpris.MediaPlayer2.";
const OBJECT: &str = "/org/mpris/MediaPlayer2";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";

pub struct MediaPoller {
    snapshot: Arc<Mutex<String>>,
    local: String,
    // Dropping the sender wakes the worker's timed receive. The worker is
    // detached, so dropping this cache never waits for an in-flight request.
    _stop: mpsc::Sender<()>,
}

impl MediaPoller {
    pub fn new() -> Self {
        let snapshot = Arc::new(Mutex::new(EMPTY.to_owned()));
        let (stop, receiver) = mpsc::channel();
        let shared = Arc::clone(&snapshot);
        // Thread creation failure leaves the initial fallback available.
        let _worker = thread::Builder::new()
            .name("mpris-metadata".to_owned())
            .spawn(move || loop {
                let mut disconnected = false;
                let text = poll_with(|args| {
                    if receiver.try_recv() != Err(mpsc::TryRecvError::Empty) {
                        disconnected = true;
                        return None;
                    }
                    busctl(args)
                });
                if disconnected {
                    break;
                }
                if let Ok(mut current) = shared.lock() {
                    *current = text;
                } else {
                    break;
                }
                match receiver.recv_timeout(Duration::from_secs(1)) {
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    _ => break,
                }
            });
        Self {
            snapshot,
            local: EMPTY.to_owned(),
            _stop: stop,
        }
    }

    /// Return the latest one-line snapshot without waiting for the worker.
    pub fn cached(&mut self) -> &str {
        if let Ok(current) = self.snapshot.try_lock() {
            if self.local != *current {
                self.local.clone_from(&current);
            }
        }
        &self.local
    }
}

/// Perform the same read-only poll synchronously, without starting a worker.
pub fn query_once() -> String {
    poll_with(busctl)
}

fn busctl(args: &[&str]) -> Option<Value> {
    let output = Command::new("busctl")
        .args([
            "--user",
            "--json=short",
            "--timeout=0.25",
            "--auto-start=no",
            "--allow-interactive-authorization=no",
            "--no-pager",
        ])
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice(&output.stdout).ok()
}

struct Player {
    name: String,
    identity: String,
    status: String,
    artist: String,
    title: String,
}

impl Player {
    fn rank(&self) -> u8 {
        match self.status.as_str() {
            "Playing" => 0,
            "Paused" => 1,
            _ => 2,
        }
    }

    fn line(&self) -> String {
        let mut line = format!("{} | {}", self.identity, self.status);
        for field in [&self.artist, &self.title] {
            if !field.is_empty() {
                line.push_str(" | ");
                line.push_str(field);
            }
        }
        line
    }
}

fn typed<'a>(value: &'a Value, signature: &str) -> Option<&'a Value> {
    if value.get("type")?.as_str()? != signature {
        return None;
    }
    value.get("data")
}

fn text(value: &Value) -> Option<String> {
    Some(sanitize(typed(value, "s")?.as_str()?))
}

fn sanitize(value: &str) -> String {
    let mut clean = String::with_capacity(value.len());
    let mut space = false;
    for character in value.chars() {
        if character.is_control() || character.is_whitespace() {
            space = !clean.is_empty();
        } else {
            if space {
                clean.push(' ');
                space = false;
            }
            clean.push(character);
        }
    }
    clean
}

fn names(reply: &Value) -> Option<Vec<&str>> {
    let values = typed(reply, "as")?.as_array()?.first()?.as_array()?;
    let mut names = Vec::new();
    for value in values {
        let name = value.as_str()?;
        if name.starts_with(PREFIX) {
            names.push(name);
        }
    }
    names.sort_unstable();
    names.dedup();
    Some(names)
}

fn parse_player(name: &str, identity: &Value, properties: &Value) -> Option<Player> {
    let mut identity = text(typed(identity, "v")?.as_array()?.first()?)?;
    if identity.is_empty() {
        identity = sanitize(name.strip_prefix(PREFIX).unwrap_or(name));
    }
    let properties = typed(properties, "a{sv}")?
        .as_array()?
        .first()?
        .as_object()?;
    let status = text(properties.get("PlaybackStatus")?)?;
    if !matches!(status.as_str(), "Playing" | "Paused" | "Stopped") {
        return None;
    }
    let metadata = properties
        .get("Metadata")
        .and_then(|value| typed(value, "a{sv}"))
        .and_then(Value::as_object);
    let title = metadata
        .and_then(|metadata| metadata.get("xesam:title"))
        .and_then(text)
        .unwrap_or_default();
    let artist = metadata
        .and_then(|metadata| metadata.get("xesam:artist"))
        .and_then(|value| typed(value, "as"))
        .and_then(Value::as_array)
        .map(|values| {
            let mut artists = String::new();
            for value in values.iter().filter_map(Value::as_str) {
                let artist = sanitize(value);
                if artist.is_empty() {
                    continue;
                }
                if !artists.is_empty() {
                    artists.push_str(", ");
                }
                artists.push_str(&artist);
            }
            artists
        })
        .unwrap_or_default();
    Some(Player {
        name: name.to_owned(),
        identity,
        status,
        artist,
        title,
    })
}

fn poll_with(mut query: impl FnMut(&[&str]) -> Option<Value>) -> String {
    let Some(reply) = query(&[
        "call",
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "ListNames",
    ]) else {
        return EMPTY.to_owned();
    };
    let Some(names) = names(&reply) else {
        return EMPTY.to_owned();
    };
    let mut selected: Option<Player> = None;
    for name in names {
        let Some(properties) = query(&["call", name, OBJECT, PROPERTIES, "GetAll", "s", PLAYER])
        else {
            continue;
        };
        let Some(identity) = query(&[
            "call",
            name,
            OBJECT,
            PROPERTIES,
            "Get",
            "ss",
            "org.mpris.MediaPlayer2",
            "Identity",
        ]) else {
            continue;
        };
        let Some(player) = parse_player(name, &identity, &properties) else {
            continue;
        };
        let replace = match selected.as_ref() {
            None => true,
            Some(current) => (player.rank(), &player.name) < (current.rank(), &current.name),
        };
        if replace {
            selected = Some(player);
        }
    }
    selected.map_or_else(|| EMPTY.to_owned(), |player| player.line())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn identity() -> Value {
        json!({"type":"v","data":[{"type":"s","data":"Brave"}]})
    }

    fn properties(status: &str, artists: Value, title: &str) -> Value {
        json!({"type":"a{sv}","data":[{
            "PlaybackStatus":{"type":"s","data":status},
            "Metadata":{"type":"a{sv}","data":{
                "xesam:artist":{"type":"as","data":artists},
                "xesam:title":{"type":"s","data":title}
            }}
        }]})
    }

    #[test]
    fn parses_busctl_wire_json_with_empty_artist() {
        // Shapes observed from busctl --user --json=short Get and GetAll.
        let identity =
            serde_json::from_str::<Value>(r#"{"type":"v","data":[{"type":"s","data":"Brave"}]}"#);
        let properties = serde_json::from_str::<Value>(
            r#"{"type":"a{sv}","data":[{"PlaybackStatus":{"type":"s","data":"Playing"},"Metadata":{"type":"a{sv}","data":{"xesam:artist":{"type":"as","data":[""]},"xesam:title":{"type":"s","data":"MIDNIGHT RUN | YouTube Music"}}}}]}"#,
        );
        match (identity, properties) {
            (Ok(identity), Ok(properties)) => {
                assert_eq!(
                    parse_player("org.mpris.MediaPlayer2.brave", &identity, &properties)
                        .map(|player| player.line()),
                    Some("Brave | Playing | MIDNIGHT RUN | YouTube Music".to_owned())
                );
            }
            _ => panic!("the observed busctl fixtures must be valid JSON"),
        }
    }

    #[test]
    fn busctl_metadata_variants_and_unicode() {
        let player = parse_player(
            "org.mpris.MediaPlayer2.brave",
            &identity(),
            &properties("Playing", json!(["Château Gris", "Guest"]), "YouTube Music"),
        );
        assert_eq!(
            player.map(|p| p.line()),
            Some("Brave | Playing | Château Gris, Guest | YouTube Music".to_owned())
        );
    }

    #[test]
    fn empty_artist_preserves_sanitized_title() {
        let player = parse_player(
            "org.mpris.MediaPlayer2.brave",
            &identity(),
            &properties(
                "Playing",
                json!([""]),
                "MIDNIGHT\nRUN\u{001b} | YouTube Music",
            ),
        );
        assert_eq!(
            player.map(|p| p.line()),
            Some("Brave | Playing | MIDNIGHT RUN | YouTube Music".to_owned())
        );
    }

    fn fake_poll(entries: &[(&str, &str)]) -> String {
        poll_with(|args| match args[4] {
            "ListNames" => Some(
                json!({"type":"as","data":[entries.iter().map(|(name, _)| *name).collect::<Vec<_>>()]}),
            ),
            "Get" => Some(identity()),
            "GetAll" => entries
                .iter()
                .find(|(name, _)| *name == args[1])
                .map(|(name, status)| properties(status, json!([]), name)),
            _ => None,
        })
    }

    #[test]
    fn playing_then_paused_then_stopped_with_name_tie_break() {
        let stopped = ("org.mpris.MediaPlayer2.a", "Stopped");
        let paused = ("org.mpris.MediaPlayer2.b", "Paused");
        let playing = ("org.mpris.MediaPlayer2.z", "Playing");
        assert!(fake_poll(&[stopped, paused, playing]).contains(playing.0));
        assert!(fake_poll(&[stopped, paused]).contains(paused.0));
        assert!(fake_poll(&[stopped]).contains(stopped.0));
        assert!(fake_poll(&[
            ("org.mpris.MediaPlayer2.z", "Playing"),
            ("org.mpris.MediaPlayer2.a", "Playing")
        ])
        .contains("org.mpris.MediaPlayer2.a"));
    }

    #[test]
    fn failure_replaces_playback_instead_of_retaining_stale_text() {
        let (sender, _receiver) = mpsc::channel();
        let snapshot = Arc::new(Mutex::new(fake_poll(&[(
            "org.mpris.MediaPlayer2.a",
            "Playing",
        )])));
        let mut poller = MediaPoller {
            snapshot: Arc::clone(&snapshot),
            local: String::new(),
            _stop: sender,
        };
        assert!(poller.cached().contains("Playing"));
        if let Ok(mut value) = snapshot.lock() {
            *value = poll_with(|_| None);
        }
        assert_eq!(poller.cached(), EMPTY);
        assert_eq!(fake_poll(&[]), EMPTY);
        assert_eq!(
            poll_with(|_| Some(json!({"type":"as","data":"malformed"}))),
            EMPTY
        );
    }

    #[test]
    fn vanished_player_falls_back_to_survivor() {
        let result = poll_with(|args| match args[4] {
            "ListNames" => Some(
                json!({"type":"as","data":[["org.mpris.MediaPlayer2.a", "org.mpris.MediaPlayer2.b"]]}),
            ),
            "GetAll" if args[1].ends_with(".a") => None,
            "GetAll" => Some(properties("Paused", json!([]), "Survivor")),
            "Get" => Some(identity()),
            _ => None,
        });
        assert_eq!(result, "Brave | Paused | Survivor");
    }

    #[test]
    fn malformed_metadata_does_not_hide_valid_status() {
        let mut value = properties("Playing", json!(42), "Title");
        assert_eq!(
            parse_player("org.mpris.MediaPlayer2.a", &identity(), &value).map(|p| p.line()),
            Some("Brave | Playing | Title".to_owned())
        );
        value["data"][0]["Metadata"] = json!({"type":"s", "data":"invalid"});
        assert_eq!(
            parse_player("org.mpris.MediaPlayer2.a", &identity(), &value).map(|p| p.line()),
            Some("Brave | Playing".to_owned())
        );
        value["data"][0]["PlaybackStatus"] = json!({"type":"s", "data":"Invalid"});
        assert!(parse_player("org.mpris.MediaPlayer2.a", &identity(), &value).is_none());
    }

    #[test]
    fn cached_never_waits_for_snapshot_lock() {
        let (sender, _receiver) = mpsc::channel();
        let snapshot = Arc::new(Mutex::new("Updated".to_owned()));
        let mut poller = MediaPoller {
            snapshot: Arc::clone(&snapshot),
            local: "Previous".to_owned(),
            _stop: sender,
        };
        let held = snapshot.lock();
        assert_eq!(poller.cached(), "Previous");
        drop(held);
        assert_eq!(poller.cached(), "Updated");
    }
}

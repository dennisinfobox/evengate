use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::event::Event;
use crate::state::State;

pub const EVENTS_FILE: &str = "events.jsonl";
pub const STATE_FILE: &str = "state.json";

pub fn events_path(task_dir: &Path) -> PathBuf {
    task_dir.join(EVENTS_FILE)
}

/// Read the log. A missing file is an empty log; a malformed line is an error.
pub fn read_events(task_dir: &Path) -> io::Result<Vec<Event>> {
    let path = events_path(task_dir);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut events = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let event = serde_json::from_str(line)
            .map_err(|e| io::Error::other(format!("{EVENTS_FILE}:{}: {e}", index + 1)))?;
        events.push(event);
    }
    Ok(events)
}

/// Append one event and flush it before returning, so a crash leaves a log
/// whose last line is complete.
pub fn append_event(task_dir: &Path, event: &Event) -> io::Result<()> {
    fs::create_dir_all(task_dir)?;
    let line = serde_json::to_string(event)
        .map_err(|e| io::Error::other(format!("cannot encode event: {e}")))?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(events_path(task_dir))?;
    writeln!(file, "{line}")?;
    file.sync_data()
}

/// Stable snapshot bytes. `state.json` is written from these exact bytes both
/// live and during replay, which is what makes the equality check meaningful.
pub fn state_bytes(state: &State) -> io::Result<String> {
    let json = serde_json::to_string_pretty(state)
        .map_err(|e| io::Error::other(format!("cannot encode state: {e}")))?;
    Ok(format!("{json}\n"))
}

pub fn write_state(task_dir: &Path, state: &State) -> io::Result<()> {
    let bytes = state_bytes(state)?;
    let path = task_dir.join(STATE_FILE);
    let temp = task_dir.join(format!("{STATE_FILE}.tmp"));
    fs::create_dir_all(task_dir)?;
    fs::write(&temp, bytes)?;
    fs::rename(&temp, &path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Event;

    #[test]
    fn append_and_read_roundtrip() {
        let dir = std::env::temp_dir().join(format!("evengate-log-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let event = Event::Tick { now_unix: 7 };
        append_event(&dir, &event).unwrap();
        let events = read_events(&dir).unwrap();
        assert_eq!(events, vec![event]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn state_bytes_are_stable() {
        let state = crate::state::initial();
        assert_eq!(state_bytes(&state).unwrap(), state_bytes(&state).unwrap());
        assert!(state_bytes(&state).unwrap().ends_with('\n'));
    }
}

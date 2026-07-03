//! Reading an events directory back into envelopes.

use std::io::{BufRead as _, BufReader};
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use curio_types::EventEnvelope;

use super::{EventLogError, parse_file_name};

/// Reads every envelope under an events directory, in contract order:
/// files sorted by `(date, suffix)` — numerically, so `-10` follows `-2` —
/// then line order within each file. A missing directory reads as empty
/// (consumers must tolerate files vanishing; a whole directory is the
/// degenerate case). Foreign files are ignored; blank lines are skipped.
///
/// This is the test-side reconstruction primitive: pair it with
/// [`super::FoldedState::fold`] to rebuild state from a stream honoring
/// `event_id` dedupe and negation events.
///
/// # Errors
///
/// Filesystem errors, or [`EventLogError::InvalidLine`] for a line that is
/// not a valid `curio.events.v1` envelope.
pub fn read_all(dir: &Path) -> Result<Vec<EventEnvelope>, EventLogError> {
    let mut files: Vec<(NaiveDate, u32, PathBuf)> = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err.into()),
    };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if let Some((date, suffix)) = parse_file_name(name) {
            files.push((date, suffix, entry.path()));
        }
    }
    files.sort();

    let mut events = Vec::new();
    for (_, _, path) in files {
        let reader = BufReader::new(std::fs::File::open(&path)?);
        for (idx, line) in reader.lines().enumerate() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let envelope: EventEnvelope =
                serde_json::from_str(&line).map_err(|err| EventLogError::InvalidLine {
                    path: path.clone(),
                    line: (idx as u64) + 1,
                    message: err.to_string(),
                })?;
            events.push(envelope);
        }
    }
    Ok(events)
}

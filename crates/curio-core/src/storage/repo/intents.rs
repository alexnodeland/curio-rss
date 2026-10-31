//! Staged event intents: the write-ahead rows the events layer drains.

use curio_types::Timestamp;

use super::{PendingIntent, begin_write};
use crate::storage::{Storage, StorageError};

impl Storage {
    /// Staged-but-unemitted event intents, oldest first (emission order).
    ///
    /// A row whose envelope no longer decodes is **quarantined**, not
    /// returned: it is moved, verbatim and with the decode error, to the
    /// `event_intents_dead` table and logged at `warn`. One bad row must
    /// not block every later event — nor the open-time replay, which
    /// would otherwise keep the whole profile from opening. That move is a
    /// write (through the writer thread), so this is a drain step, not a
    /// probe — read-only callers want [`Storage::pending_intent_count`].
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn pending_intents(&self) -> Result<Vec<PendingIntent>, StorageError> {
        let (pending, undecodable) = self.read(|conn| {
            let mut stmt =
                conn.prepare_cached("SELECT id, envelope_json FROM event_intents ORDER BY id ASC")?;
            let raws = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            let mut pending = Vec::with_capacity(raws.len());
            let mut undecodable = Vec::new();
            for (intent_id, json) in raws {
                match serde_json::from_str(&json) {
                    Ok(envelope) => pending.push(PendingIntent {
                        intent_id,
                        envelope,
                    }),
                    Err(err) => undecodable.push((intent_id, err.to_string())),
                }
            }
            Ok((pending, undecodable))
        })?;
        if !undecodable.is_empty() {
            self.quarantine_intents(undecodable)?;
        }
        Ok(pending)
    }

    /// How many intent rows are staged, decodable or not. A pure read —
    /// unlike [`Storage::pending_intents`] it never quarantines, so a
    /// diagnostic probe can call it without writing to the database.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn pending_intent_count(&self) -> Result<u64, StorageError> {
        self.read(|conn| {
            let n: i64 =
                conn.query_row("SELECT COUNT(*) FROM event_intents", [], |row| row.get(0))?;
            Ok(u64::try_from(n).unwrap_or_default())
        })
    }

    /// How many intents have been quarantined into `event_intents_dead`
    /// (a doctor-surface signal: non-zero means events were lost from the
    /// log and the rows are kept for diagnosis).
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn quarantined_intent_count(&self) -> Result<u64, StorageError> {
        self.read(|conn| {
            let n: i64 = conn.query_row("SELECT COUNT(*) FROM event_intents_dead", [], |row| {
                row.get(0)
            })?;
            Ok(u64::try_from(n).unwrap_or_default())
        })
    }

    /// Moves undecodable intent rows to the dead-letter table.
    fn quarantine_intents(&self, rows: Vec<(i64, String)>) -> Result<(), StorageError> {
        self.write(move |conn| {
            let tx = begin_write(conn)?;
            let now = Timestamp::now().to_string();
            for (id, error) in rows {
                tracing::warn!(
                    intent_id = id,
                    %error,
                    "quarantined an undecodable event intent into event_intents_dead"
                );
                tx.prepare_cached(
                    "INSERT INTO event_intents_dead \
                     (intent_id, event_id, ts, envelope_json, error, quarantined_at) \
                     SELECT id, event_id, ts, envelope_json, ?2, ?3 \
                     FROM event_intents WHERE id = ?1",
                )?
                .execute((id, &error, &now))?;
                tx.prepare_cached("DELETE FROM event_intents WHERE id = ?1")?
                    .execute([id])?;
            }
            tx.commit()?;
            Ok(())
        })
    }

    /// Deletes intents whose envelopes have been durably appended to the
    /// JSONL log.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn mark_intents_emitted(&self, intent_ids: Vec<i64>) -> Result<(), StorageError> {
        self.write(move |conn| {
            let tx = begin_write(conn)?;
            for id in intent_ids {
                tx.prepare_cached("DELETE FROM event_intents WHERE id = ?1")?
                    .execute([id])?;
            }
            tx.commit()?;
            Ok(())
        })
    }
}

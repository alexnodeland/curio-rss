//! Staged event intents: the write-ahead rows the events layer drains.

use super::{PendingIntent, begin_write};
use crate::storage::{Storage, StorageError};

impl Storage {
    /// Staged-but-unemitted event intents, oldest first (emission order).
    ///
    /// # Errors
    ///
    /// Database or envelope-deserialization errors.
    pub fn pending_intents(&self) -> Result<Vec<PendingIntent>, StorageError> {
        self.read(|conn| {
            let mut stmt =
                conn.prepare_cached("SELECT id, envelope_json FROM event_intents ORDER BY id ASC")?;
            let raws = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            raws.into_iter()
                .map(|(intent_id, json)| {
                    Ok(PendingIntent {
                        intent_id,
                        envelope: serde_json::from_str(&json)?,
                    })
                })
                .collect()
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

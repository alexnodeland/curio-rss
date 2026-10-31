//! The key/value settings table.

use crate::storage::{Storage, StorageError};
use rusqlite::OptionalExtension as _;

impl Storage {
    /// Reads a settings value.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn get_setting(&self, key: &str) -> Result<Option<String>, StorageError> {
        self.read(move |conn| {
            let value = conn
                .prepare_cached("SELECT value FROM settings WHERE key = ?1")?
                .query_row([key], |row| row.get(0))
                .optional()?;
            Ok(value)
        })
    }

    /// Writes (upserts) a settings value.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), StorageError> {
        let (key, value) = (key.to_owned(), value.to_owned());
        self.write(move |conn| {
            conn.prepare_cached(
                "INSERT INTO settings (key, value) VALUES (?1, ?2) \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            )?
            .execute((&key, &value))?;
            Ok(())
        })
    }
}

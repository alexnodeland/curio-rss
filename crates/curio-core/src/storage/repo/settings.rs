//! The key/value settings table.

use crate::storage::{Storage, StorageError};
use rusqlite::{OptionalExtension as _, Transaction};

use super::begin_write;

const SELECT_VALUE: &str = "SELECT value FROM settings WHERE key = ?1";
const UPSERT_VALUE: &str = "INSERT INTO settings (key, value) VALUES (?1, ?2) \
                            ON CONFLICT(key) DO UPDATE SET value = excluded.value";

impl Storage {
    /// Reads a settings value.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn get_setting(&self, key: &str) -> Result<Option<String>, StorageError> {
        self.read(move |conn| {
            let value = conn
                .prepare_cached(SELECT_VALUE)?
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
            conn.prepare_cached(UPSERT_VALUE)?.execute((&key, &value))?;
            Ok(())
        })
    }

    /// Read-modify-write of settings values as one atomic step, across
    /// processes too: `edit` runs inside an `IMMEDIATE` transaction on the
    /// writer thread, so every value it reads is the latest committed one,
    /// and nobody (no other thread, no other process on the profile) can
    /// write the database between its reads and its writes. The
    /// transaction commits when `edit` returns `Ok` and rolls back,
    /// writing nothing, when it returns `Err`.
    ///
    /// `edit` runs on the writer thread, so it must stay short: this
    /// process's other writes queue behind it.
    ///
    /// # Errors
    ///
    /// Whatever `edit` returns; database errors (converted into `E`).
    pub(crate) fn edit_settings<T, E, F>(&self, edit: F) -> Result<T, E>
    where
        T: Send + 'static,
        E: From<StorageError> + Send + 'static,
        F: FnOnce(&SettingsTx<'_>) -> Result<T, E> + Send + 'static,
    {
        self.write(move |conn| {
            let tx = begin_write(conn)?;
            let outcome = edit(&SettingsTx { tx: &tx });
            if outcome.is_ok() {
                tx.commit()?;
            }
            // On `Err` the transaction is dropped here, which rolls it back.
            Ok(outcome)
        })?
    }
}

/// The settings table inside one [`Storage::edit_settings`] transaction.
pub(crate) struct SettingsTx<'a> {
    tx: &'a Transaction<'a>,
}

impl SettingsTx<'_> {
    /// Reads a settings value as of this transaction, its own writes
    /// included.
    pub(crate) fn get(&self, key: &str) -> Result<Option<String>, StorageError> {
        Ok(self
            .tx
            .prepare_cached(SELECT_VALUE)?
            .query_row([key], |row| row.get(0))
            .optional()?)
    }

    /// Writes (upserts) a settings value. Other connections see it once
    /// the transaction commits.
    pub(crate) fn set(&self, key: &str, value: &str) -> Result<(), StorageError> {
        self.tx
            .prepare_cached(UPSERT_VALUE)?
            .execute((key, value))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn storage() -> (tempfile::TempDir, Storage) {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(dir.path().join("curio.db")).unwrap();
        (dir, storage)
    }

    #[test]
    fn an_edit_reads_the_stored_values_and_commits_its_writes() {
        let (_dir, storage) = storage();
        storage.set_setting("a", "1").unwrap();
        let (before, inside) = storage
            .edit_settings(|settings| {
                let before = settings.get("a")?;
                settings.set("a", "2")?;
                settings.set("b", "new")?;
                Ok::<_, StorageError>((before, settings.get("a")?))
            })
            .unwrap();
        assert_eq!(before.as_deref(), Some("1"));
        assert_eq!(inside.as_deref(), Some("2"), "its own write is visible");
        assert_eq!(storage.get_setting("a").unwrap().as_deref(), Some("2"));
        assert_eq!(storage.get_setting("b").unwrap().as_deref(), Some("new"));
    }

    #[test]
    fn a_failed_edit_writes_nothing() {
        let (_dir, storage) = storage();
        storage.set_setting("a", "1").unwrap();
        let result: Result<(), StorageError> = storage.edit_settings(|settings| {
            settings.set("a", "2")?;
            Err(StorageError::WriterGone)
        });
        assert!(matches!(result, Err(StorageError::WriterGone)));
        assert_eq!(storage.get_setting("a").unwrap().as_deref(), Some("1"));
    }

    /// Two storages on one database stand in for two processes:
    /// interleaved read-modify-write edits never lose an update.
    #[test]
    fn concurrent_edits_from_two_connections_never_lose_an_update() {
        let (dir, first) = storage();
        let second = Storage::open(dir.path().join("curio.db")).unwrap();
        std::thread::scope(|scope| {
            for storage in [&first, &second] {
                scope.spawn(move || {
                    for _ in 0..50 {
                        storage
                            .edit_settings(|settings| {
                                let n: u32 =
                                    settings.get("n")?.map_or(0, |raw| raw.parse().unwrap());
                                settings.set("n", &(n + 1).to_string())
                            })
                            .unwrap();
                    }
                });
            }
        });
        assert_eq!(first.get_setting("n").unwrap().as_deref(), Some("100"));
    }
}

//! The key/value settings store heads keep their preferences in.

use super::destinations::{DESTINATIONS_KEY, EXPORTED_DESTINATIONS_KEY, SEEDED_DESTINATIONS_KEY};
use super::{CoreError, CoreHandle};

impl CoreHandle {
    /// Reads a settings value (`None` when the key was never written).
    ///
    /// Heads keep their own preferences here under namespaced keys
    /// (`ui.*`, `refresh.*`, `notify.*`, …).
    ///
    /// # Errors
    ///
    /// [`CoreError::ReservedSettingKey`] for the facade-owned
    /// `"destinations"`, `"destinations.exported"` and
    /// `"destinations.seeded"` keys (raw paths never cross the facade —
    /// read the registry with [`Self::destinations`]); storage errors.
    pub fn get_setting(&self, key: &str) -> Result<Option<String>, CoreError> {
        reject_reserved(key)?;
        Ok(self.storage.get_setting(key)?)
    }

    /// Writes (upserts) a settings value.
    ///
    /// # Errors
    ///
    /// [`CoreError::ReservedSettingKey`] for the facade-owned
    /// `"destinations"`, `"destinations.exported"` and
    /// `"destinations.seeded"` keys — the destination registry changes
    /// only through [`Self::add_destination`] /
    /// [`Self::remove_destination`] /
    /// [`Self::reconcile_destinations`] (and exports); storage errors.
    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), CoreError> {
        reject_reserved(key)?;
        Ok(self.storage.set_setting(key, value)?)
    }
}

/// Refuses the settings keys the facade owns, so no head can read or
/// clobber them as raw values.
fn reject_reserved(key: &str) -> Result<(), CoreError> {
    if [
        DESTINATIONS_KEY,
        EXPORTED_DESTINATIONS_KEY,
        SEEDED_DESTINATIONS_KEY,
    ]
    .contains(&key)
    {
        return Err(CoreError::ReservedSettingKey {
            key: key.to_owned(),
        });
    }
    Ok(())
}

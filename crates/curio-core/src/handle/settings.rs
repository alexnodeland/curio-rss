//! The key/value settings store heads keep their preferences in.

use super::{CoreError, CoreHandle};

impl CoreHandle {
    /// Reads a settings value (`None` when the key was never written).
    ///
    /// Heads keep their own preferences here under namespaced keys
    /// (`ui.*`, `refresh.*`, `notify.*`, …).
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn get_setting(&self, key: &str) -> Result<Option<String>, CoreError> {
        Ok(self.storage.get_setting(key)?)
    }

    /// Writes (upserts) a settings value.
    ///
    /// The `"destinations"` key holds the facade-owned destination
    /// registry; heads change that only through
    /// [`Self::add_destination`] / [`Self::remove_destination`] and
    /// refuse it as a raw key.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), CoreError> {
        Ok(self.storage.set_setting(key, value)?)
    }
}

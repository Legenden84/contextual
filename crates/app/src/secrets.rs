//! The API key's home: the OS keychain (plan §9). When no keychain is available (e.g. WSL without
//! a Secret Service), the key is kept in memory for this session and the Settings screen says so.

use contextual_core::settings::{MemorySecretStore, SecretStore};
use contextual_core::{Error, Result};

const SERVICE: &str = "contextual";

pub struct AppSecrets {
    keychain: bool,
    memory: MemorySecretStore,
}

impl AppSecrets {
    pub fn new() -> Self {
        let keychain = keyring::Entry::store_status().is_ok();
        if !keychain {
            eprintln!(
                "contextual: no OS keychain available; the API key is kept for this session only"
            );
        }
        Self {
            keychain,
            memory: MemorySecretStore::new(),
        }
    }

    /// Whether the key survives a restart.
    pub fn persistent(&self) -> bool {
        self.keychain
    }

    fn entry(name: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(SERVICE, name).map_err(|e| Error::internal(format!("Keychain: {e}")))
    }
}

impl SecretStore for AppSecrets {
    fn get(&self, name: &str) -> Result<Option<String>> {
        if !self.keychain {
            return self.memory.get(name);
        }
        match Self::entry(name)?.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(Error::internal(format!("Keychain: {e}"))),
        }
    }

    fn set(&self, name: &str, value: &str) -> Result<()> {
        if !self.keychain {
            return self.memory.set(name, value);
        }
        Self::entry(name)?
            .set_password(value)
            .map_err(|e| Error::internal(format!("Keychain: {e}")))
    }

    fn delete(&self, name: &str) -> Result<()> {
        if !self.keychain {
            return self.memory.delete(name);
        }
        match Self::entry(name)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(Error::internal(format!("Keychain: {e}"))),
        }
    }
}

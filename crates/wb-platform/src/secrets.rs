//! Secretos (tokens de los trackers) en el almacén de credenciales del sistema: Administrador de
//! credenciales de Windows, Llavero de macOS, Secret Service en Linux. Nunca en archivos de la app.
//!
//! El nombre del servicio se puede cambiar con [`SECRET_SERVICE_ENV`] (pruebas: así no se tocan las
//! credenciales reales). Los errores nunca incluyen el valor guardado.

use std::collections::HashMap;
use std::io;
use std::sync::Mutex;

/// Variable que cambia el nombre del servicio en el almacén (pruebas, varias instancias).
pub const SECRET_SERVICE_ENV: &str = "WORKBOARD_SECRET_SERVICE";
pub const DEFAULT_SERVICE: &str = "workboard";

/// Dónde se guardan los secretos. `key` identifica el secreto (no es secreto).
pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> io::Result<Option<String>>;
    fn set(&self, key: &str, value: &str) -> io::Result<()>;
    /// Borrar uno que no existe no es un error.
    fn delete(&self, key: &str) -> io::Result<()>;
}

/// El almacén de credenciales del sistema.
pub struct OsSecrets {
    service: String,
}

impl OsSecrets {
    pub fn new(service: &str) -> Self {
        Self { service: service.to_string() }
    }

    /// Con el servicio de [`SECRET_SERVICE_ENV`] o [`DEFAULT_SERVICE`].
    pub fn from_env() -> Self {
        let service = std::env::var(SECRET_SERVICE_ENV).ok().filter(|s| !s.trim().is_empty());
        Self::new(service.as_deref().unwrap_or(DEFAULT_SERVICE))
    }

    fn entry(&self, key: &str) -> io::Result<keyring::Entry> {
        keyring::Entry::new(&self.service, key).map_err(other)
    }
}

/// El error del almacén sin datos: solo su tipo (los mensajes de `keyring` no llevan el secreto, pero
/// así no hay forma de que lo hagan).
fn other(e: keyring::Error) -> io::Error {
    let kind = match e {
        keyring::Error::PlatformFailure(_) => "platform failure",
        keyring::Error::NoStorageAccess(_) => "no access to the credential store",
        keyring::Error::NoEntry => "no entry",
        keyring::Error::BadEncoding(_) => "bad encoding",
        keyring::Error::TooLong(..) => "value too long",
        keyring::Error::Invalid(..) => "invalid key",
        keyring::Error::Ambiguous(_) => "ambiguous entry",
        _ => "credential store error",
    };
    io::Error::other(kind)
}

impl SecretStore for OsSecrets {
    fn get(&self, key: &str) -> io::Result<Option<String>> {
        match self.entry(key)?.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(other(e)),
        }
    }

    fn set(&self, key: &str, value: &str) -> io::Result<()> {
        self.entry(key)?.set_password(value).map_err(other)
    }

    fn delete(&self, key: &str) -> io::Result<()> {
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(other(e)),
        }
    }
}

/// En memoria (tests).
#[derive(Default)]
pub struct MemorySecrets(Mutex<HashMap<String, String>>);

impl SecretStore for MemorySecrets {
    fn get(&self, key: &str) -> io::Result<Option<String>> {
        Ok(self.0.lock().expect("secrets").get(key).cloned())
    }

    fn set(&self, key: &str, value: &str) -> io::Result<()> {
        self.0.lock().expect("secrets").insert(key.to_string(), value.to_string());
        Ok(())
    }

    fn delete(&self, key: &str) -> io::Result<()> {
        self.0.lock().expect("secrets").remove(key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_roundtrip() {
        let s = MemorySecrets::default();
        assert_eq!(s.get("a").unwrap(), None);
        s.set("a", "1").unwrap();
        assert_eq!(s.get("a").unwrap().as_deref(), Some("1"));
        s.delete("a").unwrap();
        s.delete("a").unwrap();
        assert_eq!(s.get("a").unwrap(), None);
    }

    /// El almacén real, con un servicio de prueba que se borra al final. Solo si se pide
    /// (`WORKBOARD_TEST_KEYRING=1`): en CI puede no haber almacén (Linux sin Secret Service).
    #[test]
    fn os_store_roundtrip_when_enabled() {
        if std::env::var("WORKBOARD_TEST_KEYRING").as_deref() != Ok("1") {
            return;
        }
        let s = OsSecrets::new(&format!("workboard-test-{}", std::process::id()));
        s.delete("k").unwrap();
        assert_eq!(s.get("k").unwrap(), None);
        s.set("k", "valor-secreto").unwrap();
        assert_eq!(s.get("k").unwrap().as_deref(), Some("valor-secreto"));
        s.delete("k").unwrap();
        assert_eq!(s.get("k").unwrap(), None);
    }
}

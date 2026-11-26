//! DPAPI de Windows con el alcance del usuario actual: lo que usaba la v1 para guardar los PATs en su
//! `config.json` (`CryptProtectData` sin entropía, en base64). Solo hace falta para migrar desde la v1:
//! en los demás sistemas no hay nada que descifrar ([`SUPPORTED`] es `false` y todo devuelve
//! `Unsupported`). Los errores nunca llevan el valor (ni cifrado ni en claro).

use std::io;

/// Si este sistema tiene DPAPI (y por lo tanto puede haber una v1 que migrar).
pub const SUPPORTED: bool = cfg!(windows);

#[cfg(windows)]
fn failed() -> io::Error {
    io::Error::other("DPAPI could not process the value")
}

/// Cifra `secret` como la v1 (`ado.protect`): DPAPI de usuario, base64. Para armar fixtures en los tests.
#[cfg(windows)]
pub fn protect(secret: &str) -> io::Result<String> {
    use base64::Engine;
    let data = windows_dpapi::encrypt_data(secret.as_bytes(), windows_dpapi::Scope::User, None)
        .map_err(|_| failed())?;
    Ok(base64::engine::general_purpose::STANDARD.encode(data))
}

/// Descifra un valor de la v1 (`ado.unprotect`).
#[cfg(windows)]
pub fn unprotect(token: &str) -> io::Result<String> {
    use base64::Engine;
    let blob = base64::engine::general_purpose::STANDARD
        .decode(token.trim())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "not base64"))?;
    let data = windows_dpapi::decrypt_data(&blob, windows_dpapi::Scope::User, None).map_err(|_| failed())?;
    String::from_utf8(data).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "not utf-8"))
}

#[cfg(not(windows))]
pub fn protect(_secret: &str) -> io::Result<String> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "DPAPI is only available on Windows"))
}

#[cfg(not(windows))]
pub fn unprotect(_token: &str) -> io::Result<String> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "DPAPI is only available on Windows"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_or_unsupported() {
        let fake = "fake-token-for-tests";
        match (protect(fake), SUPPORTED) {
            (Ok(token), true) => {
                assert!(!token.contains(fake));
                assert_eq!(unprotect(&token).unwrap(), fake);
                let err = unprotect("bm90IGEgYmxvYg==").unwrap_err();
                assert!(!err.to_string().contains("bm90"));
                assert!(unprotect("%%%").is_err());
            }
            (Err(e), false) => assert_eq!(e.kind(), io::ErrorKind::Unsupported),
            (r, supported) => panic!("protect: {:?} with SUPPORTED = {supported}", r.map(|_| "<token>")),
        }
    }
}

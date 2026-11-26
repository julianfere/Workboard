//! Consola para los subcomandos (`workboard migrate-v1`, `workboard plugin …`).
//!
//! En Windows la app de release es un programa de ventana (`windows_subsystem = "windows"`): no tiene consola
//! y lo que escribe en stdout se pierde aunque se la lance desde una. [`attach_parent`] la engancha a la
//! consola del proceso que la lanzó. Si la salida ya va a otro lado (redirigida a un archivo o a un pipe), o
//! en los demás sistemas, no hace nada.
//!
//! Es el único `unsafe` del workspace (FFI de Windows, sin wrapper seguro disponible).

/// Engancha stdout y stderr a la consola del proceso padre si no tienen destino. Devuelve `true` si lo hizo.
pub fn attach_parent() -> bool {
    imp::attach_parent()
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Console::{
        ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_HANDLE, STD_OUTPUT_HANDLE,
        SetStdHandle,
    };

    fn missing(which: STD_HANDLE) -> bool {
        // SAFETY: GetStdHandle solo lee la tabla de handles estándar del proceso.
        let h: HANDLE = unsafe { GetStdHandle(which) };
        h.is_null() || h == INVALID_HANDLE_VALUE
    }

    pub fn attach_parent() -> bool {
        // Redirigida (archivo, pipe): se respeta.
        if !missing(STD_OUTPUT_HANDLE) {
            return false;
        }
        // SAFETY: AttachConsole no toma punteros; falla sin efectos si no hay consola padre o ya hay una.
        if unsafe { AttachConsole(ATTACH_PARENT_PROCESS) } == 0 {
            return false;
        }
        let name: Vec<u16> = "CONOUT$\0".encode_utf16().collect();
        // SAFETY: `name` es un string UTF-16 terminado en 0 que vive durante la llamada; el resto son
        // constantes o nulos que CreateFileW acepta.
        let out = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };
        if out == INVALID_HANDLE_VALUE {
            return false;
        }
        // La salida de Rust pide el handle estándar en cada escritura: alcanza con fijarlo acá.
        // SAFETY: `out` es un handle válido recién abierto; queda abierto mientras viva el proceso.
        unsafe {
            SetStdHandle(STD_OUTPUT_HANDLE, out);
            if missing(STD_ERROR_HANDLE) {
                SetStdHandle(STD_ERROR_HANDLE, out);
            }
        }
        true
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn attach_parent() -> bool {
        false
    }
}

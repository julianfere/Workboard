//! Lado del plugin, para plugins escritos en Rust: lee los mensajes de stdin, contesta `initialize` y
//! pasa la llamada al handler, que puede hacerle pedidos a la app con `Client::request`.

use std::io::{BufRead, Write};

use serde_json::{Value as Json, json};

/// Pedidos a la app durante una llamada.
pub struct Client<'a> {
    input: &'a mut dyn BufRead,
    output: &'a mut dyn Write,
    next: u64,
    /// Lo que mandó la app en `initialize`.
    pub init: Json,
}

impl Client<'_> {
    fn send(&mut self, msg: &Json) -> Result<(), String> {
        let line = serde_json::to_string(msg).map_err(|e| e.to_string())?;
        writeln!(self.output, "{line}").and_then(|_| self.output.flush()).map_err(|e| e.to_string())
    }

    /// Manda `method` a la app y espera la respuesta. `Err` = el código de error que devolvió.
    pub fn request(&mut self, method: &str, params: Json) -> Result<Json, String> {
        self.next += 1;
        let id = 1000 + self.next;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        loop {
            let mut line = String::new();
            if self.input.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
                return Err("host closed the connection".into());
            }
            let msg: Json = serde_json::from_str(line.trim()).map_err(|e| e.to_string())?;
            if msg["id"].as_u64() == Some(id) {
                if let Some(e) = msg.get("error").filter(|e| !e.is_null()) {
                    return Err(e["message"].as_str().unwrap_or("error").to_string());
                }
                return Ok(msg["result"].clone());
            }
        }
    }

    /// El idioma que pidió la app (`en` si no dijo).
    pub fn lang(&self) -> &str {
        self.init["lang"].as_str().unwrap_or("en")
    }
}

/// Atiende la conexión hasta que la app cierra stdin. `handler(method, params, client)` contesta
/// `action/open` y `action/submit`; su `Err` vuelve como error JSON-RPC.
pub fn serve(
    input: &mut dyn BufRead,
    output: &mut dyn Write,
    mut handler: impl FnMut(&str, &Json, &mut Client) -> Result<Json, String>,
) -> Result<(), String> {
    let mut client = Client { input, output, next: 0, init: Json::Null };
    loop {
        let mut line = String::new();
        if client.input.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
            return Ok(());
        }
        if line.trim().is_empty() {
            continue;
        }
        let msg: Json = serde_json::from_str(line.trim()).map_err(|e| e.to_string())?;
        let (Some(method), id) = (msg["method"].as_str(), msg["id"].clone()) else { continue };
        let out = if method == "initialize" {
            client.init = msg["params"].clone();
            Ok(json!({}))
        } else {
            handler(method, &msg["params"], &mut client)
        };
        let reply = match out {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err(e) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": e}}),
        };
        client.send(&reply)?;
    }
}

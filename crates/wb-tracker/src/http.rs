//! Transporte HTTP de los trackers. El cliente arma pedidos y lee respuestas; quién los manda es un
//! `Transport`: en la app, `UreqTransport` (ureq + rustls con los certificados del sistema, sirve detrás
//! de proxies corporativos que inspeccionan TLS); en los tests, respuestas fijas sin red.

use std::time::Duration;

/// Tiempo máximo de cada pedido.
pub const TIMEOUT: Duration = Duration::from_secs(30);
/// Tamaño máximo de una respuesta (los lotes de work items son de 200).
const MAX_BODY: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
    pub body: Option<Vec<u8>>,
}

impl HttpRequest {
    /// Valor de un header (sin distinguir mayúsculas).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
}

/// Manda un pedido. `Err` = no hubo respuesta (red, DNS, TLS, timeout); un 4xx/5xx es una respuesta.
pub trait Transport: Send + Sync {
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse, String>;
}

pub struct UreqTransport {
    agent: ureq::Agent,
}

impl Default for UreqTransport {
    fn default() -> Self {
        let tls = ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build();
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .http_status_as_error(false)
            .tls_config(tls)
            .user_agent("workboard")
            .build()
            .new_agent();
        Self { agent }
    }
}

impl Transport for UreqTransport {
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse, String> {
        let mut builder = ureq::http::Request::builder().method(req.method).uri(&req.url);
        for (k, v) in &req.headers {
            builder = builder.header(*k, v);
        }
        let res = match &req.body {
            Some(body) => self.agent.run(builder.body(body.clone()).map_err(|e| e.to_string())?),
            None => self.agent.run(builder.body(()).map_err(|e| e.to_string())?),
        };
        let mut res = res.map_err(|e| e.to_string())?;
        let content_type =
            res.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
        let status = res.status().as_u16();
        let body = res.body_mut().with_config().limit(MAX_BODY).read_to_vec().map_err(|e| e.to_string())?;
        Ok(HttpResponse { status, content_type, body })
    }
}

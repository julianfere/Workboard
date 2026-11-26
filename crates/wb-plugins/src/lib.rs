//! Plugins: ejecutables aparte, con un `manifest.json`, que hablan JSON-RPC por stdio con la app. Aportan
//! acciones (formularios que arma un modal genérico) y presets de flujo. Ver `docs/plugins.md`.

pub mod cli;
pub mod manifest;
pub mod rpc;
pub mod sdk;

pub use manifest::{
    Action, Manifest, PERMISSIONS, PLACEMENTS, PROTOCOL, Plugin, Plugins, Text, When, matches,
    resolve_workflow,
};
pub use rpc::{Host, PluginError, call, handshake};

//! Plugins: ejecutables aparte, con un `manifest.json`, que hablan JSON-RPC por stdio con la app. Aportan
//! acciones (formularios que arma un modal genérico), presets de flujo y trackers. Ver `docs/plugins.md`.

pub mod manifest;
pub mod rpc;
pub mod sdk;
pub mod trackers;

pub use manifest::{
    Action, Manifest, PERMISSIONS, PLACEMENTS, PROTOCOL, Plugin, Plugins, Text, When, matches,
    resolve_workflow,
};
pub use rpc::{Host, PluginError, call};
pub use trackers::{PluginTracker, TrackerClash, TrackerDecl};

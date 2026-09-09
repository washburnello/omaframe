//! omaframe — native terminal TUI wireframing app.
//!
//! Scaffold wave: document model only (`crate::model`). The full Ratatui UI,
//! tools, palette, theme provider, and CLI exporters arrive in later waves.

pub mod model;
pub mod draw;
pub mod theme;
pub mod clipboard;
pub mod chars;

//! The readers. Those here are pure Rust and run everywhere; macOS adds its
//! own (PDFKit, Vision and others) in macos/.

pub mod html;
pub mod json;
pub mod text;

#[cfg(target_os = "macos")]
pub mod macos;

use super::Reader;
use std::sync::Arc;

/// Every reader that runs on any platform.
pub fn all() -> Vec<Arc<dyn Reader>> {
    vec![Arc::new(text::Text), Arc::new(text::Code), Arc::new(text::Plain), Arc::new(json::Json), Arc::new(html::Html)]
}

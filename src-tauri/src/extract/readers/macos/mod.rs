pub mod image;
pub mod pdf;
mod supervise;

use crate::extract::Registry;
use std::sync::Arc;

pub fn register(registry: &mut Registry) {
    registry.register(Arc::new(pdf::Pdf::default()));
    registry.register(Arc::new(image::Image::default()));
}

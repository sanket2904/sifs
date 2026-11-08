pub mod pdf;

use crate::extract::Registry;
use std::sync::Arc;

pub fn register(registry: &mut Registry) {
    registry.register(Arc::new(pdf::Pdf::default()));
}

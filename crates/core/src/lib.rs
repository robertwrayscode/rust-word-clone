//! RWord core: document model, styles, editing operations, layout and file formats.

pub mod edit;
pub mod model;
pub mod styles;
pub mod util;

pub use edit::{CellRef, FindOptions, ParaPath, Pos};
pub use model::*;

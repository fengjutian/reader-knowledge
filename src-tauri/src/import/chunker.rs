//! 分块逻辑的独立入口。
//!
//! 实际实现放在 `mod.rs`（`split_into_chunks`），这里只做一次重导出，
//! 让 `epub` / `pdf` 依赖 `chunker::…` 而不是 `super::…`，读起来更清楚。

#[allow(unused_imports)]
pub use super::{split_into_chunks, validate_size, Locator, MAX_DOCUMENTS_PER_SOURCE};

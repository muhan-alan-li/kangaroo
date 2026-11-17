//! The fit-brief engine: three model stages, a mechanical validator, and the
//! SQLite rows the dashboard reads.

pub mod chain;
pub mod llm;
pub mod stages;
pub mod validate;

pub use chain::generate;
pub use llm::LLMError;

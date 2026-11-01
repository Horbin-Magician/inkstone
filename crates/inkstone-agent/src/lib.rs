//! UI-independent notebook agent. The model can propose edits, never apply them.
pub mod provider;
pub mod runtime;
pub mod storage;
pub mod tools;
pub use provider::{ModelConfig, ModelProvider};
pub use runtime::{AgentEvent, AgentRun, RunState};
pub use tools::{Scope, ToolHost};

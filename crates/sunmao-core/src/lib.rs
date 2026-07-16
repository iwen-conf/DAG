//! sunmao-core: pure domain logic, zero IO.
//! No tokio / sqlx / git dependencies.
#![forbid(unsafe_code)]

pub mod graph;
pub mod id;
pub mod ready;
pub mod state_machine;
pub mod write_scope;

pub use graph::{
    GraphSnapshot, GraphViolation, NodeDraft, PublishInput, PublishResult, ValidationReport,
    BUILTIN_VALIDATORS,
};
pub use id::{new_id, ArtifactId, AttemptId, EventId, NodeId, ProjectId};
pub use ready::{compute_ready, ReadyInput};
pub use state_machine::{
    PlanState, ScopeState, TaskState, TransitionError, TransitionKind,
};
pub use write_scope::{scopes_conflict, scopes_prefix_intersect, path_in_scopes};

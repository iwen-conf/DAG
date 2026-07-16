//! sunmao-store: PostgreSQL + git workspace adapters.

pub mod error;
pub mod event;
pub mod git;
pub mod projects;
pub mod ready_maint;
pub mod repo;
pub mod validators;

pub use error::{StoreError, StoreResult};
pub use git::GitWorkspace;
pub use projects::{Project, ProjectsRepo};
pub use repo::Store;

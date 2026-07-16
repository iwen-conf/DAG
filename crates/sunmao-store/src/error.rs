use thiserror::Error;

pub type StoreResult<T> = Result<T, StoreError>;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database: {0}")]
    Db(#[from] sqlx::Error),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("conflict: {code}: {message}")]
    Conflict { code: String, message: String },
    #[error("validation: {code}: {message}")]
    Validation {
        code: String,
        message: String,
        details: serde_json::Value,
    },
    #[error("git: {0}")]
    Git(String),
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("internal: {0}")]
    Internal(String),
}

impl StoreError {
    pub fn lease_lost(msg: impl Into<String>) -> Self {
        Self::Conflict {
            code: "LEASE_LOST".into(),
            message: msg.into(),
        }
    }

    pub fn stale_graph(msg: impl Into<String>) -> Self {
        Self::Conflict {
            code: "STALE_GRAPH_VERSION".into(),
            message: msg.into(),
        }
    }

    pub fn handover_required() -> Self {
        Self::Conflict {
            code: "HANDOVER_REVIEW_REQUIRED".into(),
            message: "relay attempt must call handover-review before submit".into(),
        }
    }
}

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Task execution state (D-04 includes `cancelled`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Todo,
    Ready,
    Claimed,
    Running,
    Review,
    Done,
    Failed,
    Cancelled,
}

impl TaskState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::Ready => "ready",
            Self::Claimed => "claimed",
            Self::Running => "running",
            Self::Review => "review",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "todo" => Some(Self::Todo),
            "ready" => Some(Self::Ready),
            "claimed" => Some(Self::Claimed),
            "running" => Some(Self::Running),
            "review" => Some(Self::Review),
            "done" => Some(Self::Done),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    pub fn all() -> &'static [TaskState] {
        &[
            Self::Todo,
            Self::Ready,
            Self::Claimed,
            Self::Running,
            Self::Review,
            Self::Done,
            Self::Failed,
            Self::Cancelled,
        ]
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Cancelled)
    }

    pub fn is_active_lease(self) -> bool {
        matches!(self, Self::Claimed | Self::Running | Self::Review)
    }
}

/// Package scope state — human-controlled (D-18).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeState {
    Active,
    Paused,
    Closed,
    Archived,
}

impl ScopeState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Closed => "closed",
            Self::Archived => "archived",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "active" => Some(Self::Active),
            "paused" => Some(Self::Paused),
            "closed" => Some(Self::Closed),
            "archived" => Some(Self::Archived),
            _ => None,
        }
    }

    pub fn all() -> &'static [ScopeState] {
        &[Self::Active, Self::Paused, Self::Closed, Self::Archived]
    }

    pub fn is_restricting(self) -> bool {
        !matches!(self, Self::Active)
    }
}

/// Package plan state — Planner-written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanState {
    Draft,
    Planning,
    Planned,
}

impl PlanState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Planning => "planning",
            Self::Planned => "planned",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "draft" => Some(Self::Draft),
            "planning" => Some(Self::Planning),
            "planned" => Some(Self::Planned),
            _ => None,
        }
    }

    pub fn all() -> &'static [PlanState] {
        &[Self::Draft, Self::Planning, Self::Planned]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TransitionError {
    #[error("illegal task transition {from:?} -> {to:?}")]
    IllegalTask { from: TaskState, to: TaskState },
    #[error("illegal scope transition {from:?} -> {to:?}")]
    IllegalScope { from: ScopeState, to: ScopeState },
    #[error("illegal plan transition {from:?} -> {to:?}")]
    IllegalPlan { from: PlanState, to: PlanState },
}

pub enum TransitionKind {
    Task,
    Scope,
    Plan,
}

/// Legal task transitions (storage-side; ready is also set by projection).
///
/// ```text
/// todo → ready → claimed → running → review → done
///                  │          │         │
///                  └──────────┴─────────┴→ ready (reopen / lease_expired)
///                  └──────────┴─────────┴→ failed (max attempts)
///                  └──────────┴─────────┴→ cancelled
/// todo → cancelled (drain force before claim)
/// failed → todo (replan)
/// ```
pub fn task_transition_allowed(from: TaskState, to: TaskState) -> bool {
    if from == to {
        return true; // idempotent no-op
    }
    matches!(
        (from, to),
        (TaskState::Todo, TaskState::Ready)
            | (TaskState::Todo, TaskState::Cancelled)
            | (TaskState::Ready, TaskState::Claimed)
            | (TaskState::Ready, TaskState::Todo) // scope block
            | (TaskState::Ready, TaskState::Cancelled)
            | (TaskState::Claimed, TaskState::Running)
            | (TaskState::Claimed, TaskState::Review) // allow skip running
            | (TaskState::Claimed, TaskState::Ready) // lease expire / reopen
            | (TaskState::Claimed, TaskState::Failed)
            | (TaskState::Claimed, TaskState::Cancelled)
            | (TaskState::Claimed, TaskState::Done) // submit shortcut
            | (TaskState::Running, TaskState::Review)
            | (TaskState::Running, TaskState::Done)
            | (TaskState::Running, TaskState::Ready)
            | (TaskState::Running, TaskState::Failed)
            | (TaskState::Running, TaskState::Cancelled)
            | (TaskState::Review, TaskState::Done)
            | (TaskState::Review, TaskState::Ready)
            | (TaskState::Review, TaskState::Failed)
            | (TaskState::Review, TaskState::Cancelled)
            | (TaskState::Failed, TaskState::Todo) // replan
            | (TaskState::Failed, TaskState::Ready)
            | (TaskState::Failed, TaskState::Cancelled)
    )
}

pub fn scope_transition_allowed(from: ScopeState, to: ScopeState) -> bool {
    if from == to {
        return true;
    }
    matches!(
        (from, to),
        (ScopeState::Active, ScopeState::Paused)
            | (ScopeState::Active, ScopeState::Closed)
            | (ScopeState::Active, ScopeState::Archived)
            | (ScopeState::Paused, ScopeState::Active)
            | (ScopeState::Paused, ScopeState::Closed)
            | (ScopeState::Paused, ScopeState::Archived)
            | (ScopeState::Closed, ScopeState::Active) // reopen
            | (ScopeState::Closed, ScopeState::Archived)
            | (ScopeState::Archived, ScopeState::Active) // rare restore
    )
}

pub fn plan_transition_allowed(from: PlanState, to: PlanState) -> bool {
    if from == to {
        return true;
    }
    matches!(
        (from, to),
        (PlanState::Draft, PlanState::Planning)
            | (PlanState::Draft, PlanState::Planned)
            | (PlanState::Planning, PlanState::Planned)
            | (PlanState::Planning, PlanState::Draft)
            | (PlanState::Planned, PlanState::Planning) // replan
            | (PlanState::Planned, PlanState::Draft)
    )
}

pub fn assert_task_transition(from: TaskState, to: TaskState) -> Result<(), TransitionError> {
    if task_transition_allowed(from, to) {
        Ok(())
    } else {
        Err(TransitionError::IllegalTask { from, to })
    }
}

pub fn assert_scope_transition(from: ScopeState, to: ScopeState) -> Result<(), TransitionError> {
    if scope_transition_allowed(from, to) {
        Ok(())
    } else {
        Err(TransitionError::IllegalScope { from, to })
    }
}

pub fn assert_plan_transition(from: PlanState, to: PlanState) -> Result<(), TransitionError> {
    if plan_transition_allowed(from, to) {
        Ok(())
    } else {
        Err(TransitionError::IllegalPlan { from, to })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn happy_path_task_lifecycle() {
        let path = [
            TaskState::Todo,
            TaskState::Ready,
            TaskState::Claimed,
            TaskState::Running,
            TaskState::Review,
            TaskState::Done,
        ];
        for w in path.windows(2) {
            assert_task_transition(w[0], w[1]).unwrap();
        }
    }

    #[test]
    fn illegal_task_transitions_fail() {
        assert!(assert_task_transition(TaskState::Done, TaskState::Ready).is_err());
        assert!(assert_task_transition(TaskState::Todo, TaskState::Done).is_err());
        assert!(assert_task_transition(TaskState::Cancelled, TaskState::Ready).is_err());
        assert!(assert_task_transition(TaskState::Done, TaskState::Todo).is_err());
    }

    #[test]
    fn lease_expire_reopens() {
        assert_task_transition(TaskState::Claimed, TaskState::Ready).unwrap();
        assert_task_transition(TaskState::Running, TaskState::Ready).unwrap();
    }

    #[test]
    fn scope_pause_close_reopen() {
        assert_scope_transition(ScopeState::Active, ScopeState::Paused).unwrap();
        assert_scope_transition(ScopeState::Paused, ScopeState::Closed).unwrap();
        assert_scope_transition(ScopeState::Closed, ScopeState::Active).unwrap();
        assert!(assert_scope_transition(ScopeState::Archived, ScopeState::Paused).is_err());
    }

    #[test]
    fn plan_states() {
        assert_plan_transition(PlanState::Draft, PlanState::Planned).unwrap();
        assert_plan_transition(PlanState::Planned, PlanState::Planning).unwrap();
    }

    proptest! {
        #[test]
        fn task_transition_table_is_total(
            from in prop::sample::select(TaskState::all()),
            to in prop::sample::select(TaskState::all()),
        ) {
            let ok = task_transition_allowed(from, to);
            let res = assert_task_transition(from, to);
            assert_eq!(ok, res.is_ok());
            if !ok {
                match res {
                    Err(TransitionError::IllegalTask { from: f, to: t }) => {
                        assert_eq!(f, from);
                        assert_eq!(t, to);
                    }
                    other => panic!("expected IllegalTask, got {other:?}"),
                }
            }
        }

        #[test]
        fn scope_transition_table_is_total(
            from in prop::sample::select(ScopeState::all()),
            to in prop::sample::select(ScopeState::all()),
        ) {
            let ok = scope_transition_allowed(from, to);
            assert_eq!(ok, assert_scope_transition(from, to).is_ok());
        }

        #[test]
        fn plan_transition_table_is_total(
            from in prop::sample::select(PlanState::all()),
            to in prop::sample::select(PlanState::all()),
        ) {
            let ok = plan_transition_allowed(from, to);
            assert_eq!(ok, assert_plan_transition(from, to).is_ok());
        }
    }

    #[test]
    fn roundtrip_as_str() {
        for s in TaskState::all() {
            assert_eq!(TaskState::parse(s.as_str()), Some(*s));
        }
        for s in ScopeState::all() {
            assert_eq!(ScopeState::parse(s.as_str()), Some(*s));
        }
        for s in PlanState::all() {
            assert_eq!(PlanState::parse(s.as_str()), Some(*s));
        }
    }
}

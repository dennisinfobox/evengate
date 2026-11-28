use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::spec::Spec;

/// The complete event vocabulary for v0. Events are serialized as JSON Lines
/// with declaration-order fields, so the encoding is stable across runs.
///
/// Nothing is added here without a corresponding reducer rule in `state.rs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    TaskCreated {
        spec: Spec,
        /// Mandatory gates in force when the task was created. Empty in logs
        /// written before this field existed; the reducer maps those to the
        /// historical gate set so replay never reinterprets a past task.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        gates: Vec<GateId>,
    },
    /// The repository tree a self-edit started from, recorded before the first
    /// model call so materialization never needs the live checkout.
    SelfEditBase {
        files: BTreeMap<String, String>,
    },
    AgentInvoked {
        role: AgentRole,
        prompt_hash: String,
    },
    AgentResponded {
        role: AgentRole,
        response: String,
        tokens: TokenUsage,
    },
    CommandExecuted {
        cmd: String,
        exit_code: i32,
        stdout: String,
        stderr: String,
    },
    GateEvaluated {
        gate: GateId,
        result: GateResult,
    },
    HumanApproved {
        reviewer: String,
        /// Required when the task's recorded gate set differs from the running
        /// binary's mandatory set. This is the only gate-set override path.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    HumanRejected {
        reviewer: String,
        reason: String,
    },
    /// A human asks the executor to fix a gate-blocked artifact. Valid only
    /// during `Verifying` with at least one non-passing gate verdict.
    RevisionRequested {
        reviewer: String,
        guidance: Option<String>,
    },
    /// A human ends a task that a gate blocked, instead of rejecting a PR.
    HumanAbandoned {
        reviewer: String,
        reason: String,
    },
    /// Cross-task lesson counts recorded by the executor after a review.
    LessonRepeated {
        lesson: String,
        text: String,
        count: u32,
    },
    /// A human promoted a repeated lesson into `rules/<project>.md`.
    LessonPromoted {
        reviewer: String,
        lesson: String,
    },
    /// A human declined promotion for this task.
    LessonDismissed {
        reviewer: String,
        lesson: String,
    },
    PrOpened {
        url: String,
    },
    Merged {
        commit: String,
    },
    TaskAbandoned {
        reason: String,
    },
    /// Reserved for future wall-clock checkpoints; the seed kernel never
    /// emits it because nothing it does depends on time.
    #[allow(dead_code)]
    Tick {
        now_unix: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    /// Reserved by the v0 vocabulary; the seed kernel only invokes the executor.
    #[allow(dead_code)]
    Planner,
    Executor,
    /// Reviews the project tree before human approval.
    Reviewer,
}

impl AgentRole {
    pub fn slug(self) -> &'static str {
        match self {
            AgentRole::Planner => "planner",
            AgentRole::Executor => "executor",
            AgentRole::Reviewer => "reviewer",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateId {
    Compiles,
    TestsPass,
    LintsClean,
    /// `cargo fmt --check` on the artifact.
    FormatClean,
    /// LLM-backed review of the project tree; the last mandatory v0 gate.
    ReviewAgent,
    /// Reserved; human approval is an event, not a deterministic gate.
    #[allow(dead_code)]
    HumanApproval,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum GateResult {
    Pass,
    Fail { reason: String },
    NeedsHuman { question: String },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input: u64,
    pub output: u64,
}

impl TokenUsage {
    pub fn add(&mut self, other: TokenUsage) {
        self.input = self.input.saturating_add(other.input);
        self.output = self.output.saturating_add(other.output);
    }
}

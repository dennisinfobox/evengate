use serde::{Deserialize, Serialize};

use crate::spec::Spec;

/// The complete event vocabulary for v0. Events are serialized as JSON Lines
/// with declaration-order fields, so the encoding is stable across runs.
///
/// Nothing is added here without a corresponding reducer rule in `state.rs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    TaskCreated { spec: Spec },
    AgentInvoked { role: AgentRole, prompt_hash: String },
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
    GateEvaluated { gate: GateId, result: GateResult },
    HumanApproved { reviewer: String },
    HumanRejected { reviewer: String, reason: String },
    PrOpened { url: String },
    Merged { commit: String },
    TaskAbandoned { reason: String },
    /// Reserved for future wall-clock checkpoints; the seed kernel never
    /// emits it because nothing it does depends on time.
    #[allow(dead_code)]
    Tick { now_unix: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    /// Reserved by the v0 vocabulary; the seed kernel only invokes the executor.
    #[allow(dead_code)]
    Planner,
    Executor,
    /// Reserved by the v0 vocabulary; the review agent arrives in week 5–6.
    #[allow(dead_code)]
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
    /// Reserved by the v0 vocabulary; recorded as `NeedsHuman` only.
    #[allow(dead_code)]
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

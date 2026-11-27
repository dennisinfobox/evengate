use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::event::{AgentRole, Event, GateId, GateResult, TokenUsage};
use crate::gate;
use crate::spec::Spec;

/// Phase of a task. `Drafting` and `Generating` are transient; a healthy run
/// passes through them in one executor step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Drafting,
    Generating,
    Verifying,
    Revising,
    Reviewing,
    Merged,
    Abandoned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub path: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum HumanDecision {
    Approved { reviewer: String },
    Rejected { reviewer: String, reason: String },
}

/// Denormalized snapshot. Every field is derived from events by `reduce`;
/// nothing here is written by the executor or the CLI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub phase: Phase,
    pub task_created: bool,
    pub spec: Spec,
    pub artifacts: Vec<ArtifactRef>,
    pub gate_results: BTreeMap<GateId, GateResult>,
    pub gate_attempts: BTreeMap<GateId, u32>,
    /// Number of human-requested revisions so far.
    pub revisions: u32,
    /// Gate verdicts the last revision request was based on.
    pub revision_failures: BTreeMap<GateId, GateResult>,
    /// Optional human guidance attached to the last revision request.
    pub revision_guidance: Option<String>,
    pub token_spend: TokenUsage,
    pub human_decision: Option<HumanDecision>,
    pub agent_invocations: u32,
    pub agent_responses: u32,
    pub pr_url: Option<String>,
    pub merged_commit: Option<String>,
    pub abandoned_reason: Option<String>,
    pub last_tick: Option<u64>,
    /// Events that were recorded but did not fit the current phase. Replay
    /// stays total: an anomaly is data, never a panic.
    pub anomalies: Vec<String>,
}

impl Default for State {
    fn default() -> Self {
        State {
            phase: Phase::Drafting,
            task_created: false,
            spec: Spec::default(),
            artifacts: Vec::new(),
            gate_results: BTreeMap::new(),
            gate_attempts: BTreeMap::new(),
            revisions: 0,
            revision_failures: BTreeMap::new(),
            revision_guidance: None,
            token_spend: TokenUsage::default(),
            human_decision: None,
            agent_invocations: 0,
            agent_responses: 0,
            pr_url: None,
            merged_commit: None,
            abandoned_reason: None,
            last_tick: None,
            anomalies: Vec::new(),
        }
    }
}

pub fn initial() -> State {
    State::default()
}

pub fn all_mandatory_passed(state: &State) -> bool {
    gate::MANDATORY_V0
        .iter()
        .all(|id| matches!(state.gate_results.get(id), Some(GateResult::Pass)))
}

/// Pure, total reducer. Every `(State, Event)` pair yields a valid `State`;
/// unexpected events are recorded in `anomalies` instead of panicking.
pub fn reduce(mut state: State, event: &Event) -> State {
    match event {
        Event::TaskCreated { spec } => {
            if state.task_created {
                anomaly(&mut state, "task_created repeated");
            } else {
                state.spec = spec.clone();
                state.task_created = true;
            }
        }
        Event::AgentInvoked { role, .. } => {
            if !state.task_created {
                anomaly(&mut state, "agent_invoked before task_created");
            }
            state.agent_invocations = state.agent_invocations.saturating_add(1);
            match (state.phase, *role) {
                (Phase::Drafting, AgentRole::Executor) => state.phase = Phase::Generating,
                (Phase::Revising, AgentRole::Executor) => state.phase = Phase::Generating,
                (Phase::Verifying, AgentRole::Reviewer) => {}
                _ => anomaly(&mut state, "agent_invoked outside its phase"),
            }
        }
        Event::AgentResponded {
            role,
            tokens,
            ..
        } => {
            state.agent_responses = state.agent_responses.saturating_add(1);
            state.token_spend.add(*tokens);
            match (state.phase, *role) {
                (Phase::Generating, AgentRole::Executor) => {
                    if !state.artifacts.iter().any(|a| a.path == "project") {
                        state.artifacts.push(ArtifactRef {
                            path: "project".into(),
                            kind: "tree".into(),
                        });
                    }
                    state.phase = Phase::Verifying;
                }
                (Phase::Verifying, AgentRole::Reviewer) => {}
                _ => anomaly(&mut state, "agent_responded outside its phase"),
            }
        }
        Event::CommandExecuted { .. } => {
            if !state.task_created {
                anomaly(&mut state, "command_executed before task_created");
            }
        }
        Event::GateEvaluated { gate, result } => {
            if state.phase != Phase::Verifying {
                anomaly(&mut state, "gate_evaluated outside verifying");
            }
            let attempts = state.gate_attempts.entry(*gate).or_insert(0);
            *attempts = attempts.saturating_add(1);
            state.gate_results.insert(*gate, result.clone());
        }
        Event::HumanApproved { reviewer } => {
            if state.phase != Phase::Reviewing {
                anomaly(&mut state, "human_approved outside reviewing");
            }
            state.human_decision = Some(HumanDecision::Approved {
                reviewer: reviewer.clone(),
            });
        }
        Event::HumanRejected { reviewer, reason } => {
            if state.phase != Phase::Reviewing {
                anomaly(&mut state, "human_rejected outside reviewing");
            }
            state.human_decision = Some(HumanDecision::Rejected {
                reviewer: reviewer.clone(),
                reason: reason.clone(),
            });
        }
        Event::RevisionRequested {
            guidance,
            reviewer: _,
        } => {
            if state.phase != Phase::Verifying {
                anomaly(&mut state, "revision_requested outside verifying");
            } else {
                let failures = failures(&state.gate_results);
                if failures.is_empty() {
                    anomaly(&mut state, "revision_requested without a failing gate");
                } else {
                    state.revisions = state.revisions.saturating_add(1);
                    state.revision_failures = failures;
                    state.revision_guidance = guidance
                        .clone()
                        .filter(|text| !text.trim().is_empty());
                    state.gate_results.clear();
                    state.gate_attempts.clear();
                    state.phase = Phase::Revising;
                }
            }
        }
        Event::HumanAbandoned { reason, .. } => {
            if matches!(state.phase, Phase::Merged | Phase::Abandoned) {
                anomaly(&mut state, "human_abandoned after a terminal phase");
            } else {
                state.phase = Phase::Abandoned;
                state.abandoned_reason = Some(reason.clone());
            }
        }
        Event::PrOpened { url } => {
            if state.phase != Phase::Verifying {
                anomaly(&mut state, "pr_opened outside verifying");
            }
            state.pr_url = Some(url.clone());
            state.phase = Phase::Reviewing;
        }
        Event::Merged { commit } => {
            if all_mandatory_passed(&state)
                && matches!(state.human_decision, Some(HumanDecision::Approved { .. }))
            {
                state.phase = Phase::Merged;
                state.merged_commit = Some(commit.clone());
            } else {
                anomaly(&mut state, "merged without passed gates and human approval");
            }
        }
        Event::TaskAbandoned { reason } => {
            state.phase = Phase::Abandoned;
            state.abandoned_reason = Some(reason.clone());
        }
        Event::Tick { now_unix } => {
            state.last_tick = Some(*now_unix);
        }
    }
    state
}

fn failures(results: &BTreeMap<GateId, GateResult>) -> BTreeMap<GateId, GateResult> {
    results
        .iter()
        .filter(|(_, result)| !matches!(result, GateResult::Pass))
        .map(|(gate, result)| (*gate, result.clone()))
        .collect()
}

fn anomaly(state: &mut State, message: &str) {
    state.anomalies.push(message.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::TokenUsage;

    fn created() -> State {
        reduce(
            initial(),
            &Event::TaskCreated {
                spec: Spec {
                    title: "hello".into(),
                    goal: "g".into(),
                    acceptance: vec!["a".into()],
                    constraints: vec!["c".into()],
                    ..Spec::default()
                },
            },
        )
    }

    fn passed(state: State) -> State {
        gate::MANDATORY_V0.iter().fold(state, |s, id| {
            reduce(
                s,
                &Event::GateEvaluated {
                    gate: *id,
                    result: GateResult::Pass,
                },
            )
        })
    }

    #[test]
    fn unexpected_events_become_anomalies() {
        let state = reduce(created(), &Event::HumanApproved { reviewer: "x".into() });
        assert_eq!(state.anomalies.len(), 1);
        assert_eq!(state.phase, Phase::Drafting);
    }

    #[test]
    fn merged_requires_approval_and_gates() {
        let state = reduce(
            created(),
            &Event::AgentInvoked {
                role: crate::event::AgentRole::Executor,
                prompt_hash: "0".into(),
            },
        );
        let state = reduce(
            state,
            &Event::AgentResponded {
                role: crate::event::AgentRole::Executor,
                response: "r".into(),
                tokens: TokenUsage::default(),
            },
        );
        let state = passed(state);
        let state = reduce(state, &Event::Merged { commit: "abc".into() });
        assert_ne!(state.phase, Phase::Merged);
        assert_eq!(state.anomalies.len(), 1);

        let state = reduce(state, &Event::HumanApproved { reviewer: "d".into() });
        let state = reduce(state, &Event::Merged { commit: "abc".into() });
        assert_eq!(state.phase, Phase::Merged);
        assert_eq!(state.merged_commit.as_deref(), Some("abc"));
    }

    #[test]
    fn token_spend_accumulates() {
        let tokens = TokenUsage { input: 2, output: 3 };
        let state = created();
        let state = reduce(state, &Event::AgentInvoked { role: crate::event::AgentRole::Executor, prompt_hash: "0".into() });
        let state = reduce(state, &Event::AgentResponded { role: crate::event::AgentRole::Executor, response: "r".into(), tokens });
        assert_eq!(state.token_spend, TokenUsage { input: 2, output: 3 });
        assert_eq!(state.phase, Phase::Verifying);
    }

    #[test]
    fn reviewer_turns_in_verification_are_not_anomalies() {
        let state = reduce(
            created(),
            &Event::AgentInvoked {
                role: AgentRole::Executor,
                prompt_hash: "0".into(),
            },
        );
        let state = reduce(
            state,
            &Event::AgentResponded {
                role: AgentRole::Executor,
                response: "r".into(),
                tokens: TokenUsage::default(),
            },
        );
        let state = reduce(
            state,
            &Event::AgentInvoked {
                role: AgentRole::Reviewer,
                prompt_hash: "1".into(),
            },
        );
        let state = reduce(
            state,
            &Event::AgentResponded {
                role: AgentRole::Reviewer,
                response: "{\"approve\":true}".into(),
                tokens: TokenUsage { input: 5, output: 1 },
            },
        );
        assert!(state.anomalies.is_empty(), "{:?}", state.anomalies);
        assert_eq!(state.phase, Phase::Verifying);
        assert_eq!(state.agent_invocations, 2);
        assert_eq!(state.agent_responses, 2);
        assert_eq!(state.token_spend, TokenUsage { input: 5, output: 1 });
    }

    #[test]
    fn executor_turns_in_verification_are_anomalies() {
        let state = reduce(
            created(),
            &Event::AgentInvoked {
                role: AgentRole::Executor,
                prompt_hash: "0".into(),
            },
        );
        let state = reduce(
            state,
            &Event::AgentResponded {
                role: AgentRole::Executor,
                response: "r".into(),
                tokens: TokenUsage::default(),
            },
        );
        let state = reduce(
            state,
            &Event::AgentInvoked {
                role: AgentRole::Executor,
                prompt_hash: "1".into(),
            },
        );
        assert_eq!(state.anomalies.len(), 1);
        assert!(state.anomalies[0].contains("outside its phase"));
    }

    fn blocked() -> State {
        let state = reduce(
            created(),
            &Event::AgentInvoked {
                role: AgentRole::Executor,
                prompt_hash: "0".into(),
            },
        );
        let state = reduce(
            state,
            &Event::AgentResponded {
                role: AgentRole::Executor,
                response: "r".into(),
                tokens: TokenUsage::default(),
            },
        );
        let state = reduce(
            state,
            &Event::GateEvaluated {
                gate: GateId::Compiles,
                result: GateResult::Fail {
                    reason: "boom".into(),
                },
            },
        );
        reduce(
            state,
            &Event::GateEvaluated {
                gate: GateId::TestsPass,
                result: GateResult::Pass,
            },
        )
    }

    #[test]
    fn revision_clears_gates_and_enters_revising() {
        let state = reduce(
            blocked(),
            &Event::RevisionRequested {
                reviewer: "d".into(),
                guidance: Some("fix the type error".into()),
            },
        );
        assert_eq!(state.phase, Phase::Revising);
        assert_eq!(state.revisions, 1);
        assert!(state.gate_results.is_empty());
        assert!(state.gate_attempts.is_empty());
        assert!(matches!(
            state.revision_failures.get(&GateId::Compiles),
            Some(GateResult::Fail { .. })
        ));
        assert_eq!(
            state.revision_guidance.as_deref(),
            Some("fix the type error")
        );
        assert!(state.anomalies.is_empty(), "{:?}", state.anomalies);
    }

    #[test]
    fn revision_without_a_failure_is_an_anomaly() {
        let mut state = created();
        state.phase = Phase::Verifying;
        state = passed(state);
        state = reduce(
            state,
            &Event::RevisionRequested {
                reviewer: "d".into(),
                guidance: None,
            },
        );
        assert_eq!(state.phase, Phase::Verifying);
        assert_eq!(state.revisions, 0);
        assert_eq!(state.anomalies.len(), 1);
    }

    #[test]
    fn revision_runs_the_executor_again() {
        let state = reduce(
            blocked(),
            &Event::RevisionRequested {
                reviewer: "d".into(),
                guidance: None,
            },
        );
        let state = reduce(
            state,
            &Event::AgentInvoked {
                role: AgentRole::Executor,
                prompt_hash: "1".into(),
            },
        );
        assert_eq!(state.phase, Phase::Generating);
        let state = reduce(
            state,
            &Event::AgentResponded {
                role: AgentRole::Executor,
                response: "r2".into(),
                tokens: TokenUsage::default(),
            },
        );
        assert_eq!(state.phase, Phase::Verifying);
        assert!(state.anomalies.is_empty(), "{:?}", state.anomalies);
    }

    #[test]
    fn human_abandonment_is_terminal() {
        let state = reduce(
            created(),
            &Event::HumanAbandoned {
                reviewer: "d".into(),
                reason: "not worth it".into(),
            },
        );
        assert_eq!(state.phase, Phase::Abandoned);
        assert_eq!(state.abandoned_reason.as_deref(), Some("not worth it"));
        let state = reduce(
            state,
            &Event::HumanAbandoned {
                reviewer: "d".into(),
                reason: "again".into(),
            },
        );
        assert_eq!(state.anomalies.len(), 1);
    }
}

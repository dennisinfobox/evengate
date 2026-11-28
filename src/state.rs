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
    /// Human-facing detail for a review request, such as a promotion proposal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// A lesson seen in several tasks, with one representative spelling.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepeatedLesson {
    pub text: String,
    pub count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromotionDecision {
    Promoted,
    Dismissed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum HumanDecision {
    Approved {
        reviewer: String,
        /// Present when the human overrode a task created under a different
        /// gate set than the running binary.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    Rejected {
        reviewer: String,
        reason: String,
    },
}

/// Denormalized snapshot. Every field is derived from events by `reduce`;
/// nothing here is written by the executor or the CLI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub phase: Phase,
    pub task_created: bool,
    pub spec: Spec,
    /// Mandatory gates recorded when the task was created. Empty only for a
    /// state loaded from a pre-snapshot log; `gate_set()` supplies the
    /// historical set in that case.
    #[serde(default, skip_serializing_if = "is_legacy_gate_set")]
    pub gates: Vec<GateId>,
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
    /// Agent invocations since the phase last changed. Feeds the per-phase
    /// loop-protection budget; zero is the normal terminal value.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub phase_invocations: u32,
    pub agent_responses: u32,
    pub pr_url: Option<String>,
    pub merged_commit: Option<String>,
    pub abandoned_reason: Option<String>,
    pub last_tick: Option<u64>,
    /// Events that were recorded but did not fit the current phase. Replay
    /// stays total: an anomaly is data, never a panic.
    pub anomalies: Vec<String>,
    /// Latest cross-task counts for lessons at or above the promotion threshold.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub repeated_lessons: BTreeMap<String, RepeatedLesson>,
    /// Human promotion decisions for this task, keyed by canonical lesson.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub promotion_decisions: BTreeMap<String, PromotionDecision>,
}

impl Default for State {
    fn default() -> Self {
        State {
            phase: Phase::Drafting,
            task_created: false,
            spec: Spec::default(),
            gates: Vec::new(),
            artifacts: Vec::new(),
            gate_results: BTreeMap::new(),
            gate_attempts: BTreeMap::new(),
            revisions: 0,
            revision_failures: BTreeMap::new(),
            revision_guidance: None,
            token_spend: TokenUsage::default(),
            human_decision: None,
            agent_invocations: 0,
            phase_invocations: 0,
            agent_responses: 0,
            pr_url: None,
            merged_commit: None,
            abandoned_reason: None,
            last_tick: None,
            anomalies: Vec::new(),
            repeated_lessons: BTreeMap::new(),
            promotion_decisions: BTreeMap::new(),
        }
    }
}

pub fn initial() -> State {
    State::default()
}

impl State {
    /// The gates that judge this task. A task records its set at creation;
    /// only a state loaded from a pre-snapshot log falls back to the
    /// historical set.
    pub fn gate_set(&self) -> &[GateId] {
        if self.gates.is_empty() {
            &gate::LEGACY_V0
        } else {
            &self.gates
        }
    }
}

pub fn all_mandatory_passed(state: &State) -> bool {
    state
        .gate_set()
        .iter()
        .all(|id| matches!(state.gate_results.get(id), Some(GateResult::Pass)))
}

fn is_zero(value: &u32) -> bool {
    *value == 0
}

fn is_legacy_gate_set(gates: &[GateId]) -> bool {
    gates.is_empty() || gates == gate::LEGACY_V0
}

/// Change phase and restart the per-phase invocation budget. Keeping this in
/// one place is what makes the loop-protection counter trustworthy.
fn enter_phase(state: &mut State, phase: Phase) {
    state.phase = phase;
    state.phase_invocations = 0;
}

/// Pure, total reducer. Every `(State, Event)` pair yields a valid `State`;
/// unexpected events are recorded in `anomalies` instead of panicking.
pub fn reduce(mut state: State, event: &Event) -> State {
    match event {
        Event::TaskCreated { spec, gates } => {
            if state.task_created {
                anomaly(&mut state, "task_created repeated");
            } else {
                state.spec = spec.clone();
                state.gates = if gates.is_empty() {
                    gate::LEGACY_V0.to_vec()
                } else {
                    gates.clone()
                };
                state.task_created = true;
            }
        }
        Event::SelfEditBase { .. } => {
            if !state.task_created {
                anomaly(&mut state, "self_edit_base before task_created");
            }
        }
        Event::AgentInvoked { role, .. } => {
            if !state.task_created {
                anomaly(&mut state, "agent_invoked before task_created");
            }
            state.agent_invocations = state.agent_invocations.saturating_add(1);
            state.phase_invocations = state.phase_invocations.saturating_add(1);
            match (state.phase, *role) {
                (Phase::Drafting, AgentRole::Executor) => {
                    enter_phase(&mut state, Phase::Generating)
                }
                (Phase::Revising, AgentRole::Executor) => {
                    enter_phase(&mut state, Phase::Generating)
                }
                (Phase::Verifying, AgentRole::Reviewer) => {}
                _ => anomaly(&mut state, "agent_invoked outside its phase"),
            }
        }
        Event::AgentResponded { role, tokens, .. } => {
            state.agent_responses = state.agent_responses.saturating_add(1);
            state.token_spend.add(*tokens);
            match (state.phase, *role) {
                (Phase::Generating, AgentRole::Executor) => {
                    if !state.artifacts.iter().any(|a| a.path == "project") {
                        state.artifacts.push(ArtifactRef {
                            path: "project".into(),
                            kind: "tree".into(),
                            detail: None,
                        });
                    }
                    enter_phase(&mut state, Phase::Verifying);
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
        Event::HumanApproved { reviewer, reason } => {
            if state.phase != Phase::Reviewing {
                anomaly(&mut state, "human_approved outside reviewing");
            }
            state.human_decision = Some(HumanDecision::Approved {
                reviewer: reviewer.clone(),
                reason: reason.clone(),
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
                    state.revision_guidance =
                        guidance.clone().filter(|text| !text.trim().is_empty());
                    state.gate_results.clear();
                    state.gate_attempts.clear();
                    enter_phase(&mut state, Phase::Revising);
                }
            }
        }
        Event::HumanAbandoned { reason, .. } => {
            if matches!(state.phase, Phase::Merged | Phase::Abandoned) {
                anomaly(&mut state, "human_abandoned after a terminal phase");
            } else {
                enter_phase(&mut state, Phase::Abandoned);
                state.abandoned_reason = Some(reason.clone());
            }
        }
        Event::LessonRepeated {
            lesson,
            text,
            count,
        } => {
            if state.phase != Phase::Verifying {
                anomaly(&mut state, "lesson_repeated outside verifying");
            }
            let key = lesson.trim();
            if key.is_empty() || *count == 0 {
                anomaly(&mut state, "lesson_repeated without a lesson or count");
            } else {
                state.repeated_lessons.insert(
                    key.to_string(),
                    RepeatedLesson {
                        text: text.clone(),
                        count: *count,
                    },
                );
            }
        }
        Event::LessonPromoted { lesson, .. } => {
            record_promotion(&mut state, lesson, PromotionDecision::Promoted);
        }
        Event::LessonDismissed { lesson, .. } => {
            record_promotion(&mut state, lesson, PromotionDecision::Dismissed);
        }
        Event::PrOpened { url } => {
            if state.phase != Phase::Verifying {
                anomaly(&mut state, "pr_opened outside verifying");
            }
            state.pr_url = Some(url.clone());
            enter_phase(&mut state, Phase::Reviewing);
        }
        Event::Merged { commit } => {
            if all_mandatory_passed(&state)
                && matches!(state.human_decision, Some(HumanDecision::Approved { .. }))
            {
                enter_phase(&mut state, Phase::Merged);
                state.merged_commit = Some(commit.clone());
            } else {
                anomaly(&mut state, "merged without passed gates and human approval");
            }
        }
        Event::TaskAbandoned { reason } => {
            enter_phase(&mut state, Phase::Abandoned);
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

fn record_promotion(state: &mut State, lesson: &str, decision: PromotionDecision) {
    if state.phase != Phase::Verifying {
        anomaly(state, "promotion decision outside verifying");
    }
    let key = lesson.trim();
    if key.is_empty() || !state.repeated_lessons.contains_key(key) {
        anomaly(state, "promotion decision for an unknown lesson");
        return;
    }
    state.promotion_decisions.insert(key.to_string(), decision);
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
                gates: gate::MANDATORY_V0.to_vec(),
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

    fn generated() -> State {
        let state = reduce(
            created(),
            &Event::AgentInvoked {
                role: crate::event::AgentRole::Executor,
                prompt_hash: "0".into(),
            },
        );
        reduce(
            state,
            &Event::AgentResponded {
                role: crate::event::AgentRole::Executor,
                response: "r".into(),
                tokens: TokenUsage::default(),
            },
        )
    }

    #[test]
    fn repeated_lessons_and_promotion_decisions_are_recorded() {
        let state = reduce(
            generated(),
            &Event::LessonRepeated {
                lesson: "keep it small".into(),
                text: "Keep it small.".into(),
                count: 3,
            },
        );
        assert_eq!(state.repeated_lessons["keep it small"].count, 3);
        let state = reduce(
            state,
            &Event::LessonPromoted {
                reviewer: "dennis".into(),
                lesson: "keep it small".into(),
            },
        );
        assert_eq!(
            state.promotion_decisions["keep it small"],
            PromotionDecision::Promoted
        );
        assert!(state.anomalies.is_empty());
    }

    #[test]
    fn promotion_for_an_unknown_lesson_is_an_anomaly() {
        let state = reduce(
            generated(),
            &Event::LessonDismissed {
                reviewer: "dennis".into(),
                lesson: "nope".into(),
            },
        );
        assert!(state
            .anomalies
            .iter()
            .any(|line| line.contains("unknown lesson")));
        assert!(state.promotion_decisions.is_empty());
    }

    #[test]
    fn unexpected_events_become_anomalies() {
        let state = reduce(
            created(),
            &Event::HumanApproved {
                reviewer: "x".into(),
                reason: None,
            },
        );
        assert_eq!(state.anomalies.len(), 1);
        assert_eq!(state.phase, Phase::Drafting);
    }

    #[test]
    fn a_task_records_its_gate_set() {
        let state = created();
        assert_eq!(state.gate_set(), gate::MANDATORY_V0);
        assert!(state.gate_set().contains(&GateId::FormatClean));
    }

    #[test]
    fn a_legacy_state_serializes_without_the_new_fields() {
        let state = reduce(
            initial(),
            &Event::TaskCreated {
                spec: Spec {
                    title: "hello".into(),
                    goal: "g".into(),
                    ..Spec::default()
                },
                gates: Vec::new(),
            },
        );
        let json = crate::log::state_bytes(&state).unwrap();
        assert!(!json.contains("\"gates\""), "{json}");
        assert!(!json.contains("phase_invocations"), "{json}");

        let json = crate::log::state_bytes(&created()).unwrap();
        assert!(json.contains("\"gates\""), "{json}");
    }

    #[test]
    fn a_log_without_a_gate_snapshot_uses_the_historical_set() {
        let state = reduce(
            initial(),
            &Event::TaskCreated {
                spec: Spec {
                    title: "hello".into(),
                    goal: "g".into(),
                    ..Spec::default()
                },
                gates: Vec::new(),
            },
        );
        assert_eq!(state.gate_set(), gate::LEGACY_V0);
        assert!(!state.gate_set().contains(&GateId::FormatClean));
    }

    #[test]
    fn a_phase_change_restarts_the_invocation_budget() {
        let state = reduce(
            created(),
            &Event::AgentInvoked {
                role: AgentRole::Executor,
                prompt_hash: "0".into(),
            },
        );
        assert_eq!(state.phase, Phase::Generating);
        assert_eq!(state.phase_invocations, 0);

        let state = reduce(
            state,
            &Event::AgentResponded {
                role: AgentRole::Executor,
                response: "r".into(),
                tokens: TokenUsage::default(),
            },
        );
        assert_eq!(state.phase, Phase::Verifying);
        assert_eq!(state.phase_invocations, 0);
    }

    #[test]
    fn an_approval_reason_is_recorded() {
        let state = passed(generated());
        let state = reduce(state, &Event::PrOpened { url: "u".into() });
        let state = reduce(
            state,
            &Event::HumanApproved {
                reviewer: "d".into(),
                reason: Some("gate set changed on purpose".into()),
            },
        );
        assert_eq!(
            state.human_decision,
            Some(HumanDecision::Approved {
                reviewer: "d".into(),
                reason: Some("gate set changed on purpose".into()),
            })
        );
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
        let state = reduce(
            state,
            &Event::Merged {
                commit: "abc".into(),
            },
        );
        assert_ne!(state.phase, Phase::Merged);
        assert_eq!(state.anomalies.len(), 1);

        let state = reduce(
            state,
            &Event::HumanApproved {
                reviewer: "d".into(),
                reason: None,
            },
        );
        let state = reduce(
            state,
            &Event::Merged {
                commit: "abc".into(),
            },
        );
        assert_eq!(state.phase, Phase::Merged);
        assert_eq!(state.merged_commit.as_deref(), Some("abc"));
    }

    #[test]
    fn token_spend_accumulates() {
        let tokens = TokenUsage {
            input: 2,
            output: 3,
        };
        let state = created();
        let state = reduce(
            state,
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
                tokens,
            },
        );
        assert_eq!(
            state.token_spend,
            TokenUsage {
                input: 2,
                output: 3
            }
        );
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
                tokens: TokenUsage {
                    input: 5,
                    output: 1,
                },
            },
        );
        assert!(state.anomalies.is_empty(), "{:?}", state.anomalies);
        assert_eq!(state.phase, Phase::Verifying);
        assert_eq!(state.agent_invocations, 2);
        assert_eq!(state.agent_responses, 2);
        assert_eq!(
            state.token_spend,
            TokenUsage {
                input: 5,
                output: 1
            }
        );
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

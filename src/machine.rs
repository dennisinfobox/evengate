use serde::{Deserialize, Serialize};

use crate::event::{AgentRole, GateId, GateResult};
use crate::gate;
use crate::spec::{self, Spec};
use crate::state::{self, ArtifactRef, HumanDecision, Phase, State};

/// Retries allowed per deterministic gate before the machine asks for a human.
pub const MAX_GATE_ATTEMPTS: u32 = 2;

/// Prompt payload handed to the stub agent. Recorded indirectly through
/// `AgentInvoked::prompt_hash`, so it must serialize deterministically.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    pub project: String,
    pub title: String,
    pub goal: String,
    pub acceptance: Vec<String>,
    pub constraints: Vec<String>,
    pub non_goals: Vec<String>,
    pub context: Vec<String>,
    pub open_questions: Vec<String>,
}

impl Context {
    pub fn from_spec(spec: &Spec) -> Self {
        Context {
            project: spec::slug(&spec.title),
            title: spec.title.clone(),
            goal: spec.goal.clone(),
            acceptance: spec.acceptance.clone(),
            constraints: spec.constraints.clone(),
            non_goals: spec.non_goals.clone(),
            context: spec.context.clone(),
            open_questions: spec.open_questions.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Merged,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    InvokeAgent { role: AgentRole, context: Context },
    RunGate { gate: GateId },
    RequestHumanReview { artifact: ArtifactRef },
    OpenPullRequest { branch: String },
    AwaitHuman,
    Finalize { outcome: Outcome },
}

impl Command {
    /// Commands that intentionally produce no events and end the current run.
    pub fn is_awaiting(&self) -> bool {
        matches!(self, Command::AwaitHuman | Command::RequestHumanReview { .. })
    }
}

pub fn branch_name(spec: &Spec) -> String {
    format!("ev/{}", spec::slug(&spec.title))
}

/// Pure policy: no I/O, no time, no randomness. Progress is encoded entirely
/// in `State`, so the same state always yields the same command. Retries are
/// bounded by `MAX_GATE_ATTEMPTS`; there is no path that repeats a command
/// forever.
pub fn next_command(state: &State) -> Option<Command> {
    match state.phase {
        Phase::Drafting => {
            if state.agent_invocations == 0 {
                Some(Command::InvokeAgent {
                    role: AgentRole::Executor,
                    context: Context::from_spec(&state.spec),
                })
            } else {
                // An invocation was recorded but no response arrived: the log
                // is incomplete, so a human must decide how to continue.
                Some(Command::AwaitHuman)
            }
        }
        Phase::Generating => Some(Command::AwaitHuman),
        Phase::Verifying => {
            for gate_id in gate::MANDATORY_V0 {
                match state.gate_results.get(&gate_id) {
                    Some(GateResult::Pass) => continue,
                    Some(GateResult::NeedsHuman { .. }) => return Some(review_command()),
                    Some(GateResult::Fail { .. }) => {
                        let attempts = state.gate_attempts.get(&gate_id).copied().unwrap_or(0);
                        return if attempts >= MAX_GATE_ATTEMPTS {
                            Some(review_command())
                        } else {
                            Some(Command::RunGate { gate: gate_id })
                        };
                    }
                    None => return Some(Command::RunGate { gate: gate_id }),
                }
            }
            Some(Command::OpenPullRequest {
                branch: branch_name(&state.spec),
            })
        }
        Phase::Reviewing => match &state.human_decision {
            None => Some(Command::AwaitHuman),
            Some(HumanDecision::Approved { .. }) => {
                if state::all_mandatory_passed(state) {
                    Some(Command::Finalize {
                        outcome: Outcome::Merged,
                    })
                } else {
                    // Approval cannot bypass gates.
                    Some(review_command())
                }
            }
            Some(HumanDecision::Rejected { .. }) => Some(Command::Finalize {
                outcome: Outcome::Rejected,
            }),
        },
        Phase::Merged | Phase::Abandoned => None,
    }
}

fn review_command() -> Command {
    Command::RequestHumanReview {
        artifact: ArtifactRef {
            path: "project".into(),
            kind: "tree".into(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Event, GateId, GateResult};
    use crate::state::{initial, reduce};

    fn spec() -> Spec {
        Spec {
            title: "hello".into(),
            goal: "g".into(),
            acceptance: vec!["a".into()],
            constraints: vec!["c".into()],
            ..Spec::default()
        }
    }

    fn generated() -> State {
        let state = reduce(initial(), &Event::TaskCreated { spec: spec() });
        let state = reduce(
            state,
            &Event::AgentInvoked {
                role: AgentRole::Executor,
                prompt_hash: "0".into(),
            },
        );
        reduce(
            state,
            &Event::AgentResponded {
                role: AgentRole::Executor,
                response: "r".into(),
                tokens: Default::default(),
            },
        )
    }

    fn with_passes(mut state: State) -> State {
        for id in gate::MANDATORY_V0 {
            state = reduce(
                state,
                &Event::GateEvaluated {
                    gate: id,
                    result: GateResult::Pass,
                },
            );
        }
        state
    }

    #[test]
    fn fresh_task_invokes_the_agent() {
        let state = reduce(initial(), &Event::TaskCreated { spec: spec() });
        assert!(matches!(
            next_command(&state),
            Some(Command::InvokeAgent { .. })
        ));
    }

    #[test]
    fn gates_run_in_order() {
        let state = generated();
        assert_eq!(
            next_command(&state),
            Some(Command::RunGate {
                gate: GateId::Compiles
            })
        );
        let state = reduce(
            state,
            &Event::GateEvaluated {
                gate: GateId::Compiles,
                result: GateResult::Pass,
            },
        );
        assert_eq!(
            next_command(&state),
            Some(Command::RunGate {
                gate: GateId::TestsPass
            })
        );
    }

    #[test]
    fn failed_gate_retries_then_asks_a_human() {
        let mut state = generated();
        for _ in 0..MAX_GATE_ATTEMPTS {
            state = reduce(
                state,
                &Event::GateEvaluated {
                    gate: GateId::Compiles,
                    result: GateResult::Fail {
                        reason: "boom".into(),
                    },
                },
            );
        }
        assert!(matches!(
            next_command(&state),
            Some(Command::RequestHumanReview { .. })
        ));
    }

    #[test]
    fn all_gates_passed_opens_a_pull_request() {
        let state = with_passes(generated());
        assert_eq!(
            next_command(&state),
            Some(Command::OpenPullRequest {
                branch: "ev/hello".into()
            })
        );
    }

    #[test]
    fn approval_cannot_bypass_gates() {
        let state = generated();
        let state = reduce(state, &Event::PrOpened { url: "u".into() });
        let state = reduce(
            state,
            &Event::HumanApproved {
                reviewer: "d".into(),
            },
        );
        assert!(matches!(
            next_command(&state),
            Some(Command::RequestHumanReview { .. })
        ));
    }

    #[test]
    fn approved_with_gates_finalizes() {
        let state = with_passes(generated());
        let state = reduce(state, &Event::PrOpened { url: "u".into() });
        let state = reduce(state, &Event::HumanApproved { reviewer: "d".into() });
        assert_eq!(
            next_command(&state),
            Some(Command::Finalize {
                outcome: Outcome::Merged
            })
        );
    }

    #[test]
    fn merged_is_terminal() {
        let state = with_passes(generated());
        let state = reduce(state, &Event::PrOpened { url: "u".into() });
        let state = reduce(state, &Event::HumanApproved { reviewer: "d".into() });
        let state = reduce(state, &Event::Merged { commit: "abc".into() });
        assert_eq!(next_command(&state), None);
    }
}

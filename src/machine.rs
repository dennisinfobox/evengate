use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::event::{AgentRole, GateId, GateResult};
use crate::gate;
use crate::knowledge;
use crate::spec::{self, Spec};
use crate::state::{self, ArtifactRef, HumanDecision, Phase, RepeatedLesson, State};

/// Retries allowed per deterministic gate before the machine asks for a human.
pub const MAX_GATE_ATTEMPTS: u32 = 2;

/// Human-requested revisions allowed per task. Each revision re-invokes the
/// executor with the gate failures and resets the per-gate attempt counters.
pub const MAX_REVISIONS: u32 = 2;

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
    /// Gate failures and human guidance that the next revision must address.
    pub feedback: Vec<String>,
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
            feedback: Vec::new(),
        }
    }

    pub fn from_revision(
        spec: &Spec,
        failures: &BTreeMap<GateId, GateResult>,
        guidance: Option<&str>,
    ) -> Self {
        let mut context = Context::from_spec(spec);
        context.feedback = revision_feedback(failures, guidance);
        context
    }
}

fn revision_feedback(
    failures: &BTreeMap<GateId, GateResult>,
    guidance: Option<&str>,
) -> Vec<String> {
    let mut lines: Vec<String> = failures
        .iter()
        .filter_map(|(gate, result)| match result {
            GateResult::Fail { reason } => {
                Some(format!("{} failed: {reason}", gate::label(*gate)))
            }
            GateResult::NeedsHuman { question } => {
                Some(format!("{} needs attention: {question}", gate::label(*gate)))
            }
            GateResult::Pass => None,
        })
        .collect();
    if let Some(text) = guidance.map(str::trim).filter(|text| !text.is_empty()) {
        lines.push(format!("human guidance: {text}"));
    }
    lines
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
        Phase::Revising => {
            if state.revisions > MAX_REVISIONS {
                // Backstop: the CLI refuses more revisions, but a log with an
                // extra request must not loop the executor forever.
                Some(review_command())
            } else {
                Some(Command::InvokeAgent {
                    role: AgentRole::Executor,
                    context: Context::from_revision(
                        &state.spec,
                        &state.revision_failures,
                        state.revision_guidance.as_deref(),
                    ),
                })
            }
        }
        Phase::Verifying => {
            for gate_id in gate::MANDATORY_V0 {
                match state.gate_results.get(&gate_id) {
                    Some(GateResult::Pass) => continue,
                    Some(GateResult::NeedsHuman { .. }) => return Some(review_command()),
                    Some(GateResult::Fail { .. }) => {
                        let attempts = state.gate_attempts.get(&gate_id).copied().unwrap_or(0);
                        // A review failure is a verdict about the artifact, not
                        // a flaky check; re-asking the same tree only burns
                        // tokens. Stop for a human on the first rejection.
                        return if gate_id == GateId::ReviewAgent
                            || attempts >= MAX_GATE_ATTEMPTS
                        {
                            Some(review_command())
                        } else {
                            Some(Command::RunGate { gate: gate_id })
                        };
                    }
                    None => return Some(Command::RunGate { gate: gate_id }),
                }
            }
            if let Some(command) = promotion_command(state) {
                return Some(command);
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

/// Repeated lessons the human has not promoted or dismissed in this task.
pub fn pending_promotions(state: &State) -> Vec<(String, RepeatedLesson)> {
    state
        .repeated_lessons
        .iter()
        .filter(|(lesson, repeated)| {
            repeated.count >= knowledge::PROMOTION_THRESHOLD
                && !state.promotion_decisions.contains_key(*lesson)
        })
        .map(|(lesson, repeated)| (lesson.clone(), repeated.clone()))
        .collect()
}

fn promotion_command(state: &State) -> Option<Command> {
    let (_, repeated) = pending_promotions(state).into_iter().next()?;
    Some(Command::RequestHumanReview {
        artifact: ArtifactRef {
            path: format!("rules/{}.md", spec::slug(&state.spec.title)),
            kind: "promotion".into(),
            detail: Some(format!(
                "lesson repeated {} times: \"{}\"",
                repeated.count, repeated.text
            )),
        },
    })
}

fn review_command() -> Command {
    Command::RequestHumanReview {
        artifact: ArtifactRef {
            path: "project".into(),
            kind: "tree".into(),
            detail: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Event, GateId, GateResult};
    use crate::state::{initial, reduce, PromotionDecision};

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

    fn with_deterministic_passes(mut state: State) -> State {
        for id in [GateId::Compiles, GateId::TestsPass, GateId::LintsClean] {
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
    fn review_runs_after_the_deterministic_gates() {
        let state = with_deterministic_passes(generated());
        assert_eq!(
            next_command(&state),
            Some(Command::RunGate {
                gate: GateId::ReviewAgent
            })
        );
    }

    #[test]
    fn one_review_failure_stops_without_retrying() {
        let state = with_deterministic_passes(generated());
        let state = reduce(
            state,
            &Event::GateEvaluated {
                gate: GateId::ReviewAgent,
                result: GateResult::Fail {
                    reason: "missing tests".into(),
                },
            },
        );
        assert_eq!(state.gate_attempts[&GateId::ReviewAgent], 1);
        assert!(matches!(
            next_command(&state),
            Some(Command::RequestHumanReview { .. })
        ));
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

    fn blocked() -> State {
        let state = reduce(
            generated(),
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
    fn revision_reinvokes_the_executor_with_the_failures() {
        let state = reduce(
            blocked(),
            &Event::RevisionRequested {
                reviewer: "d".into(),
                guidance: Some("keep it small".into()),
            },
        );
        let Some(Command::InvokeAgent { role, context }) = next_command(&state) else {
            panic!("expected the executor to be invoked");
        };
        assert_eq!(role, AgentRole::Executor);
        assert_eq!(
            context.feedback,
            vec![
                "cargo check failed: boom".to_string(),
                "human guidance: keep it small".to_string()
            ]
        );
    }

    #[test]
    fn revision_invocation_returns_to_verification() {
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
        let state = reduce(
            state,
            &Event::AgentResponded {
                role: AgentRole::Executor,
                response: "r".into(),
                tokens: Default::default(),
            },
        );
        assert_eq!(
            next_command(&state),
            Some(Command::RunGate {
                gate: GateId::Compiles
            })
        );
    }

    #[test]
    fn revisions_are_bounded() {
        let mut state = reduce(
            blocked(),
            &Event::RevisionRequested {
                reviewer: "d".into(),
                guidance: None,
            },
        );
        state.revisions = MAX_REVISIONS + 1;
        assert!(matches!(
            next_command(&state),
            Some(Command::RequestHumanReview { .. })
        ));
    }

    fn with_repetition(state: State, lesson: &str, count: u32) -> State {
        reduce(
            state,
            &Event::LessonRepeated {
                lesson: lesson.into(),
                text: "Keep it small.".into(),
                count,
            },
        )
    }

    #[test]
    fn a_repeated_lesson_pauses_before_the_pull_request() {
        let state = with_repetition(with_passes(generated()), "keep it small.", 3);
        let Some(Command::RequestHumanReview { artifact }) = next_command(&state) else {
            panic!("expected a promotion review");
        };
        assert_eq!(artifact.kind, "promotion");
        assert_eq!(artifact.path, "rules/hello.md");
        let detail = artifact.detail.unwrap();
        assert!(detail.contains("repeated 3 times"), "{detail}");
        assert!(detail.contains("Keep it small."), "{detail}");
    }

    #[test]
    fn promotion_decisions_unblock_the_pull_request() {
        let base = with_repetition(with_passes(generated()), "keep it small.", 3);
        for decision in [PromotionDecision::Promoted, PromotionDecision::Dismissed] {
            let event = match decision {
                PromotionDecision::Promoted => Event::LessonPromoted {
                    reviewer: "dennis".into(),
                    lesson: "keep it small.".into(),
                },
                PromotionDecision::Dismissed => Event::LessonDismissed {
                    reviewer: "dennis".into(),
                    lesson: "keep it small.".into(),
                },
            };
            let state = reduce(base.clone(), &event);
            assert!(matches!(
                next_command(&state),
                Some(Command::OpenPullRequest { .. })
            ));
        }
    }

    #[test]
    fn a_lesson_below_the_threshold_does_not_pause() {
        let state = with_repetition(with_passes(generated()), "keep it small.", 2);
        assert!(matches!(
            next_command(&state),
            Some(Command::OpenPullRequest { .. })
        ));
    }

    #[test]
    fn promotions_are_asked_one_at_a_time_in_key_order() {
        let state = with_passes(generated());
        let state = reduce(
            state,
            &Event::LessonRepeated {
                lesson: "b lesson".into(),
                text: "B lesson".into(),
                count: 3,
            },
        );
        let state = reduce(
            state,
            &Event::LessonRepeated {
                lesson: "a lesson".into(),
                text: "A lesson".into(),
                count: 3,
            },
        );

        let Some(Command::RequestHumanReview { artifact }) = next_command(&state) else {
            panic!("expected the first promotion");
        };
        assert!(artifact.detail.unwrap().contains("\"A lesson\""));
        assert_eq!(
            pending_promotions(&state)
                .into_iter()
                .map(|(lesson, _)| lesson)
                .collect::<Vec<_>>(),
            vec!["a lesson".to_string(), "b lesson".to_string()]
        );

        let state = reduce(
            state,
            &Event::LessonDismissed {
                reviewer: "dennis".into(),
                lesson: "a lesson".into(),
            },
        );
        let Some(Command::RequestHumanReview { artifact }) = next_command(&state) else {
            panic!("expected the second promotion");
        };
        assert_eq!(
            artifact.detail.as_deref(),
            Some("lesson repeated 3 times: \"B lesson\"")
        );
    }

    #[test]
    fn abandoned_is_terminal() {
        let state = reduce(
            blocked(),
            &Event::HumanAbandoned {
                reviewer: "d".into(),
                reason: "not worth it".into(),
            },
        );
        assert_eq!(next_command(&state), None);
    }
}

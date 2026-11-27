use serde::Deserialize;

use crate::event::{GateId, GateResult};
use crate::sandbox::RunOutput;

/// Gates that must pass before a pull request may be opened in v0. The review
/// agent runs last, on the artifact the deterministic gates have accepted.
pub const MANDATORY_V0: [GateId; 4] = [
    GateId::Compiles,
    GateId::TestsPass,
    GateId::LintsClean,
    GateId::ReviewAgent,
];

const COMPILES: &[&str] = &["cargo", "check", "--locked"];
const TESTS_PASS: &[&str] = &["cargo", "test", "--locked"];
const LINTS_CLEAN: &[&str] = &["cargo", "clippy", "--locked", "--", "-D", "warnings"];

pub fn command(gate: GateId) -> Option<&'static [&'static str]> {
    match gate {
        GateId::Compiles => Some(COMPILES),
        GateId::TestsPass => Some(TESTS_PASS),
        GateId::LintsClean => Some(LINTS_CLEAN),
        GateId::ReviewAgent | GateId::HumanApproval => None,
    }
}

pub fn slug(gate: GateId) -> &'static str {
    match gate {
        GateId::Compiles => "compiles",
        GateId::TestsPass => "tests-pass",
        GateId::LintsClean => "lints-clean",
        GateId::ReviewAgent => "review-agent",
        GateId::HumanApproval => "human-approval",
    }
}

pub fn label(gate: GateId) -> &'static str {
    match gate {
        GateId::Compiles => "cargo check",
        GateId::TestsPass => "cargo test",
        GateId::LintsClean => "cargo clippy",
        GateId::ReviewAgent => "review agent",
        GateId::HumanApproval => "human approval",
    }
}

/// Deterministic verdict from a sandbox run. Only deterministic gates use this;
/// the review gate in later weeks is LLM-backed and recorded separately.
pub fn evaluate(gate: GateId, output: &RunOutput) -> GateResult {
    if output.timed_out {
        return GateResult::Fail {
            reason: format!("{}: sandbox timeout", label(gate)),
        };
    }
    if output.exit_code == 0 {
        return GateResult::Pass;
    }
    let detail = failure_detail(output);
    let reason = if detail.is_empty() {
        format!("{}: exit code {}", label(gate), output.exit_code)
    } else {
        format!("{}: {detail}", label(gate))
    };
    GateResult::Fail { reason }
}

/// Pick the most actionable line: compiler error code, test panic, clippy
/// warning, then any generic error. Cargo's `Compiling`/`Checking` lines are
/// useless as a revision prompt.
fn failure_detail(output: &RunOutput) -> String {
    let lines = || {
        output
            .stderr
            .lines()
            .chain(output.stdout.lines())
            .map(str::trim)
            .filter(|line| !line.is_empty())
    };
    let find = |predicate: fn(&str) -> bool| lines().find(|line| predicate(line));
    find(|line| line.starts_with("error["))
        .or_else(|| find(|line| line.contains("panicked at")))
        .or_else(|| find(|line| line.starts_with("warning:")))
        .or_else(|| find(|line| line.starts_with("error:")))
        .or_else(|| lines().next())
        .unwrap_or("")
        .to_string()
}

/// Structured verdict from the review model: approval, concrete issues, and
/// the lessons the next task should inherit. A malformed verdict is an error,
/// not an approval; the executor turns that into `NeedsHuman`.
#[derive(Deserialize)]
struct ReviewVerdict {
    approve: bool,
    #[serde(default)]
    issues: Vec<String>,
    #[serde(default)]
    lessons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewReport {
    pub result: GateResult,
    pub lessons: Vec<String>,
}

pub fn evaluate_review(text: &str) -> Result<ReviewReport, String> {
    let verdict: ReviewVerdict = serde_json::from_str(crate::llm::strip_fence(text))
        .map_err(|e| format!("review verdict is not JSON: {e}"))?;
    let result = if verdict.approve {
        GateResult::Pass
    } else if verdict.issues.is_empty() {
        GateResult::Fail {
            reason: "review agent withheld approval without listing an issue".to_string(),
        }
    } else {
        GateResult::Fail {
            reason: format!("review agent: {}", verdict.issues.join("; ")),
        }
    };
    Ok(ReviewReport {
        result,
        lessons: verdict.lessons,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(exit_code: i32, stderr: &str) -> RunOutput {
        RunOutput {
            exit_code,
            stdout: String::new(),
            stderr: stderr.to_string(),
            timed_out: false,
        }
    }

    #[test]
    fn exit_zero_passes() {
        assert_eq!(
            evaluate(GateId::Compiles, &output(0, "warning: something")),
            GateResult::Pass
        );
    }

    #[test]
    fn failure_carries_the_first_stderr_line() {
        let GateResult::Fail { reason } =
            evaluate(GateId::TestsPass, &output(101, "\nthread 'x' panicked\nnote"))
        else {
            panic!("expected fail");
        };
        assert_eq!(reason, "cargo test: thread 'x' panicked");
    }

    #[test]
    fn failure_detail_prefers_the_actual_diagnostic() {
        let out = output(
            101,
            "   Compiling word-count-cli v0.1.0 (/work)\nerror[E0433]: cannot find module `tempfile`\n",
        );
        let GateResult::Fail { reason } = evaluate(GateId::TestsPass, &out) else {
            panic!("expected fail");
        };
        assert_eq!(
            reason,
            "cargo test: error[E0433]: cannot find module `tempfile`"
        );
    }

    #[test]
    fn failure_detail_prefers_a_test_panic_over_cargos_error_line() {
        let mut out = output(101, "error: test failed, to rerun pass `--bin x`");
        out.stdout =
            "running 1 test\nfailures:\nthread 'x' panicked at src/main.rs:4:5".to_string();
        let GateResult::Fail { reason } = evaluate(GateId::TestsPass, &out) else {
            panic!("expected fail");
        };
        assert_eq!(
            reason,
            "cargo test: thread 'x' panicked at src/main.rs:4:5"
        );
    }

    #[test]
    fn failure_detail_uses_a_clippy_warning_when_there_is_no_error() {
        let out = output(
            101,
            "    Checking x v0.1.0 (/work)\nwarning: unused variable: `n`\n",
        );
        let GateResult::Fail { reason } = evaluate(GateId::LintsClean, &out) else {
            panic!("expected fail");
        };
        assert_eq!(reason, "cargo clippy: warning: unused variable: `n`");
    }

    #[test]
    fn timeout_fails() {
        let mut out = output(0, "");
        out.timed_out = true;
        assert!(matches!(evaluate(GateId::LintsClean, &out), GateResult::Fail { .. }));
    }

    #[test]
    fn review_gates_have_no_command() {
        assert!(command(GateId::ReviewAgent).is_none());
        assert!(command(GateId::HumanApproval).is_none());
    }

    #[test]
    fn review_approval_passes_even_when_fenced() {
        assert_eq!(
            evaluate_review("```json\n{\"approve\": true, \"issues\": []}\n```")
                .unwrap()
                .result,
            GateResult::Pass
        );
        assert_eq!(
            evaluate_review("{\"approve\": true, \"issues\": [\"nit\"]}")
                .unwrap()
                .result,
            GateResult::Pass
        );
    }

    #[test]
    fn review_rejection_carries_all_issues() {
        let GateResult::Fail { reason } =
            evaluate_review(r#"{"approve": false, "issues": ["no tests", "unsafe"]}"#)
                .unwrap()
                .result
        else {
            panic!("expected fail");
        };
        assert_eq!(reason, "review agent: no tests; unsafe");
    }

    #[test]
    fn review_without_issues_still_fails() {
        let GateResult::Fail { reason } = evaluate_review(r#"{"approve": false}"#)
            .unwrap()
            .result
        else {
            panic!("expected fail");
        };
        assert!(reason.contains("without listing an issue"), "{reason}");
    }

    #[test]
    fn review_keeps_the_lessons_for_the_next_task() {
        let report = evaluate_review(
            r#"{"approve": true, "issues": [], "lessons": ["write tests first", "keep it small"]}"#,
        )
        .unwrap();
        assert_eq!(report.result, GateResult::Pass);
        assert_eq!(report.lessons, vec!["write tests first", "keep it small"]);
        assert!(evaluate_review(r#"{"approve": true}"#)
            .unwrap()
            .lessons
            .is_empty());
    }

    #[test]
    fn malformed_review_is_an_error_not_an_approval() {
        assert!(evaluate_review("looks fine to me").is_err());
        assert!(evaluate_review(r#"{"issues": []}"#).is_err());
    }

    #[test]
    fn the_review_agent_is_the_last_mandatory_gate() {
        assert_eq!(*MANDATORY_V0.last().unwrap(), GateId::ReviewAgent);
    }
}

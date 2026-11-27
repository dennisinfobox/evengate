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
    let detail = output
        .stderr
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    let reason = if detail.is_empty() {
        format!("{}: exit code {}", label(gate), output.exit_code)
    } else {
        format!("{}: {detail}", label(gate))
    };
    GateResult::Fail { reason }
}

/// Structured verdict from the review model: approval plus concrete issues.
/// A malformed verdict is an error, not an approval; the executor turns that
/// into `NeedsHuman` so a bad answer can never pass the gate.
#[derive(Deserialize)]
struct ReviewVerdict {
    approve: bool,
    #[serde(default)]
    issues: Vec<String>,
}

pub fn evaluate_review(text: &str) -> Result<GateResult, String> {
    let verdict: ReviewVerdict = serde_json::from_str(crate::llm::strip_fence(text))
        .map_err(|e| format!("review verdict is not JSON: {e}"))?;
    if verdict.approve {
        return Ok(GateResult::Pass);
    }
    let reason = if verdict.issues.is_empty() {
        "review agent withheld approval without listing an issue".to_string()
    } else {
        format!("review agent: {}", verdict.issues.join("; "))
    };
    Ok(GateResult::Fail { reason })
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
            evaluate_review("```json\n{\"approve\": true, \"issues\": []}\n```").unwrap(),
            GateResult::Pass
        );
        assert_eq!(
            evaluate_review("{\"approve\": true, \"issues\": [\"nit\"]}").unwrap(),
            GateResult::Pass
        );
    }

    #[test]
    fn review_rejection_carries_all_issues() {
        let GateResult::Fail { reason } =
            evaluate_review(r#"{"approve": false, "issues": ["no tests", "unsafe"]}"#).unwrap()
        else {
            panic!("expected fail");
        };
        assert_eq!(reason, "review agent: no tests; unsafe");
    }

    #[test]
    fn review_without_issues_still_fails() {
        let GateResult::Fail { reason } =
            evaluate_review(r#"{"approve": false}"#).unwrap()
        else {
            panic!("expected fail");
        };
        assert!(reason.contains("without listing an issue"), "{reason}");
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

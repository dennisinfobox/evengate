use crate::event::{GateId, GateResult};
use crate::sandbox::RunOutput;

/// Gates that must pass before a pull request may be opened in v0.
pub const MANDATORY_V0: [GateId; 3] = [
    GateId::Compiles,
    GateId::TestsPass,
    GateId::LintsClean,
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
}

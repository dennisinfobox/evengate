use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as Process;

use crate::event::{AgentRole, Event, GateId, TokenUsage};
use crate::gate;
use crate::machine::{Command, Context, Outcome};
use crate::sandbox::{self, RunOutput, Sandbox};
use crate::spec::{self, Spec};

const COMMIT_NAME: &str = "Evengate";
const COMMIT_EMAIL: &str = "evengate@localhost";

/// Everything the impure boundary needs. The executor never mutates `State`.
pub struct ExecCtx {
    pub task_dir: PathBuf,
    pub project_name: String,
    pub title: String,
    pub templates_dir: PathBuf,
}

impl ExecCtx {
    pub fn new(task_dir: &Path, spec: &Spec) -> Self {
        let templates_dir = env::var("EVENGATE_TEMPLATES")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("templates/bin"));
        ExecCtx {
            task_dir: task_dir.to_path_buf(),
            project_name: spec::slug(&spec.title),
            title: spec.title.clone(),
            templates_dir,
        }
    }

    fn project_dir(&self) -> PathBuf {
        self.task_dir.join("project")
    }
}

/// Execute one command, performing its side effects and returning the events
/// that describe them. Events are the only channel back to the state machine.
pub fn run(command: &Command, ctx: &ExecCtx) -> Result<Vec<Event>, String> {
    match command {
        Command::InvokeAgent { role, context } => invoke_agent(*role, context, ctx),
        Command::RunGate { gate } => run_gate(*gate, ctx),
        Command::OpenPullRequest { branch } => open_pull_request(branch, ctx),
        Command::Finalize { outcome } => finalize(*outcome, ctx),
        Command::AwaitHuman | Command::RequestHumanReview { .. } => Ok(Vec::new()),
    }
}

fn invoke_agent(
    role: AgentRole,
    context: &Context,
    ctx: &ExecCtx,
) -> Result<Vec<Event>, String> {
    let project = ctx.project_dir();
    let files = materialize(&ctx.templates_dir, &project, &ctx.project_name)?;
    let prompt = serde_json::to_string(context).map_err(|e| format!("prompt encode: {e}"))?;
    let hash = fnv1a64(prompt.as_bytes());
    Ok(vec![
        Event::AgentInvoked {
            role,
            prompt_hash: format!("{hash:016x}"),
        },
        Event::AgentResponded {
            role,
            response: format!("stub agent copied {files} template file(s) into project/"),
            tokens: TokenUsage::default(),
        },
    ])
}

fn materialize(template: &Path, dst: &Path, crate_name: &str) -> Result<usize, String> {
    if !template.is_dir() {
        return Err(format!(
            "template directory not found: {} (set EVENGATE_TEMPLATES)",
            template.display()
        ));
    }
    if dst.exists() {
        fs::remove_dir_all(dst)
            .map_err(|e| format!("cannot clear {}: {e}", dst.display()))?;
    }
    let mut count = 0;
    copy_replacing(template, dst, crate_name, &mut count)?;
    Ok(count)
}

fn copy_replacing(
    src: &Path,
    dst: &Path,
    crate_name: &str,
    count: &mut usize,
) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| format!("cannot create {}: {e}", dst.display()))?;
    let entries = fs::read_dir(src).map_err(|e| format!("cannot read {}: {e}", src.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot read {}: {e}", src.display()))?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|e| format!("cannot stat {}: {e}", from.display()))?;
        if file_type.is_dir() {
            copy_replacing(&from, &to, crate_name, count)?;
        } else if file_type.is_file() {
            let text = fs::read_to_string(&from)
                .map_err(|e| format!("cannot read {}: {e}", from.display()))?;
            let rendered = text.replace("{{crate_name}}", crate_name);
            fs::write(&to, rendered)
                .map_err(|e| format!("cannot write {}: {e}", to.display()))?;
            *count += 1;
        }
    }
    Ok(())
}

fn run_gate(gate_id: GateId, ctx: &ExecCtx) -> Result<Vec<Event>, String> {
    let argv = gate::command(gate_id)
        .ok_or_else(|| format!("gate {} has no deterministic command in v0", gate::slug(gate_id)))?;
    let project = ctx.project_dir();
    if !project.is_dir() {
        return Err(format!(
            "project directory missing: {}",
            project.display()
        ));
    }
    let sandbox_dir = ctx
        .task_dir
        .join("artifacts")
        .join("sandboxes")
        .join(gate::slug(gate_id));
    sandbox::copy_tree(&project, &sandbox_dir)
        .map_err(|e| format!("cannot copy project into sandbox: {e}"))?;
    let output = Sandbox::from_env()
        .run(&sandbox_dir, argv)
        .map_err(|e| format!("sandbox failed: {e}"))?;
    let result = gate::evaluate(gate_id, &output);
    Ok(vec![
        Event::CommandExecuted {
            cmd: argv.join(" "),
            exit_code: output.exit_code,
            stdout: output.stdout,
            stderr: output.stderr,
        },
        Event::GateEvaluated {
            gate: gate_id,
            result,
        },
    ])
}

fn open_pull_request(branch: &str, ctx: &ExecCtx) -> Result<Vec<Event>, String> {
    let project = ctx.project_dir();
    if !project.is_dir() {
        return Err(format!(
            "project directory missing: {}",
            project.display()
        ));
    }
    let mut events = Vec::new();
    if !project.join(".git").exists() {
        git_step(&mut events, &project, &["init", "-q", "-b", "main"])?;
    }

    // A pull request needs a base commit on `main`. The first run creates an
    // empty one; the generated files belong to the feature branch only.
    let head = git_attempt(
        &mut events,
        &project,
        &["rev-parse", "--verify", "--quiet", "HEAD"],
    )?;
    if head.exit_code != 0 {
        let base = format!("ev: initialize {}", ctx.project_name);
        git_step(
            &mut events,
            &project,
            &[
                "-c",
                &format!("user.name={COMMIT_NAME}"),
                "-c",
                &format!("user.email={COMMIT_EMAIL}"),
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                &base,
            ],
        )?;
    }

    git_step(&mut events, &project, &["checkout", "-q", "-B", branch])?;
    git_step(&mut events, &project, &["add", "-A"])?;
    let message = format!("ev: {}", ctx.title);
    git_step(
        &mut events,
        &project,
        &[
            "-c",
            &format!("user.name={COMMIT_NAME}"),
            "-c",
            &format!("user.email={COMMIT_EMAIL}"),
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            &message,
        ],
    )?;

    let bare = std::path::absolute(ctx.task_dir.join("artifacts").join("remote.git"))
        .map_err(|e| format!("cannot resolve remote path: {e}"))?;
    let bare_path = bare
        .to_str()
        .ok_or_else(|| format!("non-UTF-8 path: {}", bare.display()))?
        .to_string();
    if !bare.exists() {
        if let Some(parent) = bare.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        git_step(&mut events, &project, &["init", "-q", "--bare", &bare_path])?;
    }
    git_step(&mut events, &project, &["push", "-q", &bare_path, "main"])?;
    git_step(&mut events, &project, &["push", "-q", &bare_path, branch])?;

    let url = format!("file:///{}#{}", bare_path.replace('\\', "/"), branch);
    events.push(Event::PrOpened { url });
    Ok(events)
}

fn finalize(outcome: Outcome, ctx: &ExecCtx) -> Result<Vec<Event>, String> {
    match outcome {
        Outcome::Merged => {
            let project = ctx.project_dir();
            let mut events = Vec::new();
            let head = git_step(&mut events, &project, &["rev-parse", "HEAD"])?;
            let commit = head.stdout.trim().to_string();
            if commit.is_empty() {
                return Err("git rev-parse HEAD produced no commit".into());
            }
            events.push(Event::Merged { commit });
            Ok(events)
        }
        Outcome::Rejected => Ok(vec![Event::TaskAbandoned {
            reason: "rejected by human".into(),
        }]),
    }
}

fn git_step(events: &mut Vec<Event>, dir: &Path, args: &[&str]) -> Result<RunOutput, String> {
    let output = git_attempt(events, dir, args)?;
    if output.exit_code != 0 {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            output.stderr.trim()
        ));
    }
    Ok(output)
}

/// Record a git invocation even when it fails; used for probes such as
/// `rev-parse --verify HEAD` on an unborn branch.
fn git_attempt(events: &mut Vec<Event>, dir: &Path, args: &[&str]) -> Result<RunOutput, String> {
    let output = Process::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .map_err(|e| format!("failed to run git: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let exit_code = output.status.code().unwrap_or(-1);
    events.push(Event::CommandExecuted {
        cmd: format!("git {}", args.join(" ")),
        exit_code,
        stdout: stdout.clone(),
        stderr: stderr.clone(),
    });
    Ok(RunOutput {
        exit_code,
        stdout,
        stderr,
        timed_out: false,
    })
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv_is_stable() {
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn context_serializes_deterministically() {
        let spec = Spec {
            title: "Hello There".into(),
            goal: "g".into(),
            acceptance: vec!["a".into()],
            ..Spec::default()
        };
        let context = Context::from_spec(&spec);
        assert_eq!(context.project, "hello-there");
        assert_eq!(
            serde_json::to_string(&context).unwrap(),
            r#"{"project":"hello-there","goal":"g","acceptance":["a"]}"#
        );
        assert_eq!(crate::machine::branch_name(&spec), "ev/hello-there");
    }

    #[test]
    fn materialize_replaces_the_crate_name() {
        let root = env::temp_dir().join(format!("evengate-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let template = root.join("template");
        let dst = root.join("out");
        fs::create_dir_all(template.join("src")).unwrap();
        fs::write(template.join("Cargo.toml"), "name = \"{{crate_name}}\"\n").unwrap();
        fs::write(template.join("src/main.rs"), "// {{crate_name}}\n").unwrap();
        let count = materialize(&template, &dst, "hello").unwrap();
        assert_eq!(count, 2);
        assert_eq!(
            fs::read_to_string(dst.join("Cargo.toml")).unwrap(),
            "name = \"hello\"\n"
        );
        assert_eq!(
            fs::read_to_string(dst.join("src/main.rs")).unwrap(),
            "// hello\n"
        );
        fs::remove_dir_all(&root).unwrap();
    }
}

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as Process;

use crate::event::{AgentRole, Event, GateId};
use crate::gate;
use crate::llm::{self, FileManifest, LiveClient, LlmClient, LlmRequest, RecordedClient, TemplateClient};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClientMode {
    Template,
    Live,
    Recorded,
}

fn client_mode() -> Result<ClientMode, String> {
    match env::var("EVENGATE_LLM") {
        Ok(value) => match value.trim() {
            "template" => Ok(ClientMode::Template),
            "live" => Ok(ClientMode::Live),
            "recorded" => Ok(ClientMode::Recorded),
            other => Err(format!(
                "EVENGATE_LLM must be template, live, or recorded; got `{other}`"
            )),
        },
        Err(_) => Ok(if env::var_os("EVENGATE_LLM_URL").is_some() {
            ClientMode::Live
        } else {
            ClientMode::Template
        }),
    }
}

fn client_for(ctx: &ExecCtx) -> Result<Box<dyn LlmClient>, String> {
    match client_mode()? {
        ClientMode::Template => Ok(Box::new(TemplateClient::new(
            ctx.templates_dir.clone(),
            ctx.project_name.clone(),
        ))),
        ClientMode::Live => Ok(Box::new(LiveClient::from_env()?)),
        ClientMode::Recorded => {
            let events = crate::log::read_events(&ctx.task_dir)
                .map_err(|e| format!("cannot read the event log: {e}"))?;
            Ok(Box::new(RecordedClient::from_events(&events)))
        }
    }
}

fn invoke_agent(
    role: AgentRole,
    context: &Context,
    ctx: &ExecCtx,
) -> Result<Vec<Event>, String> {
    let prompt = build_prompt(context, &ctx.templates_dir)?;
    let request = LlmRequest { role, prompt };
    let client = client_for(ctx)?;
    let response = client.complete(&request)?;
    let manifest = FileManifest::parse(&response.text)?;
    apply_manifest(&manifest, &ctx.project_dir())?;
    Ok(vec![
        Event::AgentInvoked {
            role,
            prompt_hash: request.prompt_hash(),
        },
        Event::AgentResponded {
            role,
            response: response.text,
            tokens: response.tokens,
        },
    ])
}

/// Deterministic prompt assembly. The template is included so a live agent
/// has the same source material the offline template client renders.
fn build_prompt(context: &Context, templates: &Path) -> Result<String, String> {
    let mut prompt = String::new();
    prompt.push_str("Build the complete project described below.\n");
    let _ = writeln!(prompt, "\n# Task\nTitle: {}", context.title);
    let _ = writeln!(prompt, "Project: {}", context.project);
    let _ = writeln!(prompt, "\n## Goal\n{}", context.goal.trim());
    push_list(&mut prompt, "Acceptance", &context.acceptance);
    push_list(&mut prompt, "Constraints", &context.constraints);
    push_list(&mut prompt, "Non-goals", &context.non_goals);
    push_list(&mut prompt, "Context", &context.context);
    push_list(&mut prompt, "Open questions", &context.open_questions);

    if templates.is_dir() {
        let files = llm::read_template_files(templates, &context.project)?;
        if !files.is_empty() {
            prompt.push_str(
                "\n# Reference template\nThe project starts from these files. Keep the structure unless the task requires otherwise.\n",
            );
            for (path, content) in files {
                let _ = writeln!(prompt, "\n--- {path} ---\n{content}");
            }
        }
    }

    prompt.push_str(
        "\n# Output\nReturn one JSON object and nothing else:\n\
         {\"files\": {\"relative/path\": \"file contents\"}}\n\
         Every path must be relative, use `/`, and contain no `..`. Include every file the project needs.\n",
    );
    Ok(prompt)
}

fn push_list(prompt: &mut String, title: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    let _ = writeln!(prompt, "\n## {title}");
    for item in items {
        let _ = writeln!(prompt, "- {item}");
    }
}

/// Replace `project/` with the files the agent returned. Validation happens
/// before the old tree is removed, so a bad manifest cannot destroy work.
fn apply_manifest(manifest: &FileManifest, dst: &Path) -> Result<usize, String> {
    manifest.validate()?;
    if dst.exists() {
        fs::remove_dir_all(dst).map_err(|e| format!("cannot clear {}: {e}", dst.display()))?;
    }
    fs::create_dir_all(dst).map_err(|e| format!("cannot create {}: {e}", dst.display()))?;
    for (path, content) in &manifest.files {
        let target = dst.join(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        fs::write(&target, content)
            .map_err(|e| format!("cannot write {}: {e}", target.display()))?;
    }
    Ok(manifest.files.len())
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

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
            r#"{"project":"hello-there","title":"Hello There","goal":"g","acceptance":["a"],"constraints":[],"non_goals":[],"context":[],"open_questions":[]}"#
        );
        assert_eq!(crate::machine::branch_name(&spec), "ev/hello-there");
    }

    #[test]
    fn prompt_carries_the_spec_and_the_template() {
        let root = env::temp_dir().join(format!("evengate-prompt-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let template = root.join("template");
        fs::create_dir_all(template.join("src")).unwrap();
        fs::write(template.join("Cargo.toml"), "name = \"{{crate_name}}\"\n").unwrap();
        fs::write(template.join("src/main.rs"), "fn main() {}\n").unwrap();

        let context = Context {
            project: "hello".into(),
            title: "Hello".into(),
            goal: "make a crate".into(),
            acceptance: vec!["it compiles".into()],
            constraints: vec!["no crates".into()],
            non_goals: Vec::new(),
            context: Vec::new(),
            open_questions: Vec::new(),
        };
        let prompt = build_prompt(&context, &template).unwrap();
        assert!(prompt.contains("make a crate"));
        assert!(prompt.contains("- it compiles"));
        assert!(prompt.contains("- no crates"));
        assert!(prompt.contains("--- src/main.rs ---\nfn main() {}"));
        assert!(prompt.contains("name = \"hello\""));
        assert!(prompt.contains("\"files\""));
        assert!(!prompt.contains("{{crate_name}}"));
        assert_eq!(prompt, build_prompt(&context, &template).unwrap());

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn apply_manifest_replaces_the_project_tree() {
        let root = env::temp_dir().join(format!("evengate-apply-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let project = root.join("project");
        fs::create_dir_all(project.join("old")).unwrap();
        fs::write(project.join("old/stale.txt"), "stale").unwrap();

        let mut files = BTreeMap::new();
        files.insert("Cargo.toml".to_string(), "name = \"hello\"\n".to_string());
        files.insert("src/main.rs".to_string(), "fn main() {}\n".to_string());
        let manifest = FileManifest { files };

        assert_eq!(apply_manifest(&manifest, &project).unwrap(), 2);
        assert!(!project.join("old").exists());
        assert_eq!(
            fs::read_to_string(project.join("src/main.rs")).unwrap(),
            "fn main() {}\n"
        );

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn apply_manifest_rejects_traversal_before_touching_the_tree() {
        let root = env::temp_dir().join(format!("evengate-traversal-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let project = root.join("project");
        fs::create_dir_all(&project).unwrap();
        fs::write(project.join("keep.txt"), "keep").unwrap();

        let mut files = BTreeMap::new();
        files.insert("../escape.txt".to_string(), "x".to_string());
        assert!(apply_manifest(&FileManifest { files }, &project).is_err());
        assert!(project.join("keep.txt").exists());
        assert!(!root.join("escape.txt").exists());

        fs::remove_dir_all(&root).unwrap();
    }
}

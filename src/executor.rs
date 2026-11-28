use std::collections::BTreeMap;
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as Process;

use crate::event::{AgentRole, Event, GateId, GateResult};
use crate::gate;
use crate::knowledge::{self, Lesson};
use crate::llm::{
    self, FileManifest, LiveClient, LlmClient, LlmRequest, RecordedClient, TemplateClient,
};
use crate::machine::{Command, Context, Outcome};
use crate::sandbox::{self, RunOutput, Sandbox};
use crate::spec::{self, Spec};

const COMMIT_NAME: &str = "Evengate";
const COMMIT_EMAIL: &str = "evengate@localhost";
const REVIEW_FILE_CAP: usize = 256 * 1024;
const REVIEW_SKIP_DIRS: [&str; 2] = [".git", "target"];

/// Directory holding `lessons/<task-id>.md`.
pub fn lessons_root() -> PathBuf {
    env::var("EVENGATE_LESSONS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("lessons"))
}

/// Directory holding project-wide `rules/<project>.md`.
pub fn rules_root() -> PathBuf {
    env::var("EVENGATE_RULES_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("rules"))
}

/// Everything the impure boundary needs. The executor never mutates `State`.
pub struct ExecCtx {
    pub task_dir: PathBuf,
    pub project_name: String,
    pub title: String,
    pub templates_dir: PathBuf,
    pub lessons_dir: PathBuf,
    pub rules_dir: PathBuf,
    task_id: String,
    self_edit: bool,
    context: Context,
}

impl ExecCtx {
    pub fn new(task_dir: &Path, spec: &Spec) -> Self {
        let templates_dir = env::var("EVENGATE_TEMPLATES")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("templates/bin"));
        let lessons_dir = lessons_root();
        let rules_dir = rules_root();
        let task_id = task_dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| spec::slug(&spec.title));
        ExecCtx {
            task_dir: task_dir.to_path_buf(),
            project_name: spec::slug(&spec.title),
            title: spec.title.clone(),
            templates_dir,
            lessons_dir,
            rules_dir,
            task_id,
            self_edit: spec.is_self_edit(),
            context: Context::from_spec(spec),
        }
    }

    fn project_dir(&self) -> PathBuf {
        self.task_dir.join("project")
    }

    /// A self-edit starts from the `SelfEditBase` recorded when the task was
    /// created, not from a template and not from the live tree.
    fn seed_self_edit(&self) -> Result<(), String> {
        let project = self.project_dir();
        if project.exists() {
            return Ok(());
        }
        let files = read_self_edit_base(&self.task_dir)?.ok_or_else(|| {
            format!(
                "self-edit task {} has no SelfEditBase event; recreate the task",
                self.task_dir.display()
            )
        })?;
        apply_manifest(&FileManifest { files }, &project, false)?;
        Ok(())
    }
}

/// Read the repository a self-edit starts from, excluding VCS metadata, build
/// output, and generated task artifacts. The task directory must live inside
/// the repository, so the caller's working directory never matters.
pub fn self_edit_base(task_dir: &Path, spec: &Spec) -> Result<BTreeMap<String, String>, String> {
    let repo = repo_root(task_dir)?;
    let files = read_repository_files(&repo)?;
    for target in &spec.target {
        if !target_matches(&files, target) {
            return Err(format!(
                "self-edit target does not exist in the repository: {target}"
            ));
        }
    }
    Ok(files)
}

fn read_self_edit_base(task_dir: &Path) -> Result<Option<BTreeMap<String, String>>, String> {
    let events =
        crate::log::read_events(task_dir).map_err(|e| format!("cannot read the event log: {e}"))?;
    Ok(events.into_iter().find_map(|event| match event {
        Event::SelfEditBase { files } => Some(files),
        _ => None,
    }))
}

fn repo_root(task_dir: &Path) -> Result<PathBuf, String> {
    for dir in task_dir.ancestors() {
        // A relative task path ends in the empty ancestor, which `read_dir`
        // cannot open; the current directory is the same repository root.
        let dir = if dir.as_os_str().is_empty() {
            Path::new(".")
        } else {
            dir
        };
        if dir.join(".git").exists() && dir.join("Cargo.toml").is_file() && dir.join("src").is_dir()
        {
            return Ok(dir.to_path_buf());
        }
    }
    Err(format!(
        "self-edit task {} is not inside the Evengate repository",
        task_dir.display()
    ))
}

/// A self-edit with exactly one file target can return that file directly,
/// which avoids JSON escaping of Rust attributes. A directory target or
/// several targets keep the manifest protocol.
fn single_file_target(context: &Context, files: &BTreeMap<String, String>) -> Option<String> {
    if context.target.len() != 1 {
        return None;
    }
    let clean = context.target[0].trim().replace('\\', "/");
    let clean = clean.trim_end_matches('/');
    if files.contains_key(clean) {
        Some(clean.to_string())
    } else {
        None
    }
}

fn parse_executor_response(text: &str, raw_target: Option<&str>) -> Result<FileManifest, String> {
    if let Some(path) = raw_target {
        let content = format!("{}\n", llm::strip_fence(text).trim_end());
        if content.trim().is_empty() {
            return Err(format!("agent returned an empty `{path}`"));
        }
        let mut files = BTreeMap::new();
        files.insert(path.to_string(), content);
        return Ok(FileManifest { files });
    }
    FileManifest::parse(text)
}

fn target_covers(target: &str, path: &str) -> bool {
    let clean = target.trim().replace('\\', "/");
    let clean = clean.trim_end_matches('/');
    !clean.is_empty() && (path == clean || path.starts_with(&format!("{clean}/")))
}

fn target_covers_any(targets: &[String], path: &str) -> bool {
    targets.is_empty() || targets.iter().any(|target| target_covers(target, path))
}

fn target_matches(files: &BTreeMap<String, String>, target: &str) -> bool {
    files.keys().any(|path| target_covers(target, path))
}

const SELF_EDIT_SKIP_DIRS: [&str; 3] = [".git", "target", ".tmp"];

fn skip_self_edit_entry(name: &str, rel: &Path) -> bool {
    SELF_EDIT_SKIP_DIRS.contains(&name)
        || (matches!(name, "project" | "artifacts")
            && rel.starts_with("tasks")
            && rel.components().count() == 2)
}

fn read_repository_files(root: &Path) -> Result<BTreeMap<String, String>, String> {
    let mut files = BTreeMap::new();
    let mut total = 0usize;
    read_repository_entries(root, Path::new(""), &mut files, &mut total)?;
    Ok(files)
}

fn read_repository_entries(
    src: &Path,
    rel: &Path,
    files: &mut BTreeMap<String, String>,
    total: &mut usize,
) -> Result<(), String> {
    let mut entries = fs::read_dir(src)
        .map_err(|e| format!("cannot read {}: {e}", src.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("cannot read {}: {e}", src.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        if skip_self_edit_entry(&name, rel) {
            continue;
        }
        let from = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|e| format!("cannot stat {}: {e}", from.display()))?;
        if file_type.is_dir() {
            read_repository_entries(&from, &rel.join(&file_name), files, total)?;
        } else if file_type.is_file() {
            let text = fs::read_to_string(&from).map_err(|e| {
                format!(
                    "self-edit base entry is not readable text: {}: {e}",
                    from.display()
                )
            })?;
            *total = total
                .checked_add(text.len())
                .ok_or("self-edit base size overflow")?;
            if text.len() > llm::MAX_FILE_BYTES || *total > llm::MAX_TOTAL_BYTES {
                return Err("self-edit base exceeds the manifest size limits".into());
            }
            files.insert(
                rel.join(&file_name).to_string_lossy().replace('\\', "/"),
                text,
            );
        } else {
            return Err(format!(
                "repository entry is not a file or directory: {}",
                from.display()
            ));
        }
    }
    Ok(())
}

/// Copy the repository without VCS metadata, build output, or generated task
/// artifacts. Used to restore the edited tree after the base commit.
fn copy_self_edit_tree(src: &Path, dst: &Path) -> Result<(), String> {
    copy_self_edit_entries(src, dst, Path::new(""))
}

fn copy_self_edit_entries(src: &Path, dst: &Path, rel: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| format!("cannot create {}: {e}", dst.display()))?;
    let mut entries = fs::read_dir(src)
        .map_err(|e| format!("cannot read {}: {e}", src.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("cannot read {}: {e}", src.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        if skip_self_edit_entry(&name, rel) {
            continue;
        }
        let from = entry.path();
        let to = dst.join(&file_name);
        let file_type = entry
            .file_type()
            .map_err(|e| format!("cannot stat {}: {e}", from.display()))?;
        if file_type.is_dir() {
            copy_self_edit_entries(&from, &to, &rel.join(&file_name))?;
        } else if file_type.is_file() {
            fs::copy(&from, &to)
                .map_err(|e| format!("cannot copy {} to {}: {e}", from.display(), to.display()))?;
        } else {
            return Err(format!(
                "repository entry is not a file or directory: {}",
                from.display()
            ));
        }
    }
    Ok(())
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

/// What `materialize` rebuilt from the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaterializeReport {
    pub responses: usize,
    pub files: usize,
    pub lesson: bool,
}

/// Rebuild the gitignored derivatives of a task from its event log: the
/// last executor manifest becomes `project/`, and the last usable reviewer
/// verdict becomes `lessons/<task-id>.md`. No model, network, or git.
pub fn materialize(task_dir: &Path, lessons_dir: &Path) -> Result<MaterializeReport, String> {
    let events =
        crate::log::read_events(task_dir).map_err(|e| format!("cannot read the event log: {e}"))?;
    if events.is_empty() {
        return Err(format!(
            "no events to materialize in {}",
            task_dir.display()
        ));
    }
    let state = events
        .iter()
        .fold(crate::state::initial(), crate::state::reduce);
    if !state.task_created {
        return Err("the event log has no TaskCreated event".into());
    }
    let ctx = ExecCtx::new(task_dir, &state.spec);

    let mut report = MaterializeReport {
        responses: 0,
        files: 0,
        lesson: false,
    };
    let base = if ctx.self_edit {
        let files = events
            .iter()
            .find_map(|event| match event {
                Event::SelfEditBase { files } => Some(files.clone()),
                _ => None,
            })
            .ok_or("self-edit log has no SelfEditBase event")?;
        report.files += apply_manifest(
            &FileManifest {
                files: files.clone(),
            },
            &ctx.project_dir(),
            false,
        )?;
        Some(files)
    } else {
        None
    };
    let raw_target = base
        .as_ref()
        .and_then(|files| single_file_target(&ctx.context, files));
    for event in &events {
        if let Event::AgentResponded {
            role: AgentRole::Executor,
            response,
            ..
        } = event
        {
            let manifest = parse_executor_response(response, raw_target.as_deref())?;
            report.files += apply_manifest(&manifest, &ctx.project_dir(), ctx.self_edit)?;
            report.responses += 1;
        }
    }
    if report.responses == 0 {
        return Err("the event log has no executor response to materialize".into());
    }

    for event in &events {
        if let Event::AgentResponded {
            role: AgentRole::Reviewer,
            response,
            ..
        } = event
        {
            if let Ok(verdict) = gate::evaluate_review(response) {
                if !verdict.lessons.is_empty() {
                    knowledge::write_lesson(
                        lessons_dir,
                        &ctx.task_id,
                        &ctx.title,
                        &verdict.lessons,
                    )?;
                    report.lesson = true;
                }
            }
        }
    }
    Ok(report)
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

fn client_for(ctx: &ExecCtx, role: AgentRole) -> Result<Box<dyn LlmClient>, String> {
    match client_mode()? {
        ClientMode::Template if role != AgentRole::Reviewer && ctx.self_edit => Err(
            "self-edit tasks require a model: set EVENGATE_LLM_URL (the offline template agent cannot edit the repository)"
                .into(),
        ),
        ClientMode::Template if role != AgentRole::Reviewer => Ok(Box::new(TemplateClient::new(
            ctx.templates_dir.clone(),
            ctx.project_name.clone(),
        ))),
        ClientMode::Template => Err(
            "the review gate requires a model: set EVENGATE_LLM_URL (or EVENGATE_LLM=recorded with a recorded verdict)"
                .into(),
        ),
        ClientMode::Live => Ok(Box::new(LiveClient::from_env()?)),
        ClientMode::Recorded => {
            let events = crate::log::read_events(&ctx.task_dir)
                .map_err(|e| format!("cannot read the event log: {e}"))?;
            Ok(Box::new(RecordedClient::from_events(&events)))
        }
    }
}

fn invoke_agent(role: AgentRole, context: &Context, ctx: &ExecCtx) -> Result<Vec<Event>, String> {
    let client = client_for(ctx, role)?;
    let lessons = knowledge::recent_lessons(&ctx.lessons_dir, knowledge::RECENT_LESSONS)?;
    let rules = knowledge::read_rules(&ctx.rules_dir, &ctx.project_name)?;
    let (current, repository_files, single_file) = if ctx.self_edit {
        ctx.seed_self_edit()?;
        let all = read_project_files(&ctx.project_dir())?;
        let selected = all
            .iter()
            .filter(|(path, _)| target_covers_any(&context.target, path))
            .map(|(path, content)| (path.clone(), content.clone()))
            .collect();
        let single = single_file_target(context, &all);
        (Some(selected), all.keys().cloned().collect(), single)
    } else if !context.feedback.is_empty() {
        (
            Some(read_project_files(&ctx.project_dir())?),
            Vec::new(),
            None,
        )
    } else {
        (None, Vec::new(), None)
    };
    let prompt = build_prompt(
        context,
        &ctx.templates_dir,
        &lessons,
        rules.as_deref(),
        current.as_ref(),
        &repository_files,
        single_file.as_deref(),
    )?;
    let request = LlmRequest { role, prompt };
    let response = client.complete(&request)?;
    let manifest = parse_executor_response(&response.text, single_file.as_deref())?;
    apply_manifest(&manifest, &ctx.project_dir(), ctx.self_edit)?;
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

/// Deterministic prompt assembly. A first build sees the template; a revision
/// sees the tree the gates judged instead.
fn build_prompt(
    context: &Context,
    templates: &Path,
    lessons: &[Lesson],
    rules: Option<&str>,
    current: Option<&BTreeMap<String, String>>,
    repository_files: &[String],
    raw_target: Option<&str>,
) -> Result<String, String> {
    let self_edit = context.target.iter().any(|path| spec::targets_self(path));
    let mut prompt = String::new();
    if context.feedback.is_empty() {
        if self_edit {
            prompt.push_str(
                "Edit the existing Evengate project described below. Keep every file the task does not change.\n",
            );
        } else {
            prompt.push_str("Build the complete project described below.\n");
        }
    } else {
        prompt.push_str(
            "Revise the existing project described below. Every issue under Revision feedback must be fixed.\n",
        );
    }
    push_task_spec(&mut prompt, context);
    prompt.push_str(&knowledge::rules_section(rules));
    push_list(&mut prompt, "Revision feedback", &context.feedback);
    prompt.push_str(&knowledge::prompt_section(lessons));

    if self_edit && !repository_files.is_empty() {
        prompt.push_str("\n# Repository files\n");
        for path in repository_files {
            let _ = writeln!(prompt, "- {path}");
        }
    }

    if let Some(files) = current {
        if self_edit {
            prompt.push_str(
                "\n# Files to edit\nThese are the files named under Target. Return their new contents, or no file when no change is needed.\n",
            );
        } else {
            prompt.push_str(
                "\n# Current project\nThese are the files the gates judged. Edit them into the answer instead of starting over.\n",
            );
        }
        for (path, content) in files {
            let _ = writeln!(prompt, "\n--- {path} ---\n{content}");
        }
    } else if templates.is_dir() {
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

    if let Some(target) = raw_target {
        let _ = writeln!(
            prompt,
            "\n# Output\nReturn the complete contents of `{target}` and nothing else. Do not wrap the file in JSON, Markdown fences, or commentary. End the file with a newline."
        );
    } else {
        prompt.push_str(
            "\n# Output\nReturn one JSON object and nothing else:\n\
             {\"files\": {\"relative/path\": \"file contents\"}}\n\
             Every path must be relative, use `/`, and contain no `..`. Include every file the project needs.\n",
        );
        if self_edit {
            prompt.push_str(
                "Return only the files you changed. Every unlisted path stays as it is; this task cannot delete files.\n",
            );
        }
    }
    Ok(prompt)
}

/// The reviewer sees the real artifact the deterministic gates approved, not
/// the executing agent's summary of it.
fn build_review_prompt(
    context: &Context,
    project: &Path,
    rules: Option<&str>,
    lessons: &[Lesson],
    base: Option<&BTreeMap<String, String>>,
) -> Result<String, String> {
    let files = read_project_files(project)?;
    let mut prompt = if base.is_some() {
        String::from("Review the change below before it is offered to the human owner.\n")
    } else {
        String::from("Review the complete project below before it is offered to the human owner.\n")
    };
    push_task_spec(&mut prompt, context);
    prompt.push_str(&knowledge::rules_section(rules));
    prompt.push_str(&knowledge::prompt_section(lessons));
    if let Some(base) = base {
        prompt.push_str("\n# Repository files\n");
        for path in files.keys() {
            let _ = writeln!(prompt, "- {path}");
        }
        let changed: Vec<(&String, &String)> = files
            .iter()
            .filter(|(path, content)| base.get(*path) != Some(*content))
            .collect();
        prompt.push_str("\n# Changed files\n");
        if changed.is_empty() {
            prompt.push_str("(none)\n");
        }
        for (path, content) in changed {
            let _ = writeln!(prompt, "\n--- {path} ---\n{content}");
        }
        let deleted: Vec<&String> = base
            .keys()
            .filter(|path| !files.contains_key(*path))
            .collect();
        if !deleted.is_empty() {
            prompt.push_str("\n# Deleted files\n");
            for path in deleted {
                let _ = writeln!(prompt, "- {path}");
            }
        }
    } else {
        prompt.push_str("\n# Project files\n");
        for (path, content) in &files {
            let _ = writeln!(prompt, "\n--- {path} ---\n{content}");
        }
    }
    prompt.push_str(
        "\n# Output\nReturn one JSON object and nothing else:\n\
         {\"approve\": true, \"issues\": [], \"lessons\": [\"...\"]}\n\
         Set `approve` to false and list concrete, actionable issues when the project does not satisfy its acceptance criteria or violates a constraint. Review the code, not the process.\n\
         Add 3–5 `lessons` bullets for the next task: what went wrong, or what to keep doing. Each lesson must be concrete and short. When one of the earlier lessons still applies to this artifact, copy that lesson's text exactly; otherwise write a new lesson.\n",
    );
    Ok(prompt)
}

fn push_task_spec(prompt: &mut String, context: &Context) {
    let _ = writeln!(prompt, "\n# Task\nTitle: {}", context.title);
    let _ = writeln!(prompt, "Project: {}", context.project);
    let _ = writeln!(prompt, "\n## Goal\n{}", context.goal.trim());
    push_list(prompt, "Target", &context.target);
    push_list(prompt, "Acceptance", &context.acceptance);
    push_list(prompt, "Constraints", &context.constraints);
    push_list(prompt, "Non-goals", &context.non_goals);
    push_list(prompt, "Context", &context.context);
    push_list(prompt, "Open questions", &context.open_questions);
}

/// Text files of the project in sorted path order. VCS and build output are
/// excluded so the review prompt is stable across runs.
fn read_project_files(project: &Path) -> Result<BTreeMap<String, String>, String> {
    if !project.is_dir() {
        return Err(format!("project directory missing: {}", project.display()));
    }
    let mut files = BTreeMap::new();
    collect_project(project, Path::new(""), &mut files)?;
    Ok(files)
}

fn collect_project(
    dir: &Path,
    rel: &Path,
    files: &mut BTreeMap<String, String>,
) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    let mut entries = entries
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let file_name = entry.file_name();
        let file_type = entry
            .file_type()
            .map_err(|e| format!("cannot stat {}: {e}", path.display()))?;
        if file_type.is_dir() {
            if REVIEW_SKIP_DIRS.contains(&file_name.to_string_lossy().as_ref()) {
                continue;
            }
            collect_project(&path, &rel.join(&file_name), files)?;
        } else if file_type.is_file() {
            let text = fs::read_to_string(&path)
                .map_err(|e| format!("cannot read {} for review: {e}", path.display()))?;
            if text.len() > REVIEW_FILE_CAP {
                return Err(format!(
                    "review input too large: {} is {} bytes (max {REVIEW_FILE_CAP})",
                    path.display(),
                    text.len()
                ));
            }
            files.insert(
                rel.join(&file_name).to_string_lossy().replace('\\', "/"),
                text,
            );
        } else {
            return Err(format!(
                "project entry is not a file or directory: {}",
                path.display()
            ));
        }
    }
    Ok(())
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

/// Write a manifest into `project/`. A generated project is replaced
/// wholesale; a self-edit overlays, so unlisted files survive. Validation
/// happens first, so a bad manifest cannot destroy work.
fn apply_manifest(manifest: &FileManifest, dst: &Path, overlay: bool) -> Result<usize, String> {
    manifest.validate()?;
    if !overlay && dst.exists() {
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
    if gate_id == GateId::ReviewAgent {
        return run_review_gate(ctx);
    }
    let argv = gate::command(gate_id).ok_or_else(|| {
        format!(
            "gate {} has no deterministic command in v0",
            gate::slug(gate_id)
        )
    })?;
    let project = ctx.project_dir();
    if !project.is_dir() {
        return Err(format!("project directory missing: {}", project.display()));
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

/// LLM-backed gate. The model's raw verdict is recorded as a `Reviewer`
/// response; an unusable answer becomes `NeedsHuman`, never a pass.
fn run_review_gate(ctx: &ExecCtx) -> Result<Vec<Event>, String> {
    let rules = knowledge::read_rules(&ctx.rules_dir, &ctx.project_name)?;
    let lessons = knowledge::recent_lessons(&ctx.lessons_dir, knowledge::RECENT_LESSONS)?;
    let base = if ctx.self_edit {
        read_self_edit_base(&ctx.task_dir)?
    } else {
        None
    };
    let prompt = build_review_prompt(
        &ctx.context,
        &ctx.project_dir(),
        rules.as_deref(),
        &lessons,
        base.as_ref(),
    )?;
    let request = LlmRequest {
        role: AgentRole::Reviewer,
        prompt,
    };
    let client = client_for(ctx, AgentRole::Reviewer)?;
    let response = client.complete(&request)?;
    let report = match gate::evaluate_review(&response.text) {
        Ok(report) => report,
        Err(error) => gate::ReviewReport {
            result: GateResult::NeedsHuman {
                question: format!("review agent returned an unusable verdict: {error}"),
            },
            lessons: Vec::new(),
        },
    };
    let wrote_lesson = !report.lessons.is_empty();
    if wrote_lesson {
        knowledge::write_lesson(&ctx.lessons_dir, &ctx.task_id, &ctx.title, &report.lessons)?;
    }
    let mut events = vec![
        Event::AgentInvoked {
            role: AgentRole::Reviewer,
            prompt_hash: request.prompt_hash(),
        },
        Event::AgentResponded {
            role: AgentRole::Reviewer,
            response: response.text,
            tokens: response.tokens,
        },
        Event::GateEvaluated {
            gate: GateId::ReviewAgent,
            result: report.result,
        },
    ];
    if wrote_lesson {
        events.extend(repetition_events(
            &ctx.lessons_dir,
            &ctx.rules_dir,
            &ctx.project_name,
        )?);
    }
    Ok(events)
}

/// Lessons seen in enough tasks to be promoted. Lessons already present in
/// the project's rules file are not proposed again.
fn repetition_events(
    lessons_dir: &Path,
    rules_dir: &Path,
    project: &str,
) -> Result<Vec<Event>, String> {
    let counts = knowledge::count_lessons(lessons_dir)?;
    let rules = knowledge::read_rules(rules_dir, project)?;
    let known = knowledge::rule_lessons(rules.as_deref());
    Ok(counts
        .into_iter()
        .filter(|(lesson, counted)| {
            counted.count >= knowledge::PROMOTION_THRESHOLD
                && !known
                    .iter()
                    .any(|rule| knowledge::lessons_match(rule, lesson))
        })
        .map(|(lesson, counted)| Event::LessonRepeated {
            lesson,
            text: counted.text,
            count: counted.count,
        })
        .collect())
}

/// Build `main` from the recorded base tree, then restore the edited tree for
/// the feature branch. The human sees a diff, not a repository-sized addition.
fn prepare_self_edit_base(
    events: &mut Vec<Event>,
    ctx: &ExecCtx,
    project: &Path,
) -> Result<(), String> {
    let files = read_self_edit_base(&ctx.task_dir)?
        .ok_or("self-edit log has no SelfEditBase event; recreate the task")?;
    let stash = ctx.task_dir.join("artifacts").join("self-edit-edited");
    if stash.exists() {
        fs::remove_dir_all(&stash).map_err(|e| format!("cannot clear {}: {e}", stash.display()))?;
    }
    if let Some(parent) = stash.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    fs::rename(project, &stash).map_err(|e| {
        format!(
            "cannot move {} aside to {}: {e}",
            project.display(),
            stash.display()
        )
    })?;

    apply_manifest(&FileManifest { files }, project, false)?;
    git_step(events, project, &["init", "-q", "-b", "main"])?;
    git_step(events, project, &["add", "-A"])?;
    let base = format!("ev: initialize {}", ctx.project_name);
    git_step(
        events,
        project,
        &[
            "-c",
            &format!("user.name={COMMIT_NAME}"),
            "-c",
            &format!("user.email={COMMIT_EMAIL}"),
            "commit",
            "-q",
            "-m",
            &base,
        ],
    )?;

    clear_worktree(project)?;
    copy_self_edit_tree(&stash, project)?;
    fs::remove_dir_all(&stash).map_err(|e| format!("cannot remove {}: {e}", stash.display()))?;
    Ok(())
}

fn clear_worktree(project: &Path) -> Result<(), String> {
    let entries = fs::read_dir(project)
        .map_err(|e| format!("cannot read {}: {e}", project.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("cannot read {}: {e}", project.display()))?;
    for entry in entries {
        if entry.file_name() == ".git" {
            continue;
        }
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|e| format!("cannot stat {}: {e}", path.display()))?;
        let result = if file_type.is_dir() {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
        result.map_err(|e| format!("cannot remove {}: {e}", path.display()))?;
    }
    Ok(())
}

fn open_pull_request(branch: &str, ctx: &ExecCtx) -> Result<Vec<Event>, String> {
    let project = ctx.project_dir();
    if !project.is_dir() {
        return Err(format!("project directory missing: {}", project.display()));
    }
    let mut events = Vec::new();
    if !project.join(".git").exists() {
        if ctx.self_edit {
            prepare_self_edit_base(&mut events, ctx, &project)?;
        } else {
            git_step(&mut events, &project, &["init", "-q", "-b", "main"])?;
        }
    }

    // A pull request needs a base commit on `main`. A generated project gets an
    // empty one; a self-edit gets the pristine base tree, so the branch diff
    // contains only the change.
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
    use crate::log;
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
            r#"{"project":"hello-there","title":"Hello There","goal":"g","acceptance":["a"],"constraints":[],"non_goals":[],"context":[],"open_questions":[],"feedback":[]}"#
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
            target: Vec::new(),
            context: Vec::new(),
            open_questions: Vec::new(),
            feedback: Vec::new(),
        };
        let prompt = build_prompt(&context, &template, &[], None, None, &[], None).unwrap();
        assert!(prompt.contains("make a crate"));
        assert!(prompt.contains("- it compiles"));
        assert!(prompt.contains("- no crates"));
        assert!(prompt.contains("--- src/main.rs ---\nfn main() {}"));
        assert!(prompt.contains("name = \"hello\""));
        assert!(prompt.contains("\"files\""));
        assert!(!prompt.contains("{{crate_name}}"));
        assert_eq!(
            prompt,
            build_prompt(&context, &template, &[], None, None, &[], None).unwrap()
        );

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn self_edit_base_records_the_repository_without_generated_trees() {
        let root = env::temp_dir().join(format!("evengate-self-edit-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for dir in [
            ".git",
            "src",
            "target/debug",
            "tasks/keep/artifacts",
            "tasks/keep/project",
            "tasks/self-edit",
        ] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        fs::write(root.join("Cargo.toml"), "[package]\nname = \"evengate\"\n").unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(root.join("target/debug/junk"), "junk").unwrap();
        fs::write(root.join("tasks/keep/events.jsonl"), "{}\n").unwrap();
        fs::write(root.join("tasks/keep/artifacts/junk"), "junk").unwrap();
        fs::write(root.join("tasks/keep/project/junk"), "junk").unwrap();

        let spec = Spec {
            title: "Edit the gate".into(),
            target: vec!["src/main.rs".into()],
            ..Spec::default()
        };
        let task = root.join("tasks/self-edit");
        let base = self_edit_base(&task, &spec).unwrap();
        assert_eq!(base.get("src/main.rs").unwrap(), "fn main() {}\n");
        assert!(base.contains_key("Cargo.toml"));
        assert!(!base.contains_key("target/debug/junk"));
        assert!(!base.contains_key("tasks/keep/artifacts/junk"));
        assert!(!base.contains_key("tasks/keep/project/junk"));
        assert!(base.contains_key("tasks/keep/events.jsonl"));

        log::append_event(
            &task,
            &Event::TaskCreated {
                spec: spec.clone(),
                gates: gate::MANDATORY_V0.to_vec(),
            },
        )
        .unwrap();
        log::append_event(&task, &Event::SelfEditBase { files: base }).unwrap();

        let ctx = ExecCtx::new(&task, &spec);
        ctx.seed_self_edit().unwrap();
        let project = task.join("project");
        assert_eq!(
            fs::read_to_string(project.join("src/main.rs")).unwrap(),
            "fn main() {}\n"
        );
        assert!(project.join("Cargo.toml").is_file());
        assert!(!project.join(".git").exists());
        assert!(!project.join("target").exists());

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn self_edit_rejects_a_missing_target() {
        let root =
            env::temp_dir().join(format!("evengate-self-edit-missing-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("tasks/self-edit")).unwrap();
        fs::write(root.join("Cargo.toml"), "[package]\nname = \"evengate\"\n").unwrap();

        let spec = Spec {
            title: "Edit the gate".into(),
            target: vec!["src/no-such.rs".into()],
            ..Spec::default()
        };
        let error = self_edit_base(&root.join("tasks/self-edit"), &spec).unwrap_err();
        assert!(error.contains("self-edit target does not exist"), "{error}");

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn self_edit_prompt_is_scoped_to_target_files() {
        let context = Context {
            project: "edit-the-gate".into(),
            title: "Edit the gate".into(),
            goal: "change the gate".into(),
            acceptance: Vec::new(),
            constraints: Vec::new(),
            non_goals: Vec::new(),
            target: vec!["src/gate.rs".into()],
            context: Vec::new(),
            open_questions: Vec::new(),
            feedback: Vec::new(),
        };
        let mut current = BTreeMap::new();
        current.insert("src/gate.rs".to_string(), "fn gate() {}\n".to_string());
        let repository = vec!["Cargo.toml".to_string(), "src/gate.rs".to_string()];

        let prompt = build_prompt(
            &context,
            Path::new("no-such-template-dir"),
            &[],
            None,
            Some(&current),
            &repository,
            Some("src/gate.rs"),
        )
        .unwrap();
        assert!(prompt.starts_with("Edit the existing Evengate project"));
        assert!(prompt.contains("## Target\n- src/gate.rs"));
        assert!(prompt.contains("# Repository files\n- Cargo.toml\n- src/gate.rs"));
        assert!(prompt.contains("# Files to edit"));
        assert!(prompt.contains("--- src/gate.rs ---\nfn gate() {}"));
        assert!(prompt.contains("Return the complete contents of `src/gate.rs`"));
        assert!(!prompt.contains("\"files\""));
        assert!(!prompt.contains("# Current project"));
        assert!(!prompt.contains("# Reference template"));

        let manifest_prompt = build_prompt(
            &context,
            Path::new("no-such-template-dir"),
            &[],
            None,
            Some(&current),
            &repository,
            None,
        )
        .unwrap();
        assert!(manifest_prompt.contains("Return only the files you changed"));
        assert!(manifest_prompt.contains("cannot delete files"));
    }

    #[test]
    fn single_file_responses_accept_raw_text_and_fences() {
        let raw = parse_executor_response("fn main() { changed }\n", Some("src/main.rs")).unwrap();
        assert_eq!(raw.files["src/main.rs"], "fn main() { changed }\n");

        let fenced =
            parse_executor_response("```rust\nfn main() { changed }\n```", Some("src/main.rs"))
                .unwrap();
        assert_eq!(fenced.files["src/main.rs"], "fn main() { changed }\n");

        assert!(parse_executor_response("  \n", Some("src/main.rs")).is_err());
    }

    #[test]
    fn revision_prompt_carries_the_gate_failures() {
        let spec = Spec {
            title: "Hello".into(),
            goal: "g".into(),
            acceptance: vec!["a".into()],
            ..Spec::default()
        };
        let mut failures = BTreeMap::new();
        failures.insert(
            GateId::Compiles,
            GateResult::Fail {
                reason: "boom".into(),
            },
        );
        let context = Context::from_revision(&spec, &failures, Some("keep it small"));
        let prompt = build_prompt(
            &context,
            Path::new("no-such-template-dir"),
            &[],
            None,
            None,
            &[],
            None,
        )
        .unwrap();
        assert!(prompt.starts_with("Revise the existing project"));
        assert!(prompt.contains("## Revision feedback"));
        assert!(prompt.contains("- cargo check failed: boom"));
        assert!(prompt.contains("- human guidance: keep it small"));
    }

    #[test]
    fn prompt_carries_previous_lessons() {
        let spec = Spec {
            title: "Hello".into(),
            goal: "g".into(),
            acceptance: vec!["a".into()],
            ..Spec::default()
        };
        let context = Context::from_spec(&spec);
        let lessons = vec![Lesson {
            name: "earlier".into(),
            body: "# Lessons — Earlier\n\n- write tests first".into(),
        }];
        let prompt = build_prompt(
            &context,
            Path::new("no-such-template-dir"),
            &lessons,
            None,
            None,
            &[],
            None,
        )
        .unwrap();
        assert!(prompt.contains("# Lessons from previous tasks"));
        assert!(prompt.contains("--- earlier.md ---"));
        assert!(prompt.contains("- write tests first"));
    }

    #[test]
    fn prompt_carries_project_rules_before_lessons() {
        let spec = Spec {
            title: "Hello".into(),
            goal: "g".into(),
            acceptance: vec!["a".into()],
            ..Spec::default()
        };
        let context = Context::from_spec(&spec);
        let lessons = vec![Lesson {
            name: "earlier".into(),
            body: "- be small".into(),
        }];
        let prompt = build_prompt(
            &context,
            Path::new("no-such-template-dir"),
            &lessons,
            Some("No unsafe code.\nNo network."),
            None,
            &[],
            None,
        )
        .unwrap();
        assert!(prompt.contains("# Project rules\n"));
        assert!(prompt.contains("No unsafe code."));
        let rules_at = prompt.find("# Project rules").unwrap();
        let lessons_at = prompt.find("# Lessons from previous tasks").unwrap();
        assert!(rules_at < lessons_at);
        assert_eq!(
            prompt,
            build_prompt(
                &context,
                Path::new("no-such-template-dir"),
                &lessons,
                Some("No unsafe code.\nNo network."),
                None,
                &[],
                None,
            )
            .unwrap()
        );
    }

    #[test]
    fn revision_prompt_shows_the_current_project_instead_of_the_template() {
        let spec = Spec {
            title: "Hello".into(),
            goal: "g".into(),
            acceptance: vec!["a".into()],
            ..Spec::default()
        };
        let mut failures = BTreeMap::new();
        failures.insert(
            GateId::Compiles,
            GateResult::Fail {
                reason: "boom".into(),
            },
        );
        let context = Context::from_revision(&spec, &failures, None);
        let mut current = BTreeMap::new();
        current.insert(
            "src/main.rs".to_string(),
            "fn main() { let broken: i32 = \"x\"; }\n".to_string(),
        );
        let prompt = build_prompt(
            &context,
            Path::new("no-such-template-dir"),
            &[],
            None,
            Some(&current),
            &[],
            None,
        )
        .unwrap();
        assert!(prompt.contains("# Current project"));
        assert!(prompt.contains("--- src/main.rs ---"));
        assert!(prompt.contains("let broken"));
        assert!(!prompt.contains("# Reference template"));
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

        assert_eq!(apply_manifest(&manifest, &project, false).unwrap(), 2);
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
        assert!(apply_manifest(&FileManifest { files }, &project, false).is_err());
        assert!(project.join("keep.txt").exists());
        assert!(!root.join("escape.txt").exists());

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn review_prompt_reads_the_project_and_skips_vcs_and_target() {
        let root = env::temp_dir().join(format!("evengate-review-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let project = root.join("project");
        fs::create_dir_all(project.join("src")).unwrap();
        fs::create_dir_all(project.join(".git")).unwrap();
        fs::create_dir_all(project.join("target/debug")).unwrap();
        fs::write(project.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        fs::write(project.join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(project.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(project.join("target/debug/junk"), "junk").unwrap();

        let context = Context {
            project: "hello".into(),
            title: "Hello".into(),
            goal: "make a crate".into(),
            acceptance: vec!["it compiles".into()],
            constraints: Vec::new(),
            non_goals: Vec::new(),
            target: Vec::new(),
            context: Vec::new(),
            open_questions: Vec::new(),
            feedback: Vec::new(),
        };
        let rules = Some("No unsafe code.");
        let lessons = vec![Lesson {
            name: "sum-cli".into(),
            body: "# Lessons — Sum CLI\n\n- Keep the usage message clear.".into(),
        }];
        let prompt = build_review_prompt(&context, &project, rules, &lessons, None).unwrap();
        assert!(prompt.contains("--- src/main.rs ---\nfn main() {}"));
        assert!(prompt.contains("--- Cargo.toml ---"));
        assert!(!prompt.contains(".git/HEAD"));
        assert!(!prompt.contains("target/debug/junk"));
        assert!(prompt.contains("# Project rules\n"));
        assert!(prompt.contains("No unsafe code."));
        assert!(prompt.contains("# Lessons from previous tasks"));
        assert!(prompt.contains("Keep the usage message clear."));
        assert!(prompt.contains("copy that lesson's text exactly"));
        assert!(prompt.find("# Project rules").unwrap() < prompt.find("# Project files").unwrap());
        assert!(
            prompt.find("# Lessons from previous tasks").unwrap()
                < prompt.find("# Project files").unwrap()
        );
        assert!(prompt.contains("\"approve\""));
        assert_eq!(
            prompt,
            build_review_prompt(&context, &project, rules, &lessons, None).unwrap()
        );

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn self_edit_review_prompt_carries_only_changed_files() {
        let root = env::temp_dir().join(format!("evengate-review-edit-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let project = root.join("project");
        fs::create_dir_all(project.join("src")).unwrap();
        fs::write(project.join("src/main.rs"), "fn main() { changed }\n").unwrap();
        fs::write(project.join("src/other.rs"), "fn other() {}\n").unwrap();

        let mut base = BTreeMap::new();
        base.insert("src/main.rs".to_string(), "fn main() {}\n".to_string());
        base.insert("src/other.rs".to_string(), "fn other() {}\n".to_string());
        let mut context = Context::from_spec(&Spec {
            title: "Edit".into(),
            goal: "g".into(),
            target: vec!["src/main.rs".into()],
            ..Spec::default()
        });
        context.feedback.clear();

        let prompt = build_review_prompt(&context, &project, None, &[], Some(&base)).unwrap();
        assert!(prompt.starts_with("Review the change below"));
        assert!(prompt.contains("# Changed files"));
        assert!(prompt.contains("--- src/main.rs ---\nfn main() { changed }"));
        assert!(!prompt.contains("--- src/other.rs ---"));
        assert!(prompt.contains("- src/other.rs"));
        assert!(!prompt.contains("fn main() {}\n"));

        fs::remove_dir_all(&root).unwrap();
    }

    fn materialize_dir(tag: &str) -> PathBuf {
        let root =
            env::temp_dir().join(format!("evengate-materialize-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        root
    }

    #[test]
    fn materialize_rebuilds_the_project_and_the_lesson_file() {
        let root = materialize_dir("rebuild");
        let task = root.join("word-count");
        let lessons = root.join("lessons");
        log::append_event(
            &task,
            &Event::TaskCreated {
                spec: Spec {
                    title: "Word Count".into(),
                    goal: "g".into(),
                    ..Spec::default()
                },
                gates: gate::MANDATORY_V0.to_vec(),
            },
        )
        .unwrap();
        log::append_event(
            &task,
            &Event::AgentResponded {
                role: AgentRole::Executor,
                response: r#"{"files":{"src/main.rs":"fn main() {}\n","Cargo.toml":"[package]\nname = \"word-count\"\n"}}"#.into(),
                tokens: Default::default(),
            },
        )
        .unwrap();
        log::append_event(
            &task,
            &Event::AgentResponded {
                role: AgentRole::Reviewer,
                response: r#"{"approve":true,"issues":[],"lessons":["import what the tests use"]}"#
                    .into(),
                tokens: Default::default(),
            },
        )
        .unwrap();
        fs::create_dir_all(task.join("project/old")).unwrap();
        fs::write(task.join("project/old/stale.txt"), "stale").unwrap();

        let report = materialize(&task, &lessons).unwrap();
        assert_eq!(
            report,
            MaterializeReport {
                responses: 1,
                files: 2,
                lesson: true
            }
        );
        assert_eq!(
            fs::read_to_string(task.join("project/src/main.rs")).unwrap(),
            "fn main() {}\n"
        );
        assert!(!task.join("project/old").exists());
        let lesson = fs::read_to_string(lessons.join("word-count.md")).unwrap();
        assert!(lesson.contains("- import what the tests use"));

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn materialize_applies_self_edit_responses_over_the_base() {
        let root = materialize_dir("self-edit");
        let task = root.join("self-edit");
        let lessons = root.join("lessons");
        log::append_event(
            &task,
            &Event::TaskCreated {
                spec: Spec {
                    title: "Self Edit".into(),
                    goal: "g".into(),
                    target: vec!["src/main.rs".into()],
                    ..Spec::default()
                },
                gates: gate::MANDATORY_V0.to_vec(),
            },
        )
        .unwrap();
        let mut base = BTreeMap::new();
        base.insert("src/main.rs".to_string(), "fn main() {}\n".to_string());
        base.insert("src/keep.rs".to_string(), "fn keep() {}\n".to_string());
        log::append_event(&task, &Event::SelfEditBase { files: base }).unwrap();
        log::append_event(
            &task,
            &Event::AgentResponded {
                role: AgentRole::Executor,
                response: "fn main() { changed }\n".into(),
                tokens: Default::default(),
            },
        )
        .unwrap();
        log::append_event(
            &task,
            &Event::AgentResponded {
                role: AgentRole::Reviewer,
                response: r#"{"approve":true,"issues":[],"lessons":["keep the diff small"]}"#
                    .into(),
                tokens: Default::default(),
            },
        )
        .unwrap();
        fs::create_dir_all(task.join("project")).unwrap();
        fs::write(task.join("project/stale.txt"), "stale").unwrap();

        let report = materialize(&task, &lessons).unwrap();
        assert_eq!(
            report,
            MaterializeReport {
                responses: 1,
                files: 3,
                lesson: true
            }
        );
        assert_eq!(
            fs::read_to_string(task.join("project/src/main.rs")).unwrap(),
            "fn main() { changed }\n"
        );
        assert_eq!(
            fs::read_to_string(task.join("project/src/keep.rs")).unwrap(),
            "fn keep() {}\n"
        );
        assert!(!task.join("project/stale.txt").exists());

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn materialize_keeps_only_the_last_manifest() {
        let root = materialize_dir("revision");
        let task = root.join("hello");
        let lessons = root.join("lessons");
        log::append_event(
            &task,
            &Event::TaskCreated {
                spec: Spec {
                    title: "Hello".into(),
                    goal: "g".into(),
                    ..Spec::default()
                },
                gates: gate::MANDATORY_V0.to_vec(),
            },
        )
        .unwrap();
        for response in [
            r#"{"files":{"src/main.rs":"old\n","stale.txt":"stale\n"}}"#,
            r#"{"files":{"src/main.rs":"new\n"}}"#,
        ] {
            log::append_event(
                &task,
                &Event::AgentResponded {
                    role: AgentRole::Executor,
                    response: response.into(),
                    tokens: Default::default(),
                },
            )
            .unwrap();
        }

        let report = materialize(&task, &lessons).unwrap();
        assert_eq!(report.responses, 2);
        assert_eq!(
            fs::read_to_string(task.join("project/src/main.rs")).unwrap(),
            "new\n"
        );
        assert!(!task.join("project/stale.txt").exists());
        assert!(!report.lesson);

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn materialize_requires_an_executor_response() {
        let root = materialize_dir("missing");
        let task = root.join("hello");
        log::append_event(
            &task,
            &Event::TaskCreated {
                spec: Spec {
                    title: "Hello".into(),
                    goal: "g".into(),
                    ..Spec::default()
                },
                gates: gate::MANDATORY_V0.to_vec(),
            },
        )
        .unwrap();
        assert!(materialize(&task, &root.join("lessons")).is_err());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn repetition_events_fire_at_the_threshold_and_skip_rules() {
        let root = materialize_dir("repetition");
        let lessons = root.join("lessons");
        let rules = root.join("rules");
        fs::create_dir_all(&lessons).unwrap();
        for (name, line) in [("a.md", "Keep it small."), ("b.md", "keep it  small.")] {
            fs::write(lessons.join(name), format!("# Lessons\n\n- {line}\n")).unwrap();
        }
        assert!(repetition_events(&lessons, &rules, "hello")
            .unwrap()
            .is_empty());

        fs::write(lessons.join("c.md"), "# Lessons\n\n- KEEP IT SMALL.\n").unwrap();
        let events = repetition_events(&lessons, &rules, "hello").unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::LessonRepeated {
                lesson,
                text,
                count,
            } => {
                assert_eq!(lesson, "keep it small.");
                assert_eq!(text, "Keep it small.");
                assert_eq!(*count, 3);
            }
            other => panic!("unexpected event: {other:?}"),
        }

        knowledge::append_rule(&rules, "hello", "Keep it small.").unwrap();
        assert!(repetition_events(&lessons, &rules, "hello")
            .unwrap()
            .is_empty());

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn repetition_events_group_paraphrases_and_skip_similar_rules() {
        let root = materialize_dir("paraphrase-repetition");
        let lessons = root.join("lessons");
        let rules = root.join("rules");
        fs::create_dir_all(&lessons).unwrap();
        for (name, line) in [
            (
                "a.md",
                "Ensure that the usage message is consistent and provides clear instructions.",
            ),
            ("b.md", "Keep the usage message consistent and clear."),
        ] {
            fs::write(lessons.join(name), format!("# Lessons\n\n- {line}\n")).unwrap();
        }
        assert!(repetition_events(&lessons, &rules, "hello")
            .unwrap()
            .is_empty());

        fs::write(
            lessons.join("c.md"),
            "# Lessons\n\n- Print a consistent usage message with clear instructions.\n",
        )
        .unwrap();
        let events = repetition_events(&lessons, &rules, "hello").unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::LessonRepeated {
                lesson,
                text,
                count,
            } => {
                assert_eq!(
                    lesson,
                    &knowledge::canonical_lesson(
                        "Ensure that the usage message is consistent and provides clear instructions."
                    )
                );
                assert_eq!(
                    text,
                    "Ensure that the usage message is consistent and provides clear instructions."
                );
                assert_eq!(*count, 3);
            }
            other => panic!("unexpected event: {other:?}"),
        }

        knowledge::append_rule(
            &rules,
            "hello",
            "Keep the usage message consistent and clear.",
        )
        .unwrap();
        assert!(repetition_events(&lessons, &rules, "hello")
            .unwrap()
            .is_empty());

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn self_edit_pr_commits_the_base_before_the_change() {
        let root = materialize_dir("self-edit-pr");
        let task = root.join("self-edit");
        let spec = Spec {
            title: "Test Edit".into(),
            goal: "g".into(),
            target: vec!["src/main.rs".into()],
            ..Spec::default()
        };
        log::append_event(
            &task,
            &Event::TaskCreated {
                spec: spec.clone(),
                gates: gate::MANDATORY_V0.to_vec(),
            },
        )
        .unwrap();
        let mut base = BTreeMap::new();
        base.insert("src/main.rs".to_string(), "fn main() {}\n".to_string());
        base.insert("src/keep.rs".to_string(), "fn keep() {}\n".to_string());
        log::append_event(&task, &Event::SelfEditBase { files: base }).unwrap();

        let project = task.join("project");
        fs::create_dir_all(project.join("src")).unwrap();
        fs::write(project.join("src/main.rs"), "fn main() { changed }\n").unwrap();
        fs::write(project.join("src/keep.rs"), "fn keep() {}\n").unwrap();

        let ctx = ExecCtx::new(&task, &spec);
        let events = open_pull_request("ev/test", &ctx).unwrap();
        assert!(matches!(events.last(), Some(Event::PrOpened { .. })));

        let log = Process::new("git")
            .current_dir(&project)
            .args(["log", "--format=%s", "--reverse"])
            .output()
            .unwrap();
        assert!(log.status.success());
        assert_eq!(
            String::from_utf8_lossy(&log.stdout).trim(),
            "ev: initialize test-edit\nev: Test Edit"
        );

        let diff = Process::new("git")
            .current_dir(&project)
            .args(["diff", "--name-status", "main", "ev/test"])
            .output()
            .unwrap();
        assert!(diff.status.success());
        assert_eq!(
            String::from_utf8_lossy(&diff.stdout).trim(),
            "M\tsrc/main.rs"
        );

        fs::remove_dir_all(&root).unwrap();
    }
}

mod event;
mod executor;
mod gate;
mod knowledge;
mod llm;
mod log;
mod machine;
mod sandbox;
mod spec;
mod state;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::event::{Event, GateResult};
use crate::executor::ExecCtx;
use crate::machine::Command;
use crate::state::{Phase, RepeatedLesson, State};

const USAGE: &str = "\
ev — Evengate control plane

Usage:
  ev run <spec.md>                          start or resume a task
  ev replay <task-dir>                      rebuild state.json from events.jsonl
  ev materialize <task-dir>                 rebuild project/ and lessons from events.jsonl
  ev approve <task-dir> --reviewer <name>   record human approval, then finalize
  ev reject <task-dir> --reviewer <name> --reason <text>
  ev revise <task-dir> --reviewer <name> [--reason <text>]
                                            re-run the executor with the gate failures
  ev promote <task-dir> --reviewer <name> [--lesson <text>]
                                            add a repeated lesson to rules/
  ev dismiss <task-dir> --reviewer <name> [--lesson <text>]
                                            decline promoting a repeated lesson
  ev abandon <task-dir> --reviewer <name> --reason <text>
  ev help
";

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if let Err(error) = dispatch(&args) {
        eprintln!("ev: {error}");
        std::process::exit(1);
    }
}

fn dispatch(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("run") => run_task(args),
        Some("replay") => replay(args),
        Some("materialize") => materialize(args),
        Some("approve") => decide(args, true),
        Some("reject") => decide(args, false),
        Some("revise") => revise(args),
        Some("promote") => promotion_decision(args, true),
        Some("dismiss") => promotion_decision(args, false),
        Some("abandon") => abandon(args),
        Some("help" | "--help" | "-h") | None => {
            print!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    }
}

fn run_task(args: &[String]) -> Result<(), String> {
    let spec_path = PathBuf::from(args.get(1).ok_or("usage: ev run <spec.md>")?);
    let markdown = fs::read_to_string(&spec_path)
        .map_err(|e| format!("cannot read {}: {e}", spec_path.display()))?;
    let spec = spec::parse(&markdown)?;
    let task_dir = spec_path
        .parent()
        .ok_or("spec path has no parent directory")?
        .to_path_buf();

    let mut events = log::read_events(&task_dir)
        .map_err(|e| format!("cannot read {}: {e}", log::events_path(&task_dir).display()))?;
    if events.is_empty() {
        let created = Event::TaskCreated { spec };
        log::append_event(&task_dir, &created)
            .map_err(|e| format!("cannot append event: {e}"))?;
        events.push(created);
    }
    let state = fold(&events);
    log::write_state(&task_dir, &state).map_err(|e| format!("cannot write state: {e}"))?;
    resume(&task_dir, state)
}

fn resume(task_dir: &Path, mut state: State) -> Result<(), String> {
    let ctx = ExecCtx::new(task_dir, &state.spec);
    while let Some(command) = machine::next_command(&state) {
        if command.is_awaiting() {
            match &command {
                Command::RequestHumanReview { artifact } => {
                    if artifact.kind == "promotion" {
                        println!("ev: promotion review required");
                    } else {
                        println!("ev: gate blocked; human review required");
                    }
                    print_blockers(&state);
                }
                _ => println!("ev: waiting for a human decision"),
            }
            break;
        }
        let events = executor::run(&command, &ctx)?;
        if events.is_empty() {
            return Err(format!("executor produced no events for {command:?}"));
        }
        for event in &events {
            log::append_event(task_dir, event).map_err(|e| format!("cannot append event: {e}"))?;
            state = state::reduce(state, event);
            log::write_state(task_dir, &state).map_err(|e| format!("cannot write state: {e}"))?;
        }
        println!(
            "ev: {} -> phase {}",
            command_kind(&command),
            phase_name(state.phase)
        );
    }
    println!("ev: phase {}", phase_name(state.phase));
    if let Some(url) = &state.pr_url {
        println!("ev: pull request {url}");
    }
    if !state.anomalies.is_empty() {
        println!("ev: {} anomaly(ies) recorded", state.anomalies.len());
    }
    Ok(())
}

fn replay(args: &[String]) -> Result<(), String> {
    let task_dir = PathBuf::from(args.get(1).ok_or("usage: ev replay <task-dir>")?);
    let events = log::read_events(&task_dir)
        .map_err(|e| format!("cannot read {}: {e}", log::events_path(&task_dir).display()))?;
    if events.is_empty() {
        return Err(format!("no events to replay in {}", task_dir.display()));
    }
    let state = fold(&events);
    let snapshot = task_dir.join(log::STATE_FILE);
    let previous = fs::read(&snapshot).ok();
    log::write_state(&task_dir, &state).map_err(|e| format!("cannot write state: {e}"))?;
    let current = fs::read(&snapshot).map_err(|e| format!("cannot read state: {e}"))?;
    match previous {
        Some(old) if old == current => println!(
            "ev: replay of {} event(s) produced a byte-identical state.json",
            events.len()
        ),
        Some(_) => {
            return Err("replay produced a state.json that differs from the live snapshot".into())
        }
        None => println!("ev: replay created state.json (no previous snapshot)"),
    }
    Ok(())
}

fn materialize(args: &[String]) -> Result<(), String> {
    let task_dir = PathBuf::from(args.get(1).ok_or("usage: ev materialize <task-dir>")?);
    let report = executor::materialize(&task_dir, &executor::lessons_root())?;
    println!(
        "ev: materialized {} executor response(s) into {} file(s){}",
        report.responses,
        report.files,
        if report.lesson {
            "; rebuilt the lesson file"
        } else {
            ""
        }
    );
    Ok(())
}

fn decide(args: &[String], approve: bool) -> Result<(), String> {
    let usage = if approve {
        "usage: ev approve <task-dir> --reviewer <name>"
    } else {
        "usage: ev reject <task-dir> --reviewer <name> --reason <text>"
    };
    let task_dir = PathBuf::from(args.get(1).ok_or(usage)?);
    let reviewer = flag(args, "--reviewer").ok_or(usage)?.to_string();
    let mut events = log::read_events(&task_dir)
        .map_err(|e| format!("cannot read {}: {e}", log::events_path(&task_dir).display()))?;
    if events.is_empty() {
        return Err(format!("no events in {}", task_dir.display()));
    }
    let mut state = fold(&events);
    if state.phase != Phase::Reviewing {
        return Err(format!(
            "task is in phase {}, not awaiting review",
            phase_name(state.phase)
        ));
    }
    if state.human_decision.is_some() {
        return Err("a human decision is already recorded".into());
    }
    let event = if approve {
        Event::HumanApproved { reviewer }
    } else {
        let reason = flag(args, "--reason").ok_or(usage)?.to_string();
        Event::HumanRejected { reviewer, reason }
    };
    log::append_event(&task_dir, &event).map_err(|e| format!("cannot append event: {e}"))?;
    state = state::reduce(state, &event);
    log::write_state(&task_dir, &state).map_err(|e| format!("cannot write state: {e}"))?;
    events.push(event);
    resume(&task_dir, fold(&events))
}

fn revise(args: &[String]) -> Result<(), String> {
    let usage = "usage: ev revise <task-dir> --reviewer <name> [--reason <text>]";
    let task_dir = PathBuf::from(args.get(1).ok_or(usage)?);
    let reviewer = flag(args, "--reviewer").ok_or(usage)?.to_string();
    let guidance = flag(args, "--reason").map(str::to_string);
    let mut events = log::read_events(&task_dir)
        .map_err(|e| format!("cannot read {}: {e}", log::events_path(&task_dir).display()))?;
    if events.is_empty() {
        return Err(format!("no events in {}", task_dir.display()));
    }
    let state = fold(&events);
    if state.phase == Phase::Verifying
        && state::all_mandatory_passed(&state)
        && !machine::pending_promotions(&state).is_empty()
    {
        return Err(
            "task is paused for a promotion decision; use `ev promote` or `ev dismiss`".into(),
        );
    }
    let blocked = state.phase == Phase::Verifying
        && matches!(
            machine::next_command(&state),
            Some(Command::RequestHumanReview { .. })
        );
    if !blocked {
        return Err(format!(
            "task is in phase {} and is not blocked by a gate",
            phase_name(state.phase)
        ));
    }
    if state.revisions >= machine::MAX_REVISIONS {
        return Err(format!(
            "revision limit reached ({}); abandon the task or accept the result",
            machine::MAX_REVISIONS
        ));
    }
    let event = Event::RevisionRequested {
        reviewer,
        guidance,
    };
    log::append_event(&task_dir, &event).map_err(|e| format!("cannot append event: {e}"))?;
    events.push(event);
    let state = fold(&events);
    log::write_state(&task_dir, &state).map_err(|e| format!("cannot write state: {e}"))?;
    println!(
        "ev: revision {} of {} requested",
        state.revisions,
        machine::MAX_REVISIONS
    );
    resume(&task_dir, state)
}

fn promotion_decision(args: &[String], promote: bool) -> Result<(), String> {
    let command = if promote { "promote" } else { "dismiss" };
    let usage = format!("usage: ev {command} <task-dir> --reviewer <name> [--lesson <text>]");
    let task_dir = PathBuf::from(args.get(1).ok_or(usage.clone())?);
    let reviewer = flag(args, "--reviewer").ok_or(usage)?.to_string();
    let mut events = log::read_events(&task_dir)
        .map_err(|e| format!("cannot read {}: {e}", log::events_path(&task_dir).display()))?;
    if events.is_empty() {
        return Err(format!("no events in {}", task_dir.display()));
    }
    let state = fold(&events);
    if state.phase != Phase::Verifying {
        return Err(format!(
            "task is in phase {}, not verifying",
            phase_name(state.phase)
        ));
    }
    if !state::all_mandatory_passed(&state) {
        return Err("promotion is offered only after every gate passes".into());
    }
    let pending = machine::pending_promotions(&state);
    if pending.is_empty() {
        return Err("no lesson is pending promotion".into());
    }
    let index = match flag(args, "--lesson") {
        Some(text) => {
            let key = knowledge::canonical_lesson(text);
            pending
                .iter()
                .position(|(pending_key, _)| pending_key == &key)
                .or_else(|| {
                    pending
                        .iter()
                        .position(|(pending_key, _)| knowledge::lessons_match(pending_key, &key))
                })
                .ok_or_else(|| {
                    format!(
                        "`{text}` is not a pending lesson; pending: {}",
                        pending_texts(&pending)
                    )
                })?
        }
        None if pending.len() == 1 => 0,
        None => {
            return Err(format!(
                "several lessons are pending; pass --lesson <text>: {}",
                pending_texts(&pending)
            ))
        }
    };
    let (lesson, repeated) = &pending[index];
    if promote {
        let path =
            knowledge::append_rule(&executor::rules_root(), &spec::slug(&state.spec.title), &repeated.text)?;
        println!("ev: promoted to {}", path.display());
    }
    let event = if promote {
        Event::LessonPromoted {
            reviewer,
            lesson: lesson.clone(),
        }
    } else {
        Event::LessonDismissed {
            reviewer,
            lesson: lesson.clone(),
        }
    };
    log::append_event(&task_dir, &event).map_err(|e| format!("cannot append event: {e}"))?;
    events.push(event);
    let state = fold(&events);
    log::write_state(&task_dir, &state).map_err(|e| format!("cannot write state: {e}"))?;
    resume(&task_dir, state)
}

fn pending_texts(pending: &[(String, RepeatedLesson)]) -> String {
    pending
        .iter()
        .map(|(_, repeated)| format!("\"{}\"", repeated.text))
        .collect::<Vec<_>>()
        .join("; ")
}

fn abandon(args: &[String]) -> Result<(), String> {
    let usage = "usage: ev abandon <task-dir> --reviewer <name> --reason <text>";
    let task_dir = PathBuf::from(args.get(1).ok_or(usage)?);
    let reviewer = flag(args, "--reviewer").ok_or(usage)?.to_string();
    let reason = flag(args, "--reason").ok_or(usage)?.to_string();
    let mut events = log::read_events(&task_dir)
        .map_err(|e| format!("cannot read {}: {e}", log::events_path(&task_dir).display()))?;
    if events.is_empty() {
        return Err(format!("no events in {}", task_dir.display()));
    }
    let state = fold(&events);
    if matches!(state.phase, Phase::Merged | Phase::Abandoned) {
        return Err(format!("task is already {}", phase_name(state.phase)));
    }
    let event = Event::HumanAbandoned { reviewer, reason };
    log::append_event(&task_dir, &event).map_err(|e| format!("cannot append event: {e}"))?;
    events.push(event);
    let state = fold(&events);
    log::write_state(&task_dir, &state).map_err(|e| format!("cannot write state: {e}"))?;
    resume(&task_dir, state)
}

fn print_blockers(state: &State) {
    let results = if state.phase == Phase::Revising {
        &state.revision_failures
    } else {
        &state.gate_results
    };
    for (id, result) in results {
        match result {
            GateResult::Fail { reason } => println!("ev:   {}: {reason}", gate::label(*id)),
            GateResult::NeedsHuman { question } => {
                println!("ev:   {}: {question}", gate::label(*id))
            }
            GateResult::Pass => {}
        }
    }
    if state::all_mandatory_passed(state) {
        let pending = machine::pending_promotions(state);
        if !pending.is_empty() {
            for (_, repeated) in &pending {
                println!(
                    "ev:   promotion: lesson repeated {} times: \"{}\"",
                    repeated.count, repeated.text
                );
            }
            println!("ev:   use `ev promote` or `ev dismiss` to decide");
        }
    }
    if state.revisions >= machine::MAX_REVISIONS {
        println!(
            "ev:   revision limit reached ({}); use `ev abandon` to stop",
            machine::MAX_REVISIONS
        );
    }
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    let index = args.iter().position(|arg| arg == name)?;
    args.get(index + 1).map(String::as_str)
}

fn fold(events: &[Event]) -> State {
    events
        .iter()
        .fold(state::initial(), state::reduce)
}

fn phase_name(phase: Phase) -> String {
    format!("{phase:?}").to_ascii_lowercase()
}

fn command_kind(command: &Command) -> &'static str {
    match command {
        Command::InvokeAgent { .. } => "invoke-agent",
        Command::RunGate { .. } => "run-gate",
        Command::RequestHumanReview { .. } => "request-human-review",
        Command::OpenPullRequest { .. } => "open-pull-request",
        Command::AwaitHuman => "await-human",
        Command::Finalize { .. } => "finalize",
    }
}

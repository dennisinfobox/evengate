use serde::{Deserialize, Serialize};
use std::fmt;

/// A task specification, parsed from the fixed-heading Markdown format in
/// `docs/spec.md` §8. Only headings and bullets are recognized; everything
/// else is ignored, deliberately.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spec {
    pub title: String,
    pub goal: String,
    pub acceptance: Vec<String>,
    pub constraints: Vec<String>,
    pub non_goals: Vec<String>,
    /// Repository-relative files this task edits. A target under `src/` makes
    /// the task a self-improvement task (docs/spec.md §10.2).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub target: Vec<String>,
    pub context: Vec<String>,
    pub open_questions: Vec<String>,
}

impl Spec {
    /// A task is a self-edit when it targets Evengate's own `src/`.
    pub fn is_self_edit(&self) -> bool {
        self.target.iter().any(|path| targets_self(path))
    }
}

/// Whether a repository-relative target points under `src/`.
pub fn targets_self(path: &str) -> bool {
    let normalized = path.trim().replace('\\', "/");
    let normalized = normalized.strip_prefix("./").unwrap_or(&normalized);
    normalized == "src" || normalized.starts_with("src/")
}

#[derive(Debug, PartialEq, Eq)]
pub struct SpecError(pub String);

impl fmt::Display for SpecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid spec: {}", self.0)
    }
}

impl std::error::Error for SpecError {}

impl From<SpecError> for String {
    fn from(error: SpecError) -> Self {
        error.to_string()
    }
}

enum Section {
    None,
    Goal,
    Acceptance,
    Constraints,
    NonGoals,
    Target,
    Context,
    OpenQuestions,
    Other,
}

pub fn parse(markdown: &str) -> Result<Spec, SpecError> {
    let mut spec = Spec::default();
    let mut section = Section::None;
    let mut goal_lines: Vec<String> = Vec::new();

    for raw in markdown.lines() {
        let line = raw.trim_end();
        if let Some(heading) = line.strip_prefix("## ") {
            section = match heading.trim().to_ascii_lowercase().as_str() {
                "goal" => Section::Goal,
                "acceptance" => Section::Acceptance,
                "constraints" => Section::Constraints,
                "non-goals" | "non goals" => Section::NonGoals,
                "target" => Section::Target,
                "context" => Section::Context,
                "open questions" => Section::OpenQuestions,
                _ => Section::Other,
            };
            continue;
        }
        if let Some(title) = line.strip_prefix("# ") {
            if spec.title.is_empty() {
                spec.title = title.trim().to_string();
            }
            continue;
        }
        match section {
            Section::Goal => goal_lines.push(line.to_string()),
            Section::Acceptance => push_bullet(&mut spec.acceptance, line),
            Section::Constraints => push_bullet(&mut spec.constraints, line),
            Section::NonGoals => push_bullet(&mut spec.non_goals, line),
            Section::Target => push_bullet(&mut spec.target, line),
            Section::Context => push_bullet(&mut spec.context, line),
            Section::OpenQuestions => push_bullet(&mut spec.open_questions, line),
            Section::None | Section::Other => {}
        }
    }

    spec.goal = goal_lines.join("\n").trim().to_string();

    if spec.title.is_empty() {
        return Err(SpecError("missing `# <title>` heading".into()));
    }
    if spec.goal.is_empty() {
        return Err(SpecError("missing or empty `## Goal` section".into()));
    }
    let paragraphs = spec
        .goal
        .split("\n\n")
        .filter(|p| !p.trim().is_empty())
        .count();
    if paragraphs > 1 {
        return Err(SpecError(
            "`## Goal` must be a single paragraph; split the task instead".into(),
        ));
    }
    if spec.acceptance.is_empty() {
        return Err(SpecError("missing or empty `## Acceptance` section".into()));
    }
    if spec.constraints.is_empty() {
        return Err(SpecError(
            "missing or empty `## Constraints` section".into(),
        ));
    }

    Ok(spec)
}

fn push_bullet(target: &mut Vec<String>, line: &str) {
    if let Some(item) = bullet(line) {
        target.push(item);
    }
}

fn bullet(line: &str) -> Option<String> {
    let rest = line.trim_start().strip_prefix('-')?.trim_start();
    let rest = rest
        .strip_prefix("[ ]")
        .or_else(|| rest.strip_prefix("[x]"))
        .or_else(|| rest.strip_prefix("[X]"))
        .unwrap_or(rest)
        .trim();
    if rest.is_empty() {
        None
    } else {
        Some(rest.to_string())
    }
}

/// Deterministic identifier used for branch names, project directories, and
/// crate names.
pub fn slug(text: &str) -> String {
    let mut out = String::new();
    let mut pending_dash = false;
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(ch.to_ascii_lowercase());
        } else {
            pending_dash = true;
        }
    }
    if out.is_empty() {
        return "task".to_string();
    }
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        return format!("task-{out}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = "\
# hello

## Goal
Build the hello crate.

## Acceptance
- [ ] it builds
- [x] it greets

## Constraints
- no network

## Non-goals
- nothing

## Open questions
None.
";

    #[test]
    fn parses_a_valid_spec() {
        let spec = parse(VALID).unwrap();
        assert_eq!(spec.title, "hello");
        assert_eq!(spec.goal, "Build the hello crate.");
        assert_eq!(spec.acceptance, vec!["it builds", "it greets"]);
        assert_eq!(spec.constraints, vec!["no network"]);
    }

    #[test]
    fn rejects_missing_sections() {
        assert!(parse("# t\n\n## Goal\ng\n").is_err());
        assert!(parse("# t\n\n## Acceptance\n- a\n").is_err());
        assert!(parse("# t\n\n## Constraints\n- c\n").is_err());
        assert!(parse("## Goal\ng\n").is_err());
    }

    #[test]
    fn rejects_multi_paragraph_goal() {
        let text = "# t\n\n## Goal\nfirst\n\nsecond\n\n## Acceptance\n- a\n\n## Constraints\n- c\n";
        assert!(parse(text).is_err());
    }

    #[test]
    fn target_section_marks_self_edits() {
        let text = "\
# Gate Tweak

## Goal
Change a gate.

## Acceptance
- [ ] it works

## Constraints
- no new crates

## Target
- src/gate.rs
- docs/spec.md
";
        let spec = parse(text).unwrap();
        assert_eq!(spec.target, vec!["src/gate.rs", "docs/spec.md"]);
        assert!(spec.is_self_edit());

        let spec = parse(VALID).unwrap();
        assert!(spec.target.is_empty());
        assert!(!spec.is_self_edit());
    }

    #[test]
    fn self_edit_targets_are_normalized() {
        assert!(targets_self("src"));
        assert!(targets_self("src/gate.rs"));
        assert!(targets_self("./src/gate.rs"));
        assert!(targets_self("src\\gate.rs"));
        assert!(!targets_self("docs/spec.md"));
        assert!(!targets_self("srcs/gate.rs"));
        assert!(!targets_self(""));
    }

    #[test]
    fn empty_target_serializes_like_a_legacy_spec() {
        let spec = parse(VALID).unwrap();
        let json = serde_json::to_string(&spec).unwrap();
        assert!(!json.contains("target"), "{json}");

        let with_target = parse("# t\n\n## Goal\ng\n\n## Acceptance\n- a\n\n## Constraints\n- c\n\n## Target\n- src/main.rs\n").unwrap();
        assert!(serde_json::to_string(&with_target)
            .unwrap()
            .contains("\"target\":[\"src/main.rs\"]"));
    }

    #[test]
    fn slugs_are_filesystem_safe() {
        assert_eq!(slug("Hello Binary"), "hello-binary");
        assert_eq!(slug("hello"), "hello");
        assert_eq!(slug("  "), "task");
        assert_eq!(slug("9 lives"), "task-9-lives");
    }

    #[test]
    fn slug_collapses_consecutive_non_alphanumeric_characters() {
        assert_eq!(slug("a---b"), "a-b");
        assert_eq!(slug("a  b"), "a-b");
        assert_eq!(slug("a--b"), "a-b");
    }

    #[test]
    fn slug_trims_leading_and_trailing_non_alphanumeric_characters() {
        assert_eq!(slug("-a-b-"), "a-b");
        assert_eq!(slug("a-b-"), "a-b");
        assert_eq!(slug("-a-b"), "a-b");
    }
}

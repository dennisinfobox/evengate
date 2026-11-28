use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Lessons fed into a prompt. Spec §7.8 says the five most recent.
pub const RECENT_LESSONS: usize = 5;

/// A lesson seen in this many tasks is proposed for promotion to `rules/`.
pub const PROMOTION_THRESHOLD: u32 = 3;

/// Two lessons are "the same" when their content-token Dice coefficient is at
/// least `MATCH_NUMERATOR / MATCH_DENOMINATOR`. Integer arithmetic keeps the
/// comparison exact.
const MATCH_NUMERATOR: usize = 3;
const MATCH_DENOMINATOR: usize = 5;

/// Words that carry no lesson content: articles, prepositions, auxiliaries,
/// and generic advice framing. Content words are compared after stemming.
const STOPWORDS: [&str; 72] = [
    "a", "an", "the", "and", "or", "but", "if", "then", "than", "so", "to", "of", "in", "on", "at",
    "for", "with", "from", "by", "as", "is", "are", "was", "were", "be", "been", "being", "it",
    "its", "this", "that", "these", "those", "you", "your", "we", "our", "i", "me", "my", "them",
    "their", "they", "he", "she", "his", "her", "should", "must", "can", "could", "may", "might",
    "will", "would", "do", "does", "did", "have", "has", "had", "ensure", "remember", "try",
    "always", "keep", "use", "using", "make", "consider", "continue", "prefer",
];

/// Negations flip the meaning of otherwise similar advice, so a match requires
/// both lessons to agree on whether one is present.
const NEGATIONS: [&str; 5] = ["not", "no", "never", "avoid", "cannot"];

/// A lessons file read back from disk. `name` is the file stem (the task id).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lesson {
    pub name: String,
    pub body: String,
}

/// Write `lessons/<task-id>.md` with the reviewer's bullets. Overwrites an
/// earlier file for the same task, so a revision replaces stale advice.
pub fn write_lesson(
    dir: &Path,
    task_id: &str,
    title: &str,
    bullets: &[String],
) -> Result<PathBuf, String> {
    let bullets = clean_bullets(bullets);
    if bullets.is_empty() {
        return Err("cannot write a lesson file without bullets".into());
    }
    fs::create_dir_all(dir)
        .map_err(|e| format!("cannot create lessons directory {}: {e}", dir.display()))?;
    let path = dir.join(format!("{}.md", file_stem(task_id)));
    let mut body = format!("# Lessons — {}\n", title.trim());
    for bullet in &bullets {
        body.push_str(&format!("\n- {bullet}"));
    }
    body.push('\n');
    fs::write(&path, body).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(path)
}

/// The most recent lessons, newest first by modification time. A missing
/// directory is an empty knowledge base, not an error.
pub fn recent_lessons(dir: &Path, limit: usize) -> Result<Vec<Lesson>, String> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let entries = fs::read_dir(dir)
        .map_err(|e| format!("cannot read lessons directory {}: {e}", dir.display()))?;
    let mut files: Vec<(SystemTime, String, PathBuf)> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("md") {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .unwrap_or(UNIX_EPOCH);
        let name = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        files.push((modified, name, path));
    }
    files.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));

    files
        .into_iter()
        .take(limit)
        .map(|(_, name, path)| {
            let body = fs::read_to_string(&path)
                .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            Ok(Lesson {
                name,
                body: body.trim().to_string(),
            })
        })
        .collect()
}

/// Read `rules/<project>.md`. A missing or blank file means the project has
/// no extra rules; an unreadable file is an error, never silently ignored.
pub fn read_rules(dir: &Path, project: &str) -> Result<Option<String>, String> {
    let path = dir.join(format!("{}.md", file_stem(project)));
    match fs::read_to_string(&path) {
        Ok(body) => {
            let body = body.trim();
            Ok(if body.is_empty() {
                None
            } else {
                Some(body.to_string())
            })
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("cannot read {}: {error}", path.display())),
    }
}

/// Prompt section for project-wide rules. Empty when there are none.
pub fn rules_section(rules: Option<&str>) -> String {
    match rules {
        None => String::new(),
        Some(rules) => format!(
            "\n# Project rules\nThese rules apply to every task in this project. Follow them together with the task spec; the more specific instruction wins.\n\n{rules}\n"
        ),
    }
}

/// A lesson counted across lesson files, with one representative spelling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LessonCount {
    pub text: String,
    pub count: u32,
}

/// Exact key for a lesson: bullet markers and whitespace are irrelevant, and
/// ASCII case is ignored. Punctuation is preserved. `lessons_match` decides
/// whether two different keys are the same advice.
pub fn canonical_lesson(text: &str) -> String {
    let text = text.trim();
    let text = text
        .strip_prefix("- ")
        .or_else(|| text.strip_prefix("* "))
        .unwrap_or(text);
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

/// True when two lessons give the same advice, even if the wording differs.
/// Comparison is lexical: content tokens are stemmed and compared as sets, and
/// the Dice coefficient must reach `MATCH_NUMERATOR / MATCH_DENOMINATOR`.
/// Lessons that disagree on negation never match. Promotion stays a human
/// decision, so a near-miss is proposed, not applied.
pub fn lessons_match(a: &str, b: &str) -> bool {
    let left = lesson_tokens(a);
    let right = lesson_tokens(b);
    if left.is_empty() || right.is_empty() {
        return false;
    }
    let left_negated = left.iter().any(|token| NEGATIONS.contains(&token.as_str()));
    let right_negated = right
        .iter()
        .any(|token| NEGATIONS.contains(&token.as_str()));
    if left_negated != right_negated {
        return false;
    }
    let common = left.intersection(&right).count();
    2 * common * MATCH_DENOMINATOR >= (left.len() + right.len()) * MATCH_NUMERATOR
}

fn lesson_tokens(text: &str) -> BTreeSet<String> {
    canonical_lesson(text)
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty() && !STOPWORDS.contains(token))
        .map(stem)
        .collect()
}

/// Minimal English plural folding. It is deliberately conservative: a wrong
/// stem can merge unrelated lessons, so only clear plural endings move.
fn stem(token: &str) -> String {
    if token.len() > 5 && token.ends_with("ies") {
        return format!("{}y", &token[..token.len() - 3]);
    }
    if token.len() > 4 && token.ends_with("sses") {
        return token[..token.len() - 2].to_string();
    }
    if token.len() > 3 && token.ends_with('s') && !token.ends_with("ss") {
        return token[..token.len() - 1].to_string();
    }
    token.to_string()
}

/// Count each lesson once per task file, across every `*.md` in the lessons
/// directory. Paraphrases collapse into one group: the representative is the
/// exact spelling with the most files, ties broken by canonical key, and the
/// count is the number of distinct files containing any member. A missing
/// directory is an empty knowledge base.
pub fn count_lessons(dir: &Path) -> Result<BTreeMap<String, LessonCount>, String> {
    let mut occurrences: BTreeMap<String, (String, BTreeSet<PathBuf>)> = BTreeMap::new();
    if dir.is_dir() {
        let mut paths: Vec<PathBuf> = Vec::new();
        let entries = fs::read_dir(dir)
            .map_err(|e| format!("cannot read lessons directory {}: {e}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("md") {
                paths.push(path);
            }
        }
        paths.sort();

        for path in paths {
            let body = fs::read_to_string(&path)
                .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            let mut seen = BTreeSet::new();
            for line in body.lines() {
                let Some(bullet) = lesson_bullet(line) else {
                    continue;
                };
                let key = canonical_lesson(bullet);
                if key.is_empty() || !seen.insert(key.clone()) {
                    continue;
                }
                occurrences
                    .entry(key)
                    .or_insert_with(|| (clean_text(bullet), BTreeSet::new()))
                    .1
                    .insert(path.clone());
            }
        }
    }

    let mut order: Vec<&String> = occurrences.keys().collect();
    order.sort_by(|a, b| {
        occurrences[*b]
            .1
            .len()
            .cmp(&occurrences[*a].1.len())
            .then_with(|| a.cmp(b))
    });
    let mut groups: Vec<(String, BTreeSet<PathBuf>)> = Vec::new();
    for key in order {
        let files = &occurrences[key].1;
        if let Some((_, group_files)) = groups
            .iter_mut()
            .find(|group| lessons_match(group.0.as_str(), key))
        {
            group_files.extend(files.iter().cloned());
        } else {
            groups.push((key.clone(), files.clone()));
        }
    }

    let mut counts = BTreeMap::new();
    for (representative, files) in groups {
        let text = occurrences[&representative].0.clone();
        counts.insert(
            representative,
            LessonCount {
                text,
                count: files.len() as u32,
            },
        );
    }
    Ok(counts)
}

fn lesson_bullet(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
}

fn clean_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Canonical lessons already present as bullets in a rules file.
pub fn rule_lessons(rules: Option<&str>) -> BTreeSet<String> {
    let mut lessons = BTreeSet::new();
    if let Some(text) = rules {
        for line in text.lines() {
            if let Some(bullet) = lesson_bullet(line) {
                let key = canonical_lesson(bullet);
                if !key.is_empty() {
                    lessons.insert(key);
                }
            }
        }
    }
    lessons
}

/// Append a promoted lesson to `rules/<project>.md`, creating the file when
/// needed. An already present lesson is left untouched.
pub fn append_rule(dir: &Path, project: &str, lesson: &str) -> Result<PathBuf, String> {
    let lesson = lesson.split_whitespace().collect::<Vec<_>>().join(" ");
    if lesson.is_empty() {
        return Err("cannot promote an empty lesson".into());
    }
    let path = dir.join(format!("{}.md", file_stem(project)));
    let existing = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
    };
    if rule_lessons(Some(existing.as_str()))
        .iter()
        .any(|rule| lessons_match(rule, &lesson))
    {
        return Ok(path);
    }
    fs::create_dir_all(dir)
        .map_err(|e| format!("cannot create rules directory {}: {e}", dir.display()))?;
    let mut body = existing;
    if body.is_empty() {
        body.push_str(&format!("# Rules — {project}\n\n"));
    } else if !body.ends_with('\n') {
        body.push('\n');
    }
    body.push_str(&format!("- {lesson}\n"));
    fs::write(&path, body).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(path)
}

/// Prompt section for the executor. Empty when there is no knowledge yet.
pub fn prompt_section(lessons: &[Lesson]) -> String {
    if lessons.is_empty() {
        return String::new();
    }
    let mut section = String::from(
        "\n# Lessons from previous tasks\nEarlier tasks produced these notes. Apply them where relevant; the current spec and constraints win.\n",
    );
    for lesson in lessons {
        section.push_str(&format!("\n--- {}.md ---\n{}\n", lesson.name, lesson.body));
    }
    section
}

fn clean_bullets(bullets: &[String]) -> Vec<String> {
    let mut cleaned = Vec::new();
    for bullet in bullets {
        let text = bullet.trim();
        let text = text
            .strip_prefix("- ")
            .or_else(|| text.strip_prefix("* "))
            .unwrap_or(text)
            .trim();
        if !text.is_empty() {
            cleaned.push(text.to_string());
        }
        if cleaned.len() == RECENT_LESSONS {
            break;
        }
    }
    cleaned
}

fn file_stem(task_id: &str) -> String {
    let mut stem: String = task_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '-'
            }
        })
        .collect();
    if stem.trim_matches('.').is_empty() {
        stem = "task".into();
    }
    stem
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("evengate-knowledge-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn writes_clean_bullets_without_duplicating_dashes() {
        let dir = temp_dir("write");
        let path = write_lesson(
            &dir,
            "hello",
            "Hello",
            &[
                "- first".to_string(),
                "* second".to_string(),
                "  ".to_string(),
                "third".to_string(),
            ],
        )
        .unwrap();
        assert_eq!(path.file_name().unwrap(), "hello.md");
        let body = fs::read_to_string(&path).unwrap();
        assert_eq!(body, "# Lessons — Hello\n\n- first\n- second\n- third\n");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn empty_bullets_are_rejected() {
        let dir = temp_dir("empty");
        assert!(write_lesson(&dir, "hello", "Hello", &["  ".into()]).is_err());
        assert!(!dir.exists());
    }

    #[test]
    fn recent_lessons_are_newest_first_and_limited() {
        let dir = temp_dir("recent");
        write_lesson(&dir, "first", "First", &["one".into()]).unwrap();
        thread::sleep(Duration::from_millis(20));
        write_lesson(&dir, "second", "Second", &["two".into()]).unwrap();
        thread::sleep(Duration::from_millis(20));
        write_lesson(&dir, "third", "Third", &["three".into()]).unwrap();

        let lessons = recent_lessons(&dir, 2).unwrap();
        assert_eq!(lessons.len(), 2);
        assert_eq!(lessons[0].name, "third");
        assert_eq!(lessons[1].name, "second");
        assert!(lessons[0].body.contains("- three"));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rules_are_optional_but_not_silently_broken() {
        let dir = temp_dir("rules");
        assert_eq!(read_rules(&dir, "hello").unwrap(), None);

        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("hello.md"), "  \n").unwrap();
        assert_eq!(read_rules(&dir, "hello").unwrap(), None);

        fs::write(dir.join("hello.md"), "\n No unsafe code. \n").unwrap();
        assert_eq!(
            read_rules(&dir, "hello").unwrap().as_deref(),
            Some("No unsafe code.")
        );

        fs::write(dir.join("hello.md"), [0xff, 0xfe]).unwrap();
        assert!(read_rules(&dir, "hello").is_err());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn counts_lessons_once_per_task_and_canonicalizes() {
        let dir = temp_dir("count");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("a.md"),
            "# Lessons — A\n\n- Keep it  small.\n- keep IT small.\n",
        )
        .unwrap();
        fs::write(dir.join("b.md"), "## B\n* Keep it small.\n").unwrap();

        let key = canonical_lesson("Keep it small.");
        assert_eq!(key, "keep it small.");
        let counts = count_lessons(&dir).unwrap();
        assert_eq!(counts[&key].count, 2);
        assert_eq!(counts[&key].text, "Keep it small.");

        fs::write(dir.join("c.md"), "- KEEP IT SMALL.\n").unwrap();
        assert_eq!(count_lessons(&dir).unwrap()[&key].count, 3);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn paraphrases_count_as_one_lesson_with_a_stable_representative() {
        let dir = temp_dir("paraphrase");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("a.md"),
            "# Lessons — A\n\n- Ensure that the usage message is consistent and provides clear instructions.\n",
        )
        .unwrap();
        fs::write(
            dir.join("b.md"),
            "# Lessons — B\n\n- Keep the usage message consistent and clear.\n",
        )
        .unwrap();
        fs::write(
            dir.join("c.md"),
            "# Lessons — C\n\n- Print a consistent usage message with clear instructions.\n",
        )
        .unwrap();

        let counts = count_lessons(&dir).unwrap();
        assert_eq!(counts.len(), 1);
        let key = canonical_lesson(
            "Ensure that the usage message is consistent and provides clear instructions.",
        );
        assert_eq!(counts[&key].count, 3);
        assert_eq!(
            counts[&key].text,
            "Ensure that the usage message is consistent and provides clear instructions."
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unrelated_or_opposite_lessons_stay_separate() {
        assert!(!lessons_match(
            "Keep it small.",
            "Use cargo clippy with -D warnings."
        ));
        assert!(!lessons_match(
            "Do not use unsafe code.",
            "Use unsafe code."
        ));
        assert!(!lessons_match("Keep it small.", "Do not keep it small."));

        let dir = temp_dir("separate");
        fs::create_dir_all(&dir).unwrap();
        for (name, line) in [
            ("a.md", "Keep it small."),
            ("b.md", "Use cargo clippy with -D warnings."),
            ("c.md", "Do not use unsafe code."),
            ("d.md", "Use unsafe code."),
        ] {
            fs::write(dir.join(name), format!("# Lessons\n\n- {line}\n")).unwrap();
        }
        let counts = count_lessons(&dir).unwrap();
        assert_eq!(counts.len(), 4);
        assert_eq!(
            counts[&canonical_lesson("Do not use unsafe code.")].count,
            1
        );
        assert_eq!(counts[&canonical_lesson("Use unsafe code.")].count, 1);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn append_rule_creates_appends_and_dedupes() {
        let dir = temp_dir("append-rule");
        let path = append_rule(&dir, "hello", "Keep it small.").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "# Rules — hello\n\n- Keep it small.\n"
        );

        append_rule(&dir, "hello", "keep it  SMALL.").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "# Rules — hello\n\n- Keep it small.\n"
        );

        append_rule(&dir, "hello", "No unsafe code.").unwrap();
        let body = fs::read_to_string(&path).unwrap();
        assert_eq!(body.matches("Keep it small.").count(), 1);
        assert!(body.ends_with("- No unsafe code.\n"));

        append_rule(&dir, "hello", "Keep dependencies minimal.").unwrap();
        append_rule(&dir, "hello", "Keep the dependencies minimal.").unwrap();
        let body = fs::read_to_string(&path).unwrap();
        assert_eq!(body.matches("dependencies").count(), 1);
        assert!(body.ends_with("- Keep dependencies minimal.\n"));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_directory_is_an_empty_knowledge_base() {
        let dir = temp_dir("missing");
        assert!(recent_lessons(&dir, 5).unwrap().is_empty());
    }

    #[test]
    fn prompt_section_carries_the_lessons_and_a_spec_precedence_note() {
        assert_eq!(prompt_section(&[]), "");
        let section = prompt_section(&[Lesson {
            name: "hello".into(),
            body: "# Lessons — Hello\n\n- be small".into(),
        }]);
        assert!(section.contains("# Lessons from previous tasks"));
        assert!(section.contains("current spec and constraints win"));
        assert!(section.contains("--- hello.md ---"));
        assert!(section.contains("- be small"));
    }
}

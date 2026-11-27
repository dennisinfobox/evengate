use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Lessons fed into a prompt. Spec §7.8 says the five most recent.
pub const RECENT_LESSONS: usize = 5;

/// A lesson seen in this many tasks is proposed for promotion to `rules/`.
pub const PROMOTION_THRESHOLD: u32 = 3;

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

/// Stable identity for "the same lesson": bullet markers and whitespace are
/// irrelevant, and ASCII case is ignored. Punctuation is preserved.
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

/// Count each lesson once per task file, across every `*.md` in the lessons
/// directory. A missing directory is an empty knowledge base.
pub fn count_lessons(dir: &Path) -> Result<BTreeMap<String, LessonCount>, String> {
    let mut counts: BTreeMap<String, LessonCount> = BTreeMap::new();
    if !dir.is_dir() {
        return Ok(counts);
    }
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
            let trimmed = line.trim();
            let bullet = trimmed
                .strip_prefix("- ")
                .or_else(|| trimmed.strip_prefix("* "));
            let Some(bullet) = bullet else { continue };
            let key = canonical_lesson(bullet);
            if key.is_empty() || !seen.insert(key.clone()) {
                continue;
            }
            let entry = counts.entry(key).or_insert_with(|| LessonCount {
                text: bullet.split_whitespace().collect::<Vec<_>>().join(" "),
                count: 0,
            });
            entry.count = entry.count.saturating_add(1);
        }
    }
    Ok(counts)
}

/// Canonical lessons already present as bullets in a rules file.
pub fn rule_lessons(rules: Option<&str>) -> BTreeSet<String> {
    let mut lessons = BTreeSet::new();
    if let Some(text) = rules {
        for line in text.lines() {
            let trimmed = line.trim();
            let bullet = trimmed
                .strip_prefix("- ")
                .or_else(|| trimmed.strip_prefix("* "));
            if let Some(bullet) = bullet {
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
    if rule_lessons(Some(existing.as_str())).contains(&canonical_lesson(&lesson)) {
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
        let dir = std::env::temp_dir().join(format!(
            "evengate-knowledge-{tag}-{}",
            std::process::id()
        ));
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
        assert_eq!(
            body,
            "# Lessons — Hello\n\n- first\n- second\n- third\n"
        );
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

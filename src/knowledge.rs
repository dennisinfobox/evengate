use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Lessons fed into a prompt. Spec §7.8 says the five most recent.
pub const RECENT_LESSONS: usize = 5;

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

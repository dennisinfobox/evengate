use std::env;
use std::fs;
use std::io::{self, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

// Built from `sandbox/Dockerfile`; the official image lacks clippy/rustfmt.
pub const DEFAULT_IMAGE: &str = "evengate-sandbox:1.100";
const OUTPUT_CAP: usize = 1024 * 1024;
const TMPFS_SIZE: &str = "1g";

#[derive(Debug, Clone)]
pub struct RunOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

/// Runs a command in a disposable container: no network, a bind mount of a
/// throwaway copy of the project, a wall-clock timeout, and capped output.
pub struct Sandbox {
    image: String,
    timeout: Duration,
    memory: String,
    cpus: String,
    pids: String,
}

fn env_or(key: &str, default: &str) -> String {
    env::var(key)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| default.to_string())
}

impl Sandbox {
    pub fn from_env() -> Self {
        let image = env_or("EVENGATE_SANDBOX_IMAGE", DEFAULT_IMAGE);
        let timeout = env::var("EVENGATE_SANDBOX_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or(Duration::from_secs(300));
        let memory = env_or("EVENGATE_SANDBOX_MEMORY", "2g");
        let cpus = env_or("EVENGATE_SANDBOX_CPUS", "2");
        let pids = env_or("EVENGATE_SANDBOX_PIDS", "256");
        Sandbox {
            image,
            timeout,
            memory,
            cpus,
            pids,
        }
    }

    pub fn run(&self, workdir: &Path, argv: &[&str]) -> io::Result<RunOutput> {
        let dir = docker_path(workdir)?;
        let mut child = Command::new("docker")
            .arg("run")
            .arg("--rm")
            .arg("--init")
            .arg("--network=none")
            .arg("--memory")
            .arg(&self.memory)
            .arg("--memory-swap")
            .arg(&self.memory)
            .arg("--pids-limit")
            .arg(&self.pids)
            .arg("--cpus")
            .arg(&self.cpus)
            .arg("--cap-drop=ALL")
            .arg("--security-opt=no-new-privileges")
            .arg("--read-only")
            .arg("--tmpfs")
            .arg(format!("/tmp:rw,exec,size={TMPFS_SIZE}"))
            .arg("-v")
            .arg(format!("{dir}:/work:ro"))
            .arg("-w")
            .arg("/work")
            .arg("-e")
            .arg("CARGO_TARGET_DIR=/tmp/evengate-target")
            .arg("-e")
            .arg("CARGO_NET_OFFLINE=true")
            .arg(&self.image)
            .args(argv)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stdout_pipe = child.stdout.take().expect("piped stdout");
        let stderr_pipe = child.stderr.take().expect("piped stderr");
        let stdout_reader = thread::spawn(move || read_capped(stdout_pipe));
        let stderr_reader = thread::spawn(move || read_capped(stderr_pipe));

        let started = Instant::now();
        let mut timed_out = false;
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if started.elapsed() > self.timeout {
                timed_out = true;
                let _ = child.kill();
                break child.wait()?;
            }
            thread::sleep(Duration::from_millis(50));
        };

        let (stdout, stdout_truncated) = join_reader(stdout_reader);
        let (stderr, stderr_truncated) = join_reader(stderr_reader);

        Ok(RunOutput {
            exit_code: status.code().unwrap_or(-1),
            stdout: with_marker(stdout, stdout_truncated),
            stderr: with_marker(stderr, stderr_truncated),
            timed_out,
        })
    }
}

fn join_reader(handle: thread::JoinHandle<(String, bool)>) -> (String, bool) {
    handle.join().unwrap_or_default()
}

fn with_marker(text: String, truncated: bool) -> String {
    if truncated {
        format!("{text}\n[output truncated at 1 MiB]")
    } else {
        text
    }
}

fn read_capped<R: Read>(mut reader: R) -> (String, bool) {
    let mut kept = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut truncated = false;
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                let remaining = OUTPUT_CAP.saturating_sub(kept.len());
                if remaining == 0 {
                    truncated = true;
                } else {
                    let take = n.min(remaining);
                    kept.extend_from_slice(&chunk[..take]);
                    if take < n {
                        truncated = true;
                    }
                }
            }
            Err(_) => break,
        }
    }
    (String::from_utf8_lossy(&kept).into_owned(), truncated)
}

/// Path in the form a Docker bind mount accepts on the host platform.
pub fn docker_path(path: &Path) -> io::Result<String> {
    let canonical = fs::canonicalize(path)?;
    let mut text = canonical.to_string_lossy().replace('\\', "/");
    if let Some(stripped) = text.strip_prefix("//?/") {
        text = stripped.to_string();
    }
    Ok(text)
}

/// Copy a project into a sandbox directory, skipping repository and build
/// metadata. The copy is what the container sees; the original is untouched.
pub fn copy_tree(src: &Path, dst: &Path) -> io::Result<()> {
    if dst.exists() {
        fs::remove_dir_all(dst)?;
    }
    copy_recursive(src, dst)
}

fn copy_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == ".git" || name == "target" {
            continue;
        }
        let from = entry.path();
        let to = dst.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            copy_recursive(&from, &to)?;
        } else if file_type.is_file() {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn caps_output_and_keeps_reading() {
        let input = vec![b'x'; OUTPUT_CAP + 4096];
        let (text, truncated) = read_capped(Cursor::new(input));
        assert_eq!(text.len(), OUTPUT_CAP);
        assert!(truncated);
    }

    #[test]
    fn small_output_is_not_truncated() {
        let (text, truncated) = read_capped(Cursor::new(b"ok".to_vec()));
        assert_eq!(text, "ok");
        assert!(!truncated);
    }
}

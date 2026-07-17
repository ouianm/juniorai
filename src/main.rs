use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use uuid::Uuid;

#[derive(Parser)]
#[command(
    name = "juniorai",
    version,
    about = "A local-first debugging process analyzer"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Start {
        #[arg(long)]
        goal: String,
    },
    Run {
        command: String,
    },
    Stop,
    Autopsy,
}

#[derive(Debug, Serialize, Deserialize)]
struct Session {
    id: Uuid,
    goal: String,
    started_at: DateTime<Utc>,
    stopped_at: Option<DateTime<Utc>>,
    events: Vec<CommandEvent>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CommandEvent {
    timestamp: DateTime<Utc>,
    command: String,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    git_before: Option<GitSnapshot>,
    git_after: Option<GitSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GitSnapshot {
    branch: Option<String>,
    changed_files: Vec<String>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Start { goal } => start_session(goal),
        Commands::Run { command } => run_command(command),
        Commands::Stop => stop_session(),
        Commands::Autopsy => create_autopsy(),
    }
}

fn start_session(goal: String) -> Result<()> {
    let path = active_session_path()?;

    if path.exists() {
        bail!("A session is already active.");
    }

    let session = Session {
        id: Uuid::new_v4(),
        goal,
        started_at: Utc::now(),
        stopped_at: None,
        events: Vec::new(),
    };

    save_session(&path, &session)?;

    println!("JuniorAI session started.");
    println!("Goal: {}", session.goal);

    Ok(())
}

fn run_command(command: String) -> Result<()> {
    let path = active_session_path()?;

    if !path.exists() {
        bail!("No active session. Start one first.");
    }

    let mut session = load_session(&path)?;

    let git_before = capture_git_snapshot();

    let output = Command::new("zsh")
        .arg("-lc")
        .arg(&command)
        .output()
        .with_context(|| format!("Failed to execute command: {command}"))?;

    let git_after = capture_git_snapshot();

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    print!("{stdout}");
    eprint!("{stderr}");

    session.events.push(CommandEvent {
        timestamp: Utc::now(),
        command,
        exit_code: output.status.code(),
        stdout: truncate(stdout, 10_000),
        stderr: truncate(stderr, 10_000),
        git_before,
        git_after,
    });

    save_session(&path, &session)?;

    Ok(())
}

fn stop_session() -> Result<()> {
    let active_path = active_session_path()?;

    if !active_path.exists() {
        bail!("No active session.");
    }

    let mut session = load_session(&active_path)?;
    session.stopped_at = Some(Utc::now());

    let sessions_dir = data_dir()?.join("sessions");
    fs::create_dir_all(&sessions_dir)?;

    let completed_path = sessions_dir.join(format!("{}.json", session.id));

    save_session(&completed_path, &session)?;
    fs::remove_file(active_path)?;

    println!("Session stopped.");
    println!("Recorded commands: {}", session.events.len());

    Ok(())
}

fn create_autopsy() -> Result<()> {
    let session = latest_completed_session()?;

    println!();
    println!("DEBUGGING AUTOPSY");
    println!("==================");
    println!("Goal: {}", session.goal);
    println!("Commands recorded: {}", session.events.len());

    let failed_commands: Vec<&CommandEvent> = session
        .events
        .iter()
        .filter(|event| event.exit_code != Some(0))
        .collect();

    println!("Failed commands: {}", failed_commands.len());

    if failed_commands.is_empty() {
        println!();
        println!("Observation:");
        println!("No failing commands were recorded.");
    } else {
        println!();
        println!("Observed failures:");

        for event in failed_commands {
            println!("- `{}` exited with {:?}", event.command, event.exit_code);
        }
    }

    let repeated_commands = find_repeated_commands(&session.events);

    if !repeated_commands.is_empty() {
        println!();
        println!("Possible process pattern:");
        println!("The following commands were repeated:");

        for (command, count) in repeated_commands {
            println!("- `{command}`: {count} times");
        }

        println!("Confidence: Low");
        println!("Alternative explanation: repetition may have been intentional.");
    }

    let changed_file_events = find_git_changes(&session.events);

    if !changed_file_events.is_empty() {
        println!();
        println!("Observed repository changes:");

        for change in changed_file_events {
            println!("- `{}` changed the working tree:", change.command);

            for file in change.newly_changed_files {
                println!("  - {file}");
            }
        }
    }

    let untested_changes = find_untested_changes(&session.events);

    if !untested_changes.is_empty() {
        println!();
        println!("Possible process issue:");
        println!("Repository changes were recorded without a successful test afterwards.");

        for command in untested_changes {
            println!("- Last detected change: `{command}`");
        }

        println!("Confidence: Medium");
        println!("Alternative explanation: testing may have been performed outside JuniorAI.");
    }

    Ok(())
}

struct GitChangeObservation {
    command: String,
    newly_changed_files: Vec<String>,
}

fn capture_git_snapshot() -> Option<GitSnapshot> {
    let repository_check = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .ok()?;

    if !repository_check.status.success() {
        return None;
    }

    let branch_output = Command::new("git")
        .args(["branch", "--show-current"])
        .output()
        .ok()?;

    let status_output = Command::new("git")
        .args(["status", "--short"])
        .output()
        .ok()?;

    if !status_output.status.success() {
        return None;
    }

    let branch = String::from_utf8_lossy(&branch_output.stdout)
        .trim()
        .to_string();

    let changed_files = String::from_utf8_lossy(&status_output.stdout)
        .lines()
        .map(extract_git_status_path)
        .filter(|path| !path.is_empty())
        .collect();

    Some(GitSnapshot {
        branch: if branch.is_empty() {
            None
        } else {
            Some(branch)
        },
        changed_files,
    })
}

fn extract_git_status_path(line: &str) -> String {
    let path = line.get(3..).unwrap_or(line).trim();

    match path.rsplit_once(" -> ") {
        Some((_, new_path)) => new_path.to_string(),
        None => path.to_string(),
    }
}

fn find_git_changes(events: &[CommandEvent]) -> Vec<GitChangeObservation> {
    let mut observations = Vec::new();

    for event in events {
        let (Some(before), Some(after)) = (&event.git_before, &event.git_after) else {
            continue;
        };

        let newly_changed_files: Vec<String> = after
            .changed_files
            .iter()
            .filter(|file| !before.changed_files.contains(file))
            .cloned()
            .collect();

        if !newly_changed_files.is_empty() {
            observations.push(GitChangeObservation {
                command: event.command.clone(),
                newly_changed_files,
            });
        }
    }

    observations
}

fn find_repeated_commands(events: &[CommandEvent]) -> Vec<(String, usize)> {
    let mut counts = HashMap::new();

    for event in events {
        *counts.entry(event.command.clone()).or_insert(0) += 1;
    }

    let mut repeated: Vec<(String, usize)> =
        counts.into_iter().filter(|(_, count)| *count > 1).collect();

    repeated.sort_by(|a, b| b.1.cmp(&a.1));

    repeated
}

fn truncate(value: String, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn data_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    Ok(home.join(".juniorai"))
}

fn active_session_path() -> Result<PathBuf> {
    let directory = data_dir()?;
    fs::create_dir_all(&directory)?;
    Ok(directory.join("active-session.json"))
}

fn save_session(path: &Path, session: &Session) -> Result<()> {
    let json = serde_json::to_string_pretty(session)?;
    fs::write(path, json)?;
    Ok(())
}

fn load_session(path: &Path) -> Result<Session> {
    let json = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&json)?)
}

fn latest_completed_session() -> Result<Session> {
    let sessions_dir = data_dir()?.join("sessions");

    if !sessions_dir.exists() {
        bail!("No completed sessions found.");
    }

    let latest = fs::read_dir(sessions_dir)?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, entry.path()))
        })
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, path)| path)
        .context("No completed sessions found")?;

    load_session(&latest)
}
fn is_test_command(command: &str) -> bool {
    let normalized = command.trim().to_lowercase();

    normalized.starts_with("cargo test")
        || normalized.starts_with("npm test")
        || normalized.starts_with("npm run test")
        || normalized.starts_with("pnpm test")
        || normalized.starts_with("yarn test")
        || normalized.starts_with("pytest")
        || normalized.starts_with("python -m pytest")
        || normalized.starts_with("mvn test")
        || normalized.starts_with("./mvnw test")
        || normalized.starts_with("gradle test")
        || normalized.starts_with("./gradlew test")
        || normalized.starts_with("dotnet test")
        || normalized.starts_with("go test")
}

fn find_untested_changes(events: &[CommandEvent]) -> Vec<String> {
    let mut untested_changes = Vec::new();
    let mut pending_change: Option<String> = None;

    for event in events {
        let changed_repository = match (&event.git_before, &event.git_after) {
            (Some(before), Some(after)) => before.changed_files != after.changed_files,
            _ => false,
        };

        if changed_repository {
            pending_change = Some(event.command.clone());
        }

        if is_test_command(&event.command) && event.exit_code == Some(0) {
            pending_change = None;
        }
    }

    if let Some(command) = pending_change {
        untested_changes.push(command);
    }

    untested_changes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(files: &[&str]) -> GitSnapshot {
        GitSnapshot {
            branch: Some("main".to_string()),
            changed_files: files.iter().map(|file| file.to_string()).collect(),
        }
    }

    fn event(command: &str, exit_code: i32, before: &[&str], after: &[&str]) -> CommandEvent {
        CommandEvent {
            timestamp: Utc::now(),
            command: command.to_string(),
            exit_code: Some(exit_code),
            stdout: String::new(),
            stderr: String::new(),
            git_before: Some(snapshot(before)),
            git_after: Some(snapshot(after)),
        }
    }

    #[test]
    fn recognizes_common_test_commands() {
        assert!(is_test_command("cargo test"));
        assert!(is_test_command("cargo test --all"));
        assert!(is_test_command("pytest"));
        assert!(is_test_command("npm run test"));
        assert!(is_test_command("./gradlew test"));
        assert!(is_test_command("dotnet test"));

        assert!(!is_test_command("cargo build"));
        assert!(!is_test_command("git status"));
        assert!(!is_test_command("echo test"));
    }

    #[test]
    fn detects_repository_change_without_test() {
        let events = vec![event("touch example.txt", 0, &[], &["example.txt"])];

        let result = find_untested_changes(&events);

        assert_eq!(result, vec!["touch example.txt"]);
    }

    #[test]
    fn successful_test_clears_pending_change() {
        let events = vec![
            event("touch example.txt", 0, &[], &["example.txt"]),
            event("cargo test", 0, &["example.txt"], &["example.txt"]),
        ];

        let result = find_untested_changes(&events);

        assert!(result.is_empty());
    }

    #[test]
    fn failed_test_does_not_clear_pending_change() {
        let events = vec![
            event("touch example.txt", 0, &[], &["example.txt"]),
            event("cargo test", 101, &["example.txt"], &["example.txt"]),
        ];

        let result = find_untested_changes(&events);

        assert_eq!(result, vec!["touch example.txt"]);
    }

    #[test]
    fn detects_repeated_commands() {
        let events = vec![
            event("git status", 0, &[], &[]),
            event("cargo build", 0, &[], &[]),
            event("git status", 0, &[], &[]),
        ];

        let result = find_repeated_commands(&events);

        assert_eq!(result, vec![("git status".to_string(), 2)]);
    }
}

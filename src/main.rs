use anyhow::{bail, Context, Result};
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

    let output = Command::new("zsh")
        .arg("-lc")
        .arg(&command)
        .output()
        .with_context(|| format!("Failed to execute command: {command}"))?;

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
            println!(
                "- `{}` exited with {:?}",
                event.command, event.exit_code
            );
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
        println!(
            "Alternative explanation: repetition may have been intentional."
        );
    }

    Ok(())
}

fn find_repeated_commands(events: &[CommandEvent]) -> Vec<(String, usize)> {
    let mut counts = HashMap::new();

    for event in events {
        *counts.entry(event.command.clone()).or_insert(0) += 1;
    }

    let mut repeated: Vec<(String, usize)> = counts
        .into_iter()
        .filter(|(_, count)| *count > 1)
        .collect();

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
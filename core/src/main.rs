//! Infinite Improbability Drive CLI
//!
//! CLI tool for spawning sandboxed LLM instances.

use std::path::PathBuf;

use improbability_drive::runner::{ClaudeRunner, GeminiRunner, LLMRunner};
use improbability_drive::sandbox::{PhaseState, WorktreeSandbox};
use improbability_drive::spawn::Spawner;
use improbability_drive::{SandboxManifest, SpawnConfig, SpawnStatus};

/// Check if debug mode is enabled via environment variable.
fn is_debug_mode() -> bool {
    std::env::var("CRUISE_DEBUG")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
}

/// Check if fail-fast mode is enabled via environment variable.
fn is_fail_fast_mode() -> bool {
    std::env::var("CRUISE_FAIL_FAST")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
}

/// Print main usage information.
fn print_usage(program: &str) {
    eprintln!("Usage: {} <prompt>", program);
    eprintln!("       {} cruise <subcommand>", program);
    eprintln!("\nSpawns a sandboxed LLM instance with the given prompt.");
    eprintln!("\nCommands:");
    eprintln!("  cruise fix [--comment \"...\"]  Trigger fixer round (optionally inject comment)");
    eprintln!("  cruise cleanup                Force cleanup of phase sandbox");
    eprintln!("  cruise resume                 Resume monitoring after crash");
    eprintln!("\nEnvironment variables:");
    eprintln!("  SPAWN_RUNNER=claude|gemini  Select LLM runner (default: claude)");
}

/// Print cruise subcommand usage.
fn print_cruise_usage(program: &str) {
    eprintln!("Usage: {} cruise <subcommand>", program);
    eprintln!("\nSubcommands:");
    eprintln!("  fix [--comment \"...\"]  Trigger immediate poll and fixer round");
    eprintln!("  cleanup                Force cleanup of phase sandbox");
    eprintln!("  resume                 Resume monitoring after crash");
}

/// Parse --comment argument from args.
fn parse_comment_arg(args: &[String]) -> Option<String> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--comment" {
            return iter.next().cloned();
        }
    }
    None
}

/// Find the active phase sandbox state file.
fn find_phase_state() -> Option<PathBuf> {
    // Look for .cruise/phase-state.json in common locations
    let locations = vec![
        PathBuf::from(".cruise/phase-state.json"),
        std::env::temp_dir()
            .join("improbability-drive-sandboxes")
            .join(".cruise")
            .join("phase-state.json"),
    ];

    for path in locations {
        if path.exists() {
            return Some(path);
        }
    }

    // Search temp directory for any sandbox with state
    let sandbox_dir = std::env::temp_dir().join("improbability-drive-sandboxes");
    if sandbox_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&sandbox_dir) {
            for entry in entries.flatten() {
                let state_path = entry.path().join(".cruise").join("phase-state.json");
                if state_path.exists() {
                    return Some(state_path);
                }
            }
        }
    }

    None
}

/// Handle `cruise fix` command.
async fn handle_cruise_fix(comment: Option<String>) {
    tracing::info!(comment = ?comment, "cruise fix: triggering fixer round");

    // Find active phase sandbox
    let state_path = match find_phase_state() {
        Some(path) => path,
        None => {
            eprintln!("No active phase sandbox found.");
            eprintln!("Run cruise-control to start a planning session first.");
            std::process::exit(1);
        }
    };

    // Load state
    let state_json = match std::fs::read_to_string(&state_path) {
        Ok(json) => json,
        Err(e) => {
            eprintln!("Failed to read phase state: {}", e);
            std::process::exit(1);
        }
    };

    let state: PhaseState = match serde_json::from_str(&state_json) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to parse phase state: {}", e);
            std::process::exit(1);
        }
    };

    println!("Found active phase sandbox:");
    println!("  Branch: {}", state.branch_name);
    println!("  Phase: {}", state.phase);
    if let Some(ref pr_url) = state.pr_url {
        println!("  PR: {}", pr_url);
    }

    // If comment provided, show what would be injected
    if let Some(ref c) = comment {
        println!("\nInjecting comment: {}", c);
    }

    // TODO: Implement actual polling and fixer round
    // For now, just show status
    println!("\n[TODO] Would poll for new comments and trigger fixer round");
    println!("Pending comments: {:?}", state.pending_comment_ids);
}

/// Handle `cruise cleanup` command.
async fn handle_cruise_cleanup() {
    tracing::info!("cruise cleanup: forcing sandbox cleanup");

    // Find active phase sandbox
    let state_path = match find_phase_state() {
        Some(path) => path,
        None => {
            eprintln!("No active phase sandbox found.");
            std::process::exit(0); // Not an error - nothing to clean up
        }
    };

    // Load state to get sandbox path
    let state_json = match std::fs::read_to_string(&state_path) {
        Ok(json) => json,
        Err(e) => {
            eprintln!("Failed to read phase state: {}", e);
            std::process::exit(1);
        }
    };

    let state: PhaseState = match serde_json::from_str(&state_json) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to parse phase state: {}", e);
            std::process::exit(1);
        }
    };

    println!("Cleaning up phase sandbox:");
    println!("  Path: {:?}", state.sandbox_path);
    println!("  Branch: {}", state.branch_name);

    // Remove worktree
    let repo_path = std::env::current_dir().expect("failed to get current directory");
    let output = std::process::Command::new("git")
        .current_dir(&repo_path)
        .args(["worktree", "remove", "--force"])
        .arg(&state.sandbox_path)
        .output();

    match output {
        Ok(out) if out.status.success() => {
            println!("Worktree removed successfully.");

            // Delete branch
            let _ = std::process::Command::new("git")
                .current_dir(&repo_path)
                .args(["branch", "-D", &state.branch_name])
                .output();

            println!("Branch deleted.");
        }
        Ok(out) => {
            eprintln!(
                "Failed to remove worktree: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Err(e) => {
            eprintln!("Failed to run git: {}", e);
        }
    }
}

/// Handle `cruise resume` command.
async fn handle_cruise_resume() {
    tracing::info!("cruise resume: resuming monitoring");

    // Find active phase sandbox
    let state_path = match find_phase_state() {
        Some(path) => path,
        None => {
            eprintln!("No active phase sandbox found to resume.");
            std::process::exit(1);
        }
    };

    // Load state
    let state_json = match std::fs::read_to_string(&state_path) {
        Ok(json) => json,
        Err(e) => {
            eprintln!("Failed to read phase state: {}", e);
            std::process::exit(1);
        }
    };

    let state: PhaseState = match serde_json::from_str(&state_json) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to parse phase state: {}", e);
            std::process::exit(1);
        }
    };

    println!("Resuming phase sandbox monitoring:");
    println!("  Path: {:?}", state.sandbox_path);
    println!("  Branch: {}", state.branch_name);
    println!("  Phase: {}", state.phase);
    println!("  Completed rounds: {}", state.completed_rounds);
    if let Some(ref pr_url) = state.pr_url {
        println!("  PR: {}", pr_url);
    }

    // TODO: Implement actual monitoring loop
    // For now, just show status
    println!("\n[TODO] Would start monitoring loop with backoff");
    println!("Current backoff: {}s", state.backoff_interval_secs);
}

#[tokio::main]
async fn main() {
    let debug_mode = is_debug_mode();
    let fail_fast = is_fail_fast_mode();

    // Initialize tracing with debug level if CRUISE_DEBUG is set
    let log_level = if debug_mode {
        tracing::Level::DEBUG
    } else {
        tracing::Level::INFO
    };

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(log_level.into()),
        )
        .init();

    if debug_mode {
        eprintln!("=== CRUISE DEBUG MODE ENABLED ===");
        eprintln!("CRUISE_FAIL_FAST: {}", fail_fast);
    }

    // Parse args (basic for now - will add clap in later phase)
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        print_usage(&args[0]);
        std::process::exit(1);
    }

    // Handle cruise subcommands
    if args[1] == "cruise" {
        if args.len() < 3 {
            print_cruise_usage(&args[0]);
            std::process::exit(1);
        }

        match args[2].as_str() {
            "fix" => {
                let comment = parse_comment_arg(&args[3..]);
                handle_cruise_fix(comment).await;
                return;
            }
            "cleanup" => {
                handle_cruise_cleanup().await;
                return;
            }
            "resume" => {
                handle_cruise_resume().await;
                return;
            }
            _ => {
                eprintln!("Unknown cruise subcommand: {}", args[2]);
                print_cruise_usage(&args[0]);
                std::process::exit(1);
            }
        }
    }

    let prompt = args[1..].join(" ");

    // Get current repo path
    let repo_path = std::env::current_dir().expect("failed to get current directory");

    // Setup directories
    let logs_dir = PathBuf::from(".improbability-drive/spawns");
    let sandbox_dir = std::env::temp_dir().join("improbability-drive-sandboxes");

    // Select runner based on environment variable
    let runner_name = std::env::var("SPAWN_RUNNER").unwrap_or_else(|_| "claude".to_string());
    let runner: Box<dyn LLMRunner> = match runner_name.as_str() {
        "gemini" => {
            tracing::info!("using Gemini runner");
            Box::new(GeminiRunner::new())
        }
        _ => {
            tracing::info!("using Claude runner");
            Box::new(ClaudeRunner::new())
        }
    };

    // Create spawner
    let provider = WorktreeSandbox::new(repo_path, Some(sandbox_dir));
    let spawner = Spawner::new(provider, logs_dir);

    // Create config
    let config = SpawnConfig::new(&prompt);
    let manifest = SandboxManifest::default();

    // Run spawn
    tracing::info!(prompt = %prompt, "starting spawn");

    match spawner.spawn(config, manifest, runner).await {
        Ok(result) => {
            println!("\n{}", "=".repeat(60));
            println!("Spawn Complete: {}", result.spawn_id);
            println!("{}", "=".repeat(60));
            println!();
            println!("Status: {:?}", result.status);
            println!("Duration: {:?}", result.duration);
            println!();
            println!("Summary:");
            println!("  {}", result.summary);
            println!();
            println!("Logs: {}", result.logs.stdout.parent().unwrap().display());

            if result.status != SpawnStatus::Success {
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("Spawn failed: {}", e);
            std::process::exit(1);
        }
    }
}

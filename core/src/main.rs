//! Infinite Improbability Drive CLI
//!
//! CLI tool for spawning sandboxed LLM instances.

use std::path::PathBuf;

use improbability_drive::sandbox::WorktreeSandbox;
use improbability_drive::spawn::Spawner;
use improbability_drive::{SandboxManifest, SpawnConfig, SpawnStatus};

fn main() {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    // Parse args (basic for now - will add clap in later phase)
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: {} <prompt>", args[0]);
        eprintln!("\nSpawns a sandboxed LLM instance with the given prompt.");
        std::process::exit(1);
    }

    let prompt = args[1..].join(" ");

    // Get current repo path
    let repo_path = std::env::current_dir().expect("failed to get current directory");

    // Setup directories
    let logs_dir = PathBuf::from(".improbability-drive/spawns");
    let sandbox_dir = std::env::temp_dir().join("improbability-drive-sandboxes");

    // Create spawner
    let provider = WorktreeSandbox::new(repo_path, Some(sandbox_dir));
    let spawner = Spawner::new(provider, logs_dir);

    // Create config
    let config = SpawnConfig::new(&prompt);
    let manifest = SandboxManifest::default();

    // Run spawn
    tracing::info!(prompt = %prompt, "starting spawn");

    match spawner.spawn(config, manifest) {
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

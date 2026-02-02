//! Infinite Improbability Drive CLI
//!
//! CLI tool for spawning sandboxed LLM instances.

use std::path::PathBuf;

use improbability_drive::runner::{ClaudeRunner, GeminiRunner, LLMRunner};
use improbability_drive::sandbox::WorktreeSandbox;
use improbability_drive::spawn::Spawner;
use improbability_drive::{SandboxManifest, SpawnConfig, SpawnStatus};

#[tokio::main]
async fn main() {
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
        eprintln!("\nEnvironment variables:");
        eprintln!("  SPAWN_RUNNER=claude|gemini  Select LLM runner (default: claude)");
        std::process::exit(1);
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

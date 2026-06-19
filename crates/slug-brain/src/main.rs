//! `slug-agent` — drive the Slug semantic bus with a hybrid local/cloud agent.
//!
//! ```text
//! slug-agent "open the Open dialog in the text editor"   # run a task
//! slug-agent --probe                                     # hardware report only
//! slug-agent --write-config                              # print a default slug.toml
//! slug-agent --backend cloud "..."                       # force a backend
//! ```

use std::path::PathBuf;

use clap::Parser;
use slug_brain::config::Selection;
use slug_brain::hardware::{self, SystemProbe};
use slug_brain::{Brain, Config};

#[derive(Parser)]
#[command(name = "slug-agent", version, about = "Hybrid local/cloud agent for the Slug semantic bus")]
struct Cli {
    /// The task description for the agent to carry out.
    task: Option<String>,

    /// Print the hardware "Can I run it?" report and exit.
    #[arg(long)]
    probe: bool,

    /// Print a default slug.toml to stdout and exit.
    #[arg(long)]
    write_config: bool,

    /// Path to slug.toml (default: ./slug.toml).
    #[arg(long, default_value = "slug.toml")]
    config: PathBuf,

    /// Override backend selection: auto | local | cloud.
    #[arg(long)]
    backend: Option<String>,

    /// Don't prompt for destructive-action confirmation (auto-deny instead).
    #[arg(long)]
    non_interactive: bool,

    /// Stream the run as JSON-lines events on stdout (status/step/final), for the
    /// MCP dashboard's agent controller. Implies --non-interactive.
    #[arg(long)]
    jsonl: bool,
}

/// Print one JSON-lines event to stdout, flushed.
fn emit_jsonl(event: &serde_json::Value) {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{event}");
    let _ = out.flush();
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let cli = Cli::parse();

    if cli.write_config {
        print!("{}", Config::default().to_toml());
        return Ok(());
    }

    let probe = SystemProbe::detect();
    let report = hardware::assess(&probe);

    if cli.probe {
        print!("{report}");
        return Ok(());
    }

    let mut cfg = Config::load(&cli.config)?;
    if let Some(sel) = cli.backend.as_deref() {
        cfg.backend.selection = match sel {
            "auto" => Selection::Auto,
            "local" => Selection::Local,
            "cloud" => Selection::Cloud,
            other => anyhow::bail!("unknown --backend '{other}' (use auto|local|cloud)"),
        };
    }

    let Some(task) = cli.task else {
        // No task: show the report so the user knows what would run.
        print!("{report}");
        eprintln!("\nProvide a task, e.g.: slug-agent \"click the Open button\"");
        return Ok(());
    };

    let interactive = !cli.non_interactive && !cli.jsonl;
    let mut brain = Brain::from_config(&cfg, &report, interactive)?;

    if cli.jsonl {
        use slug_brain::config::Provider;
        // Resolve `auto` to a concrete provider for the status line.
        let resolved = match cfg.brain.provider {
            Provider::Auto => match cfg.backend.selection {
                Selection::Cloud => Provider::Claude,
                Selection::Local => Provider::Ollama,
                Selection::Auto => {
                    if report.backend == slug_brain::hardware::BackendKind::Cloud {
                        Provider::Claude
                    } else {
                        Provider::Ollama
                    }
                }
            },
            other => other,
        };
        let provider = serde_json::to_value(resolved)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| "auto".to_string());
        let cfg_model = cfg.resolved_provider(resolved).model;
        let model = if cfg_model.is_empty() { report.model.clone() } else { cfg_model };
        emit_jsonl(&serde_json::json!({
            "kind": "status",
            "provider": provider,
            "tier": format!("{:?}", report.tier),
            "model": model,
        }));
        emit_jsonl(&serde_json::json!({ "kind": "task", "description": task }));
        brain = brain.with_observer(Box::new(|ev| emit_jsonl(&ev)));
    }

    let outcome = brain.run(&task).await?;

    if !cli.jsonl {
        println!("\n{}", outcome.answer);
    }
    eprintln!(
        "\n[slug-agent] {} step(s), {} tokens, ${:.4}{}",
        outcome.steps,
        outcome.tokens,
        outcome.cost_usd,
        if outcome.escalated { " (escalated)" } else { "" },
    );
    if outcome.escalated {
        std::process::exit(1);
    }
    Ok(())
}

fn init_tracing() {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("slug_brain=info,slug_bridge=info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(std::io::stderr))
        .init();
}

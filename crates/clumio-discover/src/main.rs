use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context};
use cd_model::{Config, Depth, Inventory};
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "clumio-discover",
    version,
    about = "Read-only discovery of the Clumio-protectable resources in one AWS account."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// Config file [default: ~/.config/clumio-discover/config.toml]
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    /// AWS profile; overrides the config and AWS_PROFILE
    #[arg(long, global = true)]
    profile: Option<String>,
    /// Write the inventory JSON here instead of opening the TUI
    #[arg(long, global = true)]
    out: Option<PathBuf>,
    /// Don't save the inventory JSON after a scan in the TUI
    #[arg(long, global = true)]
    no_save: bool,
    /// Print the inventory JSON to stdout even on a terminal
    #[arg(long, global = true)]
    json: bool,
    /// Also check every snapshot and AMI for public sharing (slower on large accounts)
    #[arg(long, global = true)]
    deep: bool,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Scan the account (the default): TUI on a terminal, JSON otherwise
    #[command(alias = "discover")]
    Scan,
    /// Browse a saved inventory; needs no AWS credentials
    View { file: PathBuf },
    /// Print the read-only IAM policy this build needs
    Policy,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Scan) {
        Command::Scan => {
            let output = Output {
                out: cli.out,
                json: cli.json,
                save: !cli.no_save,
            };
            scan(cli.config, cli.profile, output, cli.deep).await
        }
        Command::View { file } => view(&file),
        Command::Policy => {
            println!("{}", cd_aws::policy_document());
            Ok(())
        }
    }
}

struct Output {
    out: Option<PathBuf>,
    json: bool,
    save: bool,
}

async fn scan(
    config: Option<PathBuf>,
    profile: Option<String>,
    output: Output,
    deep: bool,
) -> anyhow::Result<()> {
    let Output { out, json, save } = output;
    let mut config = Config::load(config.as_deref())?;
    if deep {
        config.depth = Depth::Deep;
    }
    let profile = profile.or_else(|| config.profile.clone());

    if out.is_some() || json || !std::io::stdout().is_terminal() {
        let inventory = cd_aws::discover(&config, profile.as_deref(), None).await?;
        return report(&inventory, out.as_deref());
    }

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let handle = tokio::spawn(async move {
        cd_aws::discover(&config, profile.as_deref(), Some(tx))
            .await
            .map_err(|e| e.to_string())
    });
    match cd_tui::scan(rx, handle, save).await? {
        Some(scanned) => {
            summarize(&scanned.inventory);
            if let Some(path) = scanned.saved {
                eprintln!("inventory saved to {}", path.display());
            }
        }
        None => eprintln!("scan aborted; nothing written"),
    }
    Ok(())
}

fn view(file: &Path) -> anyhow::Result<()> {
    let raw = std::fs::read_to_string(file)
        .with_context(|| format!("failed to read {}", file.display()))?;
    let inventory: Inventory = serde_json::from_str(&raw)
        .with_context(|| format!("{} is not a clumio-discover inventory", file.display()))?;
    if !std::io::stdout().is_terminal() {
        bail!("view needs a terminal");
    }
    cd_tui::view(inventory)?;
    Ok(())
}

fn report(inventory: &Inventory, out: Option<&Path>) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(inventory)?;
    match out {
        Some(path) => std::fs::write(path, format!("{json}\n"))
            .with_context(|| format!("failed to write {}", path.display()))?,
        None => println!("{json}"),
    }

    summarize(inventory);
    if let Some(path) = out {
        eprintln!("inventory written to {}", path.display());
    }

    for error in &inventory.errors {
        eprintln!(
            "error: {}:{} {}{}",
            error.service,
            error.operation,
            error
                .region
                .as_deref()
                .map(|r| format!("[{r}] "))
                .unwrap_or_default(),
            error.message
        );
    }
    if !inventory.errors.is_empty() {
        bail!(
            "discovery incomplete: {} error(s), listed under `errors` in the inventory",
            inventory.errors.len()
        );
    }
    Ok(())
}

fn summarize(inventory: &Inventory) {
    let summary: Vec<String> = inventory
        .summary()
        .into_iter()
        .map(|(service, count)| format!("{}={count}", service.as_str()))
        .collect();
    eprintln!(
        "account {} regions {}: {}",
        inventory.account.id,
        inventory.regions.join(","),
        summary.join(" ")
    );
    eprintln!(
        "{} finding(s), {} error(s)",
        inventory.findings.len(),
        inventory.errors.len()
    );
    if inventory.depth == Depth::Standard {
        eprintln!("standard scan: snapshots were not checked for public sharing; run with --deep to include them");
    }
}

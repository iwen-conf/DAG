mod client;
mod config;
mod project_resolve;
mod tui;

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use serde_json::json;

use crate::client::ApiClient;
use crate::config::{load_global, save_global, GlobalConfig};
use crate::project_resolve::resolve_project;
use crate::tui::{config_interactive, run_projects_tui, ProjectRow};

#[derive(Parser, Debug)]
#[command(name = "sunmao", about = "sunmao human CLI (D-20)")]
struct Cli {
    /// Override base URL
    #[arg(long, global = true)]
    base_url: Option<String>,

    /// Explicit project id
    #[arg(long, global = true)]
    project: Option<String>,

    /// JSON output (script-friendly)
    #[arg(long, global = true, default_value_t = false)]
    json: bool,

    #[command(subcommand)]
    cmd: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// TUI / set base_url → ~/.config/sunmao/config.toml
    Config {
        /// Non-interactive: set base_url directly
        #[arg(long)]
        base_url: Option<String>,
    },
    /// List projects (TUI or --json)
    Projects,
    /// Register cwd git root as project + write .sunmao.toml
    Init {
        #[arg(long)]
        name: Option<String>,
    },
    /// Show task tree for current project
    Tree,
    /// Task show
    Task {
        #[command(subcommand)]
        sub: TaskCmd,
    },
    /// Package scope ops
    Scope {
        action: String,
        package_id: String,
        #[arg(long)]
        reason: Option<String>,
        #[arg(long, default_value_t = false)]
        force: bool,
        #[arg(long, default_value_t = false)]
        permanent: bool,
    },
    /// Approve major contract
    ApproveMajor { artifact_id: String },
    /// Admin verify ready consistency
    Admin {
        #[command(subcommand)]
        sub: AdminCmd,
    },
}

#[derive(Subcommand, Debug)]
enum TaskCmd {
    Show { id: String },
}

#[derive(Subcommand, Debug)]
enum AdminCmd {
    Verify,
    Rebuild,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let global = load_global().unwrap_or_default();
    let base = cli
        .base_url
        .clone()
        .or(global.base_url.clone())
        .unwrap_or_else(|| "http://127.0.0.1:7420".into());
    let client = ApiClient::new(&base);

    match cli.cmd {
        Commands::Config { base_url } => {
            if let Some(u) = base_url {
                let cfg = GlobalConfig {
                    base_url: Some(u.clone()),
                };
                save_global(&cfg)?;
                println!("saved base_url = {u}");
            } else if cli.json {
                println!("{}", serde_json::to_string_pretty(&global)?);
            } else {
                let u = config_interactive(&base)?;
                println!("saved base_url = {u}");
            }
        }
        Commands::Projects => {
            let v = client.get("/v1/projects").await?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                let rows: Vec<ProjectRow> = v
                    .get("projects")
                    .and_then(|p| p.as_array())
                    .map(|arr| {
                        arr.iter()
                            .map(|p| ProjectRow {
                                id: p
                                    .get("id")
                                    .and_then(|x| x.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                                name: p
                                    .get("name")
                                    .and_then(|x| x.as_str())
                                    .unwrap_or("?")
                                    .to_string(),
                                repo_path: p
                                    .get("repo_path")
                                    .and_then(|x| x.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if rows.is_empty() {
                    println!("(no projects — run `sunmao init` in a git repo)");
                } else if let Some(id) = run_projects_tui(&rows)? {
                    println!("{id}");
                }
            }
        }
        Commands::Init { name } => {
            let root = git_root(std::env::current_dir()?)?;
            let name = name.unwrap_or_else(|| {
                root.file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "project".into())
            });
            let body = json!({
                "name": name,
                "repo_path": root.display().to_string(),
            });
            let p = client.post("/v1/projects", &body).await?;
            let pid = p
                .get("id")
                .and_then(|x| x.as_str())
                .context("project id")?
                .to_string();
            let toml = format!(
                "project_id = \"{pid}\"\nbase_url = \"{base}\"\n"
            );
            std::fs::write(root.join(".sunmao.toml"), toml)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&p)?);
            } else {
                println!("initialized project {pid} at {}", root.display());
            }
        }
        Commands::Tree => {
            let pid = resolve_project(&client, cli.project.as_deref(), cli.json).await?;
            let g = client
                .get(&format!("/v1/projects/{pid}/graph"))
                .await?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&g)?);
            } else if let Some(nodes) = g.get("nodes").and_then(|n| n.as_array()) {
                for n in nodes {
                    let kind = n.get("kind").and_then(|x| x.as_str()).unwrap_or("?");
                    let id = n.get("id").and_then(|x| x.as_str()).unwrap_or("?");
                    let title = n.get("title").and_then(|x| x.as_str()).unwrap_or("?");
                    let st = n
                        .get("task_state")
                        .and_then(|x| x.as_str())
                        .or_else(|| n.get("scope_state").and_then(|x| x.as_str()))
                        .unwrap_or("-");
                    let path = n.get("path").and_then(|x| x.as_str()).unwrap_or("");
                    println!("{kind}\t{st}\t{path}\t{id}\t{title}");
                }
            }
        }
        Commands::Task { sub } => match sub {
            TaskCmd::Show { id } => {
                let pid = resolve_project(&client, cli.project.as_deref(), cli.json).await?;
                let t = client
                    .get(&format!("/v1/projects/{pid}/tasks/{id}"))
                    .await?;
                println!("{}", serde_json::to_string_pretty(&t)?);
            }
        },
        Commands::Scope {
            action,
            package_id,
            reason,
            force,
            permanent,
        } => {
            let pid = resolve_project(&client, cli.project.as_deref(), cli.json).await?;
            let body = json!({
                "action": action,
                "reason": reason.unwrap_or_else(|| "cli".into()),
                "force": force,
                "permanent": permanent,
            });
            let r = client
                .post(
                    &format!("/v1/projects/{pid}/packages/{package_id}/scope"),
                    &body,
                )
                .await?;
            println!("{}", serde_json::to_string_pretty(&r)?);
        }
        Commands::ApproveMajor { artifact_id } => {
            let pid = resolve_project(&client, cli.project.as_deref(), cli.json).await?;
            let r = client
                .post(
                    &format!("/v1/projects/{pid}/contracts/{artifact_id}/approve-major"),
                    &json!({}),
                )
                .await?;
            println!("{}", serde_json::to_string_pretty(&r)?);
        }
        Commands::Admin { sub } => {
            let pid = resolve_project(&client, cli.project.as_deref(), cli.json).await?;
            match sub {
                AdminCmd::Verify => {
                    let r = client
                        .post(&format!("/v1/projects/{pid}/admin/verify"), &json!({}))
                        .await?;
                    println!("{}", serde_json::to_string_pretty(&r)?);
                }
                AdminCmd::Rebuild => {
                    let r = client
                        .post(
                            &format!("/v1/projects/{pid}/admin/rebuild-projection"),
                            &json!({}),
                        )
                        .await?;
                    println!("{}", serde_json::to_string_pretty(&r)?);
                }
            }
        }
    }
    Ok(())
}

fn git_root(start: PathBuf) -> Result<PathBuf> {
    let mut cur = start;
    loop {
        if cur.join(".git").exists() {
            return Ok(std::fs::canonicalize(&cur)?);
        }
        if !cur.pop() {
            bail!("not inside a git repository");
        }
    }
}

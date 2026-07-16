use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::client::ApiClient;
use crate::tui::{run_projects_tui, ProjectRow};

#[derive(Debug, Deserialize)]
struct LocalBind {
    project_id: String,
    #[allow(dead_code)]
    base_url: Option<String>,
}

/// --project > .sunmao.toml > git root lookup > interactive list
pub async fn resolve_project(
    client: &ApiClient,
    explicit: Option<&str>,
    json_mode: bool,
) -> Result<String> {
    if let Some(p) = explicit {
        return Ok(p.to_string());
    }
    if let Some(id) = read_local_bind()? {
        return Ok(id);
    }
    if let Ok(root) = git_root(std::env::current_dir()?) {
        let q = urlencoding_simple(&root.display().to_string());
        if let Ok(v) = client
            .get(&format!("/v1/projects/lookup?repo_path={q}"))
            .await
        {
            if let Some(id) = v.get("id").and_then(|x| x.as_str()) {
                return Ok(id.to_string());
            }
        }
    }
    // fallback: list and pick
    let v = client.get("/v1/projects").await?;
    let projects = v
        .get("projects")
        .and_then(|p| p.as_array())
        .cloned()
        .unwrap_or_default();
    if projects.is_empty() {
        bail!("no projects; run `sunmao init` in a git repo");
    }
    if projects.len() == 1 {
        return Ok(projects[0]
            .get("id")
            .and_then(|x| x.as_str())
            .context("id")?
            .to_string());
    }
    if json_mode {
        // non-interactive multi-project: require --project
        bail!("multiple projects; pass --project <id> (or use interactive TUI without --json)");
    }
    let rows: Vec<ProjectRow> = projects
        .iter()
        .map(|p| ProjectRow {
            id: p.get("id").and_then(|x| x.as_str()).unwrap_or("").into(),
            name: p.get("name").and_then(|x| x.as_str()).unwrap_or("?").into(),
            repo_path: p
                .get("repo_path")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .into(),
        })
        .collect();
    run_projects_tui(&rows)?.context("project selection cancelled")
}

fn read_local_bind() -> Result<Option<String>> {
    let mut cur = std::env::current_dir()?;
    loop {
        let f = cur.join(".sunmao.toml");
        if f.exists() {
            let s = std::fs::read_to_string(&f)?;
            let b: LocalBind = toml::from_str(&s)?;
            return Ok(Some(b.project_id));
        }
        if !cur.pop() {
            return Ok(None);
        }
    }
}

fn git_root(start: PathBuf) -> Result<PathBuf> {
    let mut cur = start;
    loop {
        if cur.join(".git").exists() {
            return Ok(std::fs::canonicalize(cur)?);
        }
        if !cur.pop() {
            bail!("no git root");
        }
    }
}

fn urlencoding_simple(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

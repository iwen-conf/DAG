//! GitWorkspace: status/diff/add/commit via git CLI (D-09=A).

use std::path::{Path, PathBuf};
use std::process::Command;

use sunmao_core::write_scope::{path_in_scopes, scopes_conflict};

use crate::error::{StoreError, StoreResult};

#[derive(Debug, Clone)]
pub struct GitWorkspace {
    pub repo_path: PathBuf,
}

#[derive(Debug, Clone, Default)]
pub struct StatusEntry {
    pub path: String,
    pub status: String, // e.g. " M", "??", "A "
}

#[derive(Debug, Clone, Default)]
pub struct ScopeDiff {
    pub in_scope: Vec<String>,
    pub out_scope: Vec<String>,
}

impl GitWorkspace {
    pub fn new(repo_path: impl Into<PathBuf>) -> Self {
        Self {
            repo_path: repo_path.into(),
        }
    }

    pub fn ensure_repo(&self) -> StoreResult<()> {
        if self.repo_path.join(".git").exists() {
            return Ok(());
        }
        self.run(&["init"])?;
        // identity for commits in test/CI
        let _ = self.run(&["config", "user.email", "sunmao@local"]);
        let _ = self.run(&["config", "user.name", "sunmao"]);
        Ok(())
    }

    pub fn status_porcelain(&self) -> StoreResult<Vec<StatusEntry>> {
        // -uall: expand untracked dirs to file paths so write_scope prefix checks work
        // (plain --porcelain reports "?? identity/" for a new tree, which fails prefix match).
        let out = self.run_output(&["status", "--porcelain", "-uall"])?;
        let mut entries = Vec::new();
        for line in out.lines() {
            if line.len() < 4 {
                continue;
            }
            let status = line[..2].to_string();
            let path = line[3..].trim().to_string();
            // handle renames "R  a -> b"
            let path = if let Some((_, b)) = path.split_once(" -> ") {
                b.to_string()
            } else {
                path
            };
            entries.push(StatusEntry { path, status });
        }
        Ok(entries)
    }

    /// Partition changes relative to write_scope; ignore neighbors' running scopes.
    pub fn partition_by_scope(
        &self,
        write_scope: &[String],
        neighbor_scopes: &[Vec<String>],
    ) -> StoreResult<ScopeDiff> {
        let entries = self.status_porcelain()?;
        let mut diff = ScopeDiff::default();
        for e in entries {
            if path_in_scopes(&e.path, write_scope) {
                diff.in_scope.push(e.path);
            } else {
                // neighbor in-progress? ignore
                let neighbor = neighbor_scopes
                    .iter()
                    .any(|ns| path_in_scopes(&e.path, ns));
                if !neighbor {
                    diff.out_scope.push(e.path);
                }
            }
        }
        Ok(diff)
    }

    pub fn commit_paths(&self, paths: &[String], message: &str) -> StoreResult<String> {
        if paths.is_empty() {
            // empty commit not allowed unless detecting prior commit
            return Err(StoreError::Git("no paths to commit".into()));
        }
        for p in paths {
            self.run(&["add", "--", p])?;
        }
        self.run(&["commit", "-m", message])?;
        let hash = self.run_output(&["rev-parse", "HEAD"])?;
        Ok(hash.trim().to_string())
    }

    pub fn checkout_paths(&self, paths: &[String]) -> StoreResult<()> {
        if paths.is_empty() {
            return Ok(());
        }
        let mut args = vec!["checkout", "--"];
        let owned: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
        args.extend(owned);
        self.run(&args)?;
        // also clean untracked in those paths
        for p in paths {
            let full = self.repo_path.join(p);
            if full.exists() {
                let st = self.run_output(&["status", "--porcelain", "--", p])?;
                if st.starts_with("??") {
                    let _ = std::fs::remove_file(&full);
                }
            }
        }
        Ok(())
    }

    pub fn head_contains_message(&self, needle: &str) -> StoreResult<bool> {
        let log = self.run_output(&["log", "-20", "--pretty=%s"])?;
        Ok(log.lines().any(|l| l.contains(needle)))
    }

    pub fn find_commit_by_message(&self, needle: &str) -> StoreResult<Option<String>> {
        let log = self.run_output(&["log", "-50", "--pretty=%H %s"])?;
        for line in log.lines() {
            if let Some((hash, msg)) = line.split_once(' ') {
                if msg.contains(needle) {
                    return Ok(Some(hash.to_string()));
                }
            }
        }
        Ok(None)
    }

    pub fn tree_digest(&self, paths: &[String]) -> StoreResult<String> {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        for p in paths {
            let full = self.repo_path.join(p);
            if let Ok(bytes) = std::fs::read(&full) {
                p.hash(&mut h);
                bytes.hash(&mut h);
            }
        }
        Ok(format!("{:x}", h.finish()))
    }

    pub fn write_file(&self, rel: &str, content: &str) -> StoreResult<()> {
        let full = self.repo_path.join(rel);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).map_err(|e| StoreError::Git(e.to_string()))?;
        }
        std::fs::write(&full, content).map_err(|e| StoreError::Git(e.to_string()))?;
        Ok(())
    }

    fn run(&self, args: &[&str]) -> StoreResult<()> {
        let status = Command::new("git")
            .args(args)
            .current_dir(&self.repo_path)
            .status()
            .map_err(|e| StoreError::Git(e.to_string()))?;
        if !status.success() {
            return Err(StoreError::Git(format!("git {args:?} failed: {status}")));
        }
        Ok(())
    }

    fn run_output(&self, args: &[&str]) -> StoreResult<String> {
        let out = Command::new("git")
            .args(args)
            .current_dir(&self.repo_path)
            .output()
            .map_err(|e| StoreError::Git(e.to_string()))?;
        if !out.status.success() {
            return Err(StoreError::Git(format!(
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            )));
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    pub fn path(&self) -> &Path {
        &self.repo_path
    }
}

/// Check if any write_scope pair conflicts (for claim-time refinement).
pub fn any_scope_conflict(a: &[String], others: &[Vec<String>]) -> bool {
    others.iter().any(|o| scopes_conflict(a, o))
}

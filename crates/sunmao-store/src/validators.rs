//! Built-in validators (FR-08.1): scope-diff / cmd / artifact-exists.

use serde::{Deserialize, Serialize};
use std::process::Command;

use crate::error::{StoreError, StoreResult};
use crate::git::GitWorkspace;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorFailure {
    pub validator: String,
    pub report: String,
}

pub fn run_validators(
    names: &[String],
    git: &GitWorkspace,
    write_scope: &[String],
    in_scope_paths: &[String],
    out_scope_paths: &[String],
) -> StoreResult<Vec<ValidatorFailure>> {
    let mut failures = Vec::new();
    for name in names {
        match name.as_str() {
            "scope-diff" => {
                if !out_scope_paths.is_empty() {
                    failures.push(ValidatorFailure {
                        validator: "scope-diff".into(),
                        report: format!(
                            "越界写入: {} 不在声明范围 {:?}",
                            out_scope_paths.join(", "),
                            write_scope
                        ),
                    });
                }
            }
            "artifact-exists" => {
                for p in in_scope_paths {
                    let full = git.path().join(p);
                    if !full.exists() {
                        failures.push(ValidatorFailure {
                            validator: "artifact-exists".into(),
                            report: format!("missing artifact path: {p}"),
                        });
                    }
                }
            }
            "cargo-check" => {
                // Best-effort: only if Cargo.toml exists
                if git.path().join("Cargo.toml").exists() {
                    let out = Command::new("cargo")
                        .args(["check", "--quiet"])
                        .current_dir(git.path())
                        .output()
                        .map_err(|e| StoreError::Git(e.to_string()))?;
                    if !out.status.success() {
                        failures.push(ValidatorFailure {
                            validator: "cargo-check".into(),
                            report: String::from_utf8_lossy(&out.stderr).to_string(),
                        });
                    }
                }
            }
            "cmd" | "true" => {
                // `true` always passes; `cmd` without args is no-op pass in v1
            }
            other => {
                // unregistered should be caught at publish; at runtime treat as fail
                failures.push(ValidatorFailure {
                    validator: other.into(),
                    report: format!("validator not implemented at runtime: {other}"),
                });
            }
        }
    }
    // Always run implicit scope-diff if not listed
    if !names.iter().any(|n| n == "scope-diff") && !out_scope_paths.is_empty() {
        failures.push(ValidatorFailure {
            validator: "scope-diff".into(),
            report: format!(
                "越界写入: {} 不在声明范围 {:?}",
                out_scope_paths.join(", "),
                write_scope
            ),
        });
    }
    Ok(failures)
}

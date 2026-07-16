use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use sunmao_core::new_id;

use crate::error::{StoreError, StoreResult};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub repo_path: String,
    pub created_at: DateTime<Utc>,
}

pub struct ProjectsRepo<'a> {
    pub pool: &'a PgPool,
}

impl<'a> ProjectsRepo<'a> {
    pub fn new(pool: &'a PgPool) -> Self {
        Self { pool }
    }

    /// Register project; if repo_path exists, return existing (idempotent).
    pub async fn create_or_get(&self, name: &str, repo_path: &str) -> StoreResult<Project> {
        let repo_path = std::fs::canonicalize(repo_path)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| repo_path.to_string());

        if let Some(existing) = self.lookup_by_path(&repo_path).await? {
            return Ok(existing);
        }

        let id = new_id("pj_");
        let row = sqlx::query_as::<_, Project>(
            r#"
            INSERT INTO project (id, name, repo_path)
            VALUES ($1, $2, $3)
            ON CONFLICT (repo_path) DO UPDATE SET name = EXCLUDED.name
            RETURNING id, name, repo_path, created_at
            "#,
        )
        .bind(&id)
        .bind(name)
        .bind(&repo_path)
        .fetch_one(self.pool)
        .await?;

        // Ensure graph_version 0 row exists
        sqlx::query(
            r#"
            INSERT INTO graph_version (project_id, version, planner, summary)
            VALUES ($1, 0, 'system', 'init')
            ON CONFLICT DO NOTHING
            "#,
        )
        .bind(&row.id)
        .execute(self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list(&self) -> StoreResult<Vec<Project>> {
        let rows = sqlx::query_as::<_, Project>(
            "SELECT id, name, repo_path, created_at FROM project ORDER BY created_at",
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn get(&self, id: &str) -> StoreResult<Project> {
        sqlx::query_as::<_, Project>(
            "SELECT id, name, repo_path, created_at FROM project WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(self.pool)
        .await?
        .ok_or_else(|| StoreError::NotFound(format!("project {id}")))
    }

    pub async fn lookup_by_path(&self, repo_path: &str) -> StoreResult<Option<Project>> {
        let canon = std::fs::canonicalize(repo_path)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| repo_path.to_string());
        let row = sqlx::query_as::<_, Project>(
            "SELECT id, name, repo_path, created_at FROM project WHERE repo_path = $1 OR repo_path = $2",
        )
        .bind(repo_path)
        .bind(&canon)
        .fetch_optional(self.pool)
        .await?;
        Ok(row)
    }
}

use anyhow::{bail, Context, Result};
use reqwest::Client;
use serde_json::Value;

pub struct ApiClient {
    base: String,
    http: Client,
}

impl ApiClient {
    pub fn new(base: &str) -> Self {
        Self {
            base: base.trim_end_matches('/').to_string(),
            http: Client::new(),
        }
    }

    pub async fn get(&self, path: &str) -> Result<Value> {
        let url = format!("{}{path}", self.base);
        let resp = self
            .http
            .get(&url)
            .header("X-Sunmao-Actor", "human")
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            bail!("GET {path} -> {status}: {body}");
        }
        if body.is_empty() {
            return Ok(Value::Null);
        }
        Ok(serde_json::from_str(&body)?)
    }

    pub async fn post(&self, path: &str, body: &Value) -> Result<Value> {
        let url = format!("{}{path}", self.base);
        let resp = self
            .http
            .post(&url)
            .header("X-Sunmao-Actor", "human")
            .json(body)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            bail!("POST {path} -> {status}: {text}");
        }
        if text.is_empty() {
            return Ok(Value::Null);
        }
        Ok(serde_json::from_str(&text)?)
    }
}

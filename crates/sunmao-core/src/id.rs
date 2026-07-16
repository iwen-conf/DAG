use ulid::Ulid;

pub type ProjectId = String;
pub type NodeId = String;
pub type EventId = String;
pub type AttemptId = String;
pub type ArtifactId = String;

/// Prefixed ULID: `pj_` / `nd_` / `ev_` / `at_` / `ar_`
pub fn new_id(prefix: &str) -> String {
    format!("{prefix}{}", Ulid::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_and_length() {
        let id = new_id("nd_");
        assert!(id.starts_with("nd_"));
        assert!(id.len() > 10);
    }
}

//! Write-scope prefix intersection (D-10).
//! Paths are treated as directory prefixes; trailing `/` is normalized.

/// Normalize a scope path for comparison: strip leading `./`, ensure consistent trailing slash
/// for directory-like prefixes, keep file-like paths as-is if no trailing slash and no children intent.
pub fn normalize_scope(s: &str) -> String {
    let s = s.trim().trim_start_matches("./");
    if s.is_empty() {
        return String::new();
    }
    s.to_string()
}

/// True if path is under any of the scope prefixes (exact or prefix).
pub fn path_in_scopes(path: &str, scopes: &[String]) -> bool {
    let path = normalize_scope(path);
    scopes.iter().any(|sc| path_under_scope(&path, &normalize_scope(sc)))
}

fn path_under_scope(path: &str, scope: &str) -> bool {
    if scope.is_empty() {
        return true;
    }
    if path == scope {
        return true;
    }
    let scope_as_dir = if scope.ends_with('/') {
        scope.to_string()
    } else {
        format!("{scope}/")
    };
    path.starts_with(&scope_as_dir) || path == scope.trim_end_matches('/')
}

/// Prefix intersection: true if two scopes overlap (either equal, or one is prefix of the other).
pub fn scopes_prefix_intersect(a: &str, b: &str) -> bool {
    let a = normalize_scope(a);
    let b = normalize_scope(b);
    if a.is_empty() || b.is_empty() {
        return true;
    }
    path_under_scope(&a, &b) || path_under_scope(&b, &a)
}

/// Any pairwise prefix conflict between two scope sets.
pub fn scopes_conflict(a: &[String], b: &[String]) -> bool {
    for x in a {
        for y in b {
            if scopes_prefix_intersect(x, y) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_containment() {
        assert!(scopes_prefix_intersect("src/api", "src/api/user"));
        assert!(scopes_prefix_intersect("src/api/", "src/api/user.rs"));
        assert!(!scopes_prefix_intersect("src/api", "src/web"));
        assert!(scopes_prefix_intersect("src/api", "src/api"));
    }

    #[test]
    fn path_in_scopes_ok() {
        let scopes = vec!["server/identity/".into()];
        assert!(path_in_scopes("server/identity/login.rs", &scopes));
        assert!(!path_in_scopes("docs/readme.md", &scopes));
    }

    #[test]
    fn sets_conflict() {
        let a = vec!["src/api/".into()];
        let b = vec!["src/api/user/".into()];
        let c = vec!["src/web/".into()];
        assert!(scopes_conflict(&a, &b));
        assert!(!scopes_conflict(&a, &c));
    }
}

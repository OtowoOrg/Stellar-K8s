//! Intelligent request routing based on path, method, headers, and version.

use crate::api_gateway::config::{RouteConfig, VersioningConfig};
use std::collections::HashMap;

/// The result of matching a request against the route table.
#[derive(Debug, Clone)]
pub struct RouteMatch<'a> {
    pub route: &'a RouteConfig,
    /// Remaining path after stripping the matched prefix
    pub remaining_path: String,
}

/// Route table built from [`RouteConfig`] entries.
pub struct Router {
    routes: Vec<RouteConfig>,
    versioning: VersioningConfig,
}

impl Router {
    pub fn new(routes: Vec<RouteConfig>, versioning: VersioningConfig) -> Self {
        Self { routes, versioning }
    }

    /// Match an incoming request to a route.
    ///
    /// Matching priority:
    /// 1. Longest path prefix wins
    /// 2. Method must be in the allowed list (or list is empty = all)
    /// 3. All header predicates must match
    /// 4. Sunset versions return `None` (caller should respond 410)
    pub fn match_route<'a>(
        &'a self,
        path: &str,
        method: &str,
        headers: &HashMap<String, String>,
    ) -> MatchResult<'a> {
        // Check if the path starts with a sunset version prefix
        for sv in &self.versioning.sunset_versions {
            if path.contains(&format!("/{sv}/")) || path.ends_with(&format!("/{sv}")) {
                return MatchResult::Sunset(sv.clone());
            }
        }

        let mut best: Option<&RouteConfig> = None;
        let mut best_len = 0usize;

        for route in &self.routes {
            if !path.starts_with(&route.path_prefix) {
                continue;
            }
            if !route.methods.is_empty()
                && !route.methods.iter().any(|m| m.eq_ignore_ascii_case(method))
            {
                continue;
            }
            // Header predicates
            if !route
                .header_predicates
                .iter()
                .all(|(k, v)| headers.get(k.as_str()).map(|hv| hv == v).unwrap_or(false))
            {
                continue;
            }
            if route.path_prefix.len() > best_len {
                best_len = route.path_prefix.len();
                best = Some(route);
            }
        }

        match best {
            None => MatchResult::NotFound,
            Some(route) => {
                let remaining = path[route.path_prefix.len()..].to_string();
                MatchResult::Matched(RouteMatch {
                    route,
                    remaining_path: remaining,
                })
            }
        }
    }
}

pub enum MatchResult<'a> {
    Matched(RouteMatch<'a>),
    NotFound,
    /// Version has been sunset — respond 410 Gone
    Sunset(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_gateway::config::{Protocol, VersioningConfig};

    fn make_route(prefix: &str, version: &str) -> RouteConfig {
        RouteConfig {
            id: prefix.into(),
            path_prefix: prefix.into(),
            methods: vec![],
            upstream: "http://upstream".into(),
            protocol: Protocol::Rest,
            version: version.into(),
            header_predicates: HashMap::new(),
            deprecated: false,
            sunset_date: None,
        }
    }

    #[test]
    fn longest_prefix_wins() {
        let router = Router::new(
            vec![
                make_route("/api/v1", "v1"),
                make_route("/api/v1/transactions", "v1"),
            ],
            VersioningConfig::default(),
        );
        let m = router.match_route("/api/v1/transactions/abc", "GET", &HashMap::new());
        match m {
            MatchResult::Matched(r) => assert_eq!(r.route.path_prefix, "/api/v1/transactions"),
            _ => panic!("expected match"),
        }
    }

    #[test]
    fn no_match_returns_not_found() {
        let router = Router::new(vec![], VersioningConfig::default());
        assert!(matches!(
            router.match_route("/unknown", "GET", &HashMap::new()),
            MatchResult::NotFound
        ));
    }
}

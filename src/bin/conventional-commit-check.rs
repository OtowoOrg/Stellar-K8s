#!/usr/bin/env rust
//! Conventional Commit validation tool
//!
//! This utility validates that commit messages follow the Conventional Commits spec
//! (https://www.conventionalcommits.org/):
//!
//! Format: <type>[optional scope]: <description>
//! Types: feat, fix, docs, style, refactor, test, chore, perf, ci, build, revert
//!
//! Usage:
//!   conventional-commit-check "fix(auth): prevent race condition in login"
//!   conventional-commit-check --file .git/COMMIT_EDITMSG

use regex::Regex;
use std::path::Path;
use std::process;

const ALLOWED_TYPES: &[&str] = &[
    "feat", "fix", "docs", "style", "refactor", "test", "chore", "perf", "ci", "build", "revert",
];

/// Validate conventional commit format
fn validate_commit(message: &str) -> Result<(String, Option<String>, String), String> {
    let trimmed = message.trim();

    // Pattern: type(optional scope): description
    let pattern =
        r"^(feat|fix|docs|style|refactor|test|chore|perf|ci|build|revert)(\([^)]+\))?: .+";
    let regex = Regex::new(pattern).map_err(|e| format!("Regex error: {}", e))?;

    if !regex.is_match(trimmed) {
        return Err(format!(
            "Invalid commit message format.\n\
             Expected: <type>[optional scope]: <description>\n\
             Example: fix(auth): prevent race condition in login\n\
             Got: {}",
            trimmed
        ));
    }

    // Parse components
    let mut parts = trimmed.split(':');
    let type_and_scope = parts.next().ok_or("Missing type and scope")?;
    let description = parts.collect::<Vec<_>>().join(":").trim().to_string();

    let (commit_type, scope) = if let Some(paren_pos) = type_and_scope.find('(') {
        let t = type_and_scope[..paren_pos].trim().to_string();
        let s = type_and_scope[paren_pos + 1..]
            .trim_end_matches(')')
            .to_string();
        (t, Some(s))
    } else {
        (type_and_scope.trim().to_string(), None)
    };

    // Validate type
    if !ALLOWED_TYPES.contains(&commit_type.as_str()) {
        return Err(format!(
            "Invalid commit type '{}'. Allowed types: {}",
            commit_type,
            ALLOWED_TYPES.join(", ")
        ));
    }

    // Validate description is not empty
    if description.is_empty() {
        return Err("Commit description cannot be empty".to_string());
    }

    Ok((commit_type, scope, description))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: {} <commit-message> | --file <path>", args[0]);
        eprintln!("Examples:");
        eprintln!("  {} 'fix(auth): prevent race condition'", args[0]);
        eprintln!("  {} --file .git/COMMIT_EDITMSG", args[0]);
        process::exit(1);
    }

    let message = if args.len() == 3 && args[1] == "--file" {
        std::fs::read_to_string(&args[2]).unwrap_or_else(|e| {
            eprintln!("Failed to read file {}: {}", args[2], e);
            process::exit(1);
        })
    } else {
        args[1..].join(" ")
    };

    match validate_commit(&message) {
        Ok((commit_type, scope, description)) => {
            println!("✓ Valid conventional commit");
            println!("  Type: {}", commit_type);
            if let Some(s) = scope {
                println!("  Scope: {}", s);
            }
            println!("  Description: {}", description);
            process::exit(0);
        }
        Err(e) => {
            eprintln!("✗ Commit validation failed:");
            eprintln!("{}", e);
            process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_feat() {
        let result = validate_commit("feat(api): add webhook support");
        assert!(result.is_ok());
        let (t, s, d) = result.unwrap();
        assert_eq!(t, "feat");
        assert_eq!(s, Some("api".to_string()));
        assert_eq!(d, "add webhook support");
    }

    #[test]
    fn test_valid_fix() {
        let result = validate_commit("fix: prevent race condition");
        assert!(result.is_ok());
        let (t, s, d) = result.unwrap();
        assert_eq!(t, "fix");
        assert_eq!(s, None);
        assert_eq!(d, "prevent race condition");
    }

    #[test]
    fn test_invalid_type() {
        let result = validate_commit("badtype: something");
        assert!(result.is_err());
    }

    #[test]
    fn test_missing_description() {
        let result = validate_commit("feat(api):");
        assert!(result.is_err());
    }

    #[test]
    fn test_multiline() {
        let result =
            validate_commit("feat(tenant): add namespace isolation\n\nDetailed description");
        assert!(result.is_ok());
    }
}

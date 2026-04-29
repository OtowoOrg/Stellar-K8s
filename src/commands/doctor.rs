use crate::cli::DoctorArgs;
use kube::Client;
use std::process::Command;
use stellar_k8s::{preflight, Error};

/// Output-friendly status used by the doctor command.
struct CheckStatus {
    name: &'static str,
    passed: bool,
    message: String,
}

impl CheckStatus {
    fn new(name: &'static str, passed: bool, message: String) -> Self {
        Self {
            name,
            passed,
            message,
        }
    }

    fn format(&self) -> String {
        let label = if self.passed { "Green" } else { "Red" };
        format!("[{label}] {name}: {message}", name = self.name, message = self.message)
    }
}

fn run_command_check(
    name: &'static str,
    command: &str,
    args: &[&str],
    output_prefix: &'static str,
) -> CheckStatus {
    match Command::new(command).args(args).output() {
        Ok(output) => {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let summary = stdout.lines().next().unwrap_or_default().trim();
                let message = if summary.is_empty() {
                    format!("{output_prefix} found")
                } else {
                    format!("{output_prefix}: {summary}")
                };
                CheckStatus::new(name, true, message)
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let message = if stderr.is_empty() {
                    format!("{output_prefix} exists but returned an error")
                } else {
                    format!("{output_prefix} failed: {stderr}")
                };
                CheckStatus::new(name, false, message)
            }
        }
        Err(err) => {
            let message = if err.kind() == std::io::ErrorKind::NotFound {
                format!("{output_prefix} was not found in PATH")
            } else {
                format!("failed to execute {command}: {err}")
            };
            CheckStatus::new(name, false, message)
        }
    }
}

fn check_github_cli() -> CheckStatus {
    run_command_check("GitHub CLI", "gh", &["--version"], "gh CLI")
}

fn check_kubectl_cli() -> CheckStatus {
    run_command_check(
        "kubectl CLI",
        "kubectl",
        &["version", "--client", "--short"],
        "kubectl",
    )
}

fn check_helm_cli() -> CheckStatus {
    run_command_check("Helm CLI", "helm", &["version", "--short"], "helm")
}

fn check_kubectl_current_context() -> CheckStatus {
    match Command::new("kubectl")
        .args(["config", "current-context"])
        .output()
    {
        Ok(output) => {
            if output.status.success() {
                let context = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if context.is_empty() {
                    CheckStatus::new(
                        "Kubernetes Context",
                        false,
                        "no current kubectl context is set".to_string(),
                    )
                } else {
                    CheckStatus::new(
                        "Kubernetes Context",
                        true,
                        format!("current context is '{context}'"),
                    )
                }
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                CheckStatus::new(
                    "Kubernetes Context",
                    false,
                    if stderr.is_empty() {
                        "failed to determine current context".to_string()
                    } else {
                        format!("failed to determine current context: {stderr}")
                    },
                )
            }
        }
        Err(err) => {
            let message = if err.kind() == std::io::ErrorKind::NotFound {
                "kubectl was not found in PATH, so the current context cannot be resolved"
                    .to_string()
            } else {
                format!("failed to execute kubectl: {err}")
            };
            CheckStatus::new("Kubernetes Context", false, message)
        }
    }
}

async fn run_kubernetes_checks(namespace: &str) -> Vec<CheckStatus> {
    let client = match Client::try_default().await {
        Ok(client) => client,
        Err(err) => {
            return vec![CheckStatus::new(
                "Kubernetes Permissions",
                false,
                format!("failed to create Kubernetes client from current kubeconfig: {err}"),
            )];
        }
    };

    let preflight_results: Vec<stellar_k8s::preflight::CheckResult> =
        preflight::run_preflight_checks(&client, namespace).await;
    preflight_results
        .into_iter()
        .map(|result| CheckStatus::new(result.name, result.passed, result.message))
        .collect()
}

fn print_summary(checks: &[CheckStatus]) {
    let passed = checks.iter().filter(|c| c.passed).count();
    let total = checks.len();

    for check in checks {
        println!("{}", check.format());
    }
    println!();
    println!("SUMMARY: {passed}/{total} checks passed");
}

pub async fn run_doctor(args: DoctorArgs) -> Result<(), Error> {
    println!("=== Stellar Doctor: Local environment verification ===");
    println!();

    let mut checks = Vec::new();

    checks.push(check_github_cli());
    checks.push(check_kubectl_cli());
    checks.push(check_helm_cli());
    checks.push(check_kubectl_current_context());

    println!("=== CLI tool and context checks ===");
    println!();
    for check in &checks {
        println!("{}", check.format());
    }
    println!();

    println!("=== Kubernetes permissions checks ===");
    println!();
    let kube_checks = run_kubernetes_checks(&args.namespace).await;
    for check in &kube_checks {
        println!("{}", check.format());
    }

    checks.extend(kube_checks);
    println!();
    print_summary(&checks);

    if checks.iter().all(|c| c.passed) {
        Ok(())
    } else {
        Err(Error::ConfigError(
            "Environment verification failed. Fix the failing checks and retry.".to_string(),
        ))
    }
}

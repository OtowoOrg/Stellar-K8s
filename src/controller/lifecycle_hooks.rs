// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//! Lifecycle hooks framework for workload startup and shutdown (epic #1525).
//!
//! Hooks are declared with a [`HookPhase`], an `order` and a
//! [`FailurePolicy`]. Orchestration lives in the shared `stellar-hooks`
//! runner (`src/bin/stellar-hooks.rs`), so workloads only provide the hook
//! commands (callbacks). [`apply_to_pod`] wires the runner into a pod:
//!
//! | Phase       | Kubernetes mechanism        | Default policy |
//! |-------------|-----------------------------|----------------|
//! | `Setup`     | init container (app image)  | `Block`        |
//! | `Readiness` | readiness probe (exec)      | `Block`        |
//! | `Teardown`  | `preStop` handler (exec)    | `Warn`         |
//!
//! Within a phase hooks run sequentially sorted by `(order, name)`, which is
//! deterministic. A failing `Block` hook stops the phase and makes the runner
//! exit non-zero: the init container fails (pod never starts) or the probe
//! fails (pod not Ready). `Warn` failures are recorded and the phase
//! continues. Setup and readiness hooks can re-run (restarts, periodic
//! probes), so they must be declared idempotent. Teardown hooks share the
//! pod's termination grace period: each hook's timeout is clipped to the
//! remaining budget and hooks that no longer fit are skipped.
//!
//! Every hook execution is timed; [`render_metrics`] emits Prometheus text
//! the runner writes to the shared hooks volume for scraping.

use k8s_openapi::api::core::v1::{
    Container, EmptyDirVolumeSource, EnvVar, ExecAction, Lifecycle, LifecycleHandler, PodSpec,
    Probe, Volume, VolumeMount,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::Duration;

/// Env var carrying the JSON-encoded hook list to the runner.
pub const HOOKS_ENV: &str = "STELLAR_HOOKS";
/// Env var carrying the teardown budget (seconds) to the runner.
pub const GRACE_ENV: &str = "STELLAR_HOOKS_GRACE_SECONDS";
/// Mount path of the shared volume holding the runner binary and metrics.
pub const HOOKS_DIR: &str = "/stellar-hooks";
const HOOKS_VOLUME: &str = "stellar-hooks";
const RUNNER_NAME: &str = "stellar-hooks";

/// Lifecycle phase a hook belongs to, in execution order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HookPhase {
    Setup,
    Readiness,
    Teardown,
}

impl HookPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            HookPhase::Setup => "setup",
            HookPhase::Readiness => "readiness",
            HookPhase::Teardown => "teardown",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "setup" => Some(HookPhase::Setup),
            "readiness" => Some(HookPhase::Readiness),
            "teardown" => Some(HookPhase::Teardown),
            _ => None,
        }
    }

    /// Failure policy used when a hook does not declare one.
    pub fn default_policy(self) -> FailurePolicy {
        match self {
            HookPhase::Setup | HookPhase::Readiness => FailurePolicy::Block,
            HookPhase::Teardown => FailurePolicy::Warn,
        }
    }
}

/// What a hook failure does to its phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FailurePolicy {
    /// Stop the phase; setup fails the pod, readiness keeps it NotReady.
    Block,
    /// Record the failure and continue.
    Warn,
}

/// A declared lifecycle hook.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleHook {
    pub name: String,
    pub phase: HookPhase,
    /// Lower runs first; ties are broken by name.
    #[serde(default)]
    pub order: i32,
    pub command: Vec<String>,
    pub timeout_seconds: u64,
    #[serde(default)]
    pub failure_policy: Option<FailurePolicy>,
    /// The hook is safe to run more than once.
    #[serde(default)]
    pub idempotent: bool,
}

impl LifecycleHook {
    pub fn policy(&self) -> FailurePolicy {
        self.failure_policy
            .unwrap_or_else(|| self.phase.default_policy())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HookValidationError {
    #[error("duplicate hook name '{0}'")]
    DuplicateName(String),
    #[error("hook '{0}' has an empty command")]
    EmptyCommand(String),
    #[error("hook '{0}' must have a timeout greater than zero")]
    ZeroTimeout(String),
    #[error("hook '{0}' runs in a re-entrant phase and must be idempotent")]
    NotIdempotent(String),
    #[error("teardown hooks need {needed}s but the grace period is {grace}s")]
    TeardownExceedsGrace { needed: u64, grace: u64 },
}

/// Validate hooks against the pod's termination grace period.
pub fn validate(
    hooks: &[LifecycleHook],
    grace_period_seconds: u64,
) -> Result<(), HookValidationError> {
    let mut names = HashSet::new();
    for h in hooks {
        if !names.insert(h.name.as_str()) {
            return Err(HookValidationError::DuplicateName(h.name.clone()));
        }
        if h.command.is_empty() {
            return Err(HookValidationError::EmptyCommand(h.name.clone()));
        }
        if h.timeout_seconds == 0 {
            return Err(HookValidationError::ZeroTimeout(h.name.clone()));
        }
        if h.phase != HookPhase::Teardown && !h.idempotent {
            return Err(HookValidationError::NotIdempotent(h.name.clone()));
        }
    }
    let needed: u64 = hooks
        .iter()
        .filter(|h| h.phase == HookPhase::Teardown)
        .map(|h| h.timeout_seconds)
        .sum();
    if needed > grace_period_seconds {
        return Err(HookValidationError::TeardownExceedsGrace {
            needed,
            grace: grace_period_seconds,
        });
    }
    Ok(())
}

/// Hooks of `phase` in execution order.
pub fn ordered(hooks: &[LifecycleHook], phase: HookPhase) -> Vec<&LifecycleHook> {
    let mut out: Vec<_> = hooks.iter().filter(|h| h.phase == phase).collect();
    out.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.name.cmp(&b.name)));
    out
}

/// Outcome of a single hook execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum HookOutcome {
    Succeeded,
    Failed,
    TimedOut,
    /// Not run: an earlier blocking hook failed or the grace budget ran out.
    Skipped,
}

impl HookOutcome {
    fn as_str(self) -> &'static str {
        match self {
            HookOutcome::Succeeded => "succeeded",
            HookOutcome::Failed => "failed",
            HookOutcome::TimedOut => "timed_out",
            HookOutcome::Skipped => "skipped",
        }
    }
}

/// What an executor reports for one hook run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecReport {
    pub outcome: HookOutcome,
    pub elapsed: Duration,
}

/// Runs a hook command with a hard timeout.
pub trait HookExecutor {
    fn exec(&mut self, hook: &LifecycleHook, timeout: Duration) -> ExecReport;
}

/// Timing and outcome of one hook in a phase run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookResult {
    pub name: String,
    pub phase: HookPhase,
    pub outcome: HookOutcome,
    pub policy: FailurePolicy,
    pub duration_ms: u128,
}

/// Result of running one phase.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhaseResult {
    pub phase: HookPhase,
    /// A `Block` hook failed or timed out.
    pub blocked: bool,
    pub results: Vec<HookResult>,
}

/// Run every hook of `phase` in order. `budget` bounds the whole phase
/// (the termination grace period for teardown).
pub fn run_phase<E: HookExecutor>(
    hooks: &[LifecycleHook],
    phase: HookPhase,
    budget: Option<Duration>,
    exec: &mut E,
) -> PhaseResult {
    let mut results = Vec::new();
    let mut blocked = false;
    let mut spent = Duration::ZERO;

    for hook in ordered(hooks, phase) {
        let mut timeout = Duration::from_secs(hook.timeout_seconds);
        if let Some(b) = budget {
            timeout = timeout.min(b.saturating_sub(spent));
        }
        let report = if blocked || timeout.is_zero() {
            ExecReport {
                outcome: HookOutcome::Skipped,
                elapsed: Duration::ZERO,
            }
        } else {
            exec.exec(hook, timeout)
        };
        spent += report.elapsed;
        if matches!(report.outcome, HookOutcome::Failed | HookOutcome::TimedOut)
            && hook.policy() == FailurePolicy::Block
        {
            blocked = true;
        }
        results.push(HookResult {
            name: hook.name.clone(),
            phase,
            outcome: report.outcome,
            policy: hook.policy(),
            duration_ms: report.elapsed.as_millis(),
        });
    }

    PhaseResult {
        phase,
        blocked,
        results,
    }
}

/// Prometheus text exposition for a phase run.
pub fn render_metrics(result: &PhaseResult) -> String {
    let mut out = String::from(
        "# HELP stellar_lifecycle_hook_duration_seconds Duration of the last lifecycle hook run.\n\
         # TYPE stellar_lifecycle_hook_duration_seconds gauge\n",
    );
    for r in &result.results {
        out.push_str(&format!(
            "stellar_lifecycle_hook_duration_seconds{{hook=\"{}\",phase=\"{}\",outcome=\"{}\"}} {:.3}\n",
            r.name.replace('\\', "\\\\").replace('"', "\\\""),
            r.phase.as_str(),
            r.outcome.as_str(),
            r.duration_ms as f64 / 1000.0
        ));
    }
    out.push_str(&format!(
        "# HELP stellar_lifecycle_phase_blocked Whether a blocking hook failed in the phase.\n\
         # TYPE stellar_lifecycle_phase_blocked gauge\n\
         stellar_lifecycle_phase_blocked{{phase=\"{}\"}} {}\n",
        result.phase.as_str(),
        u8::from(result.blocked)
    ));
    out
}

fn runner_exec(phase: HookPhase) -> ExecAction {
    ExecAction {
        command: Some(vec![
            format!("{HOOKS_DIR}/{RUNNER_NAME}"),
            "run".into(),
            phase.as_str().into(),
        ]),
    }
}

/// Wire the hooks runner into `spec` for the container named `container`.
///
/// Adds a shared `emptyDir`, an init container (from `runner_image`) that
/// installs the runner into it, a setup init container using the app image,
/// an exec readiness probe and a `preStop` handler. The main container must
/// not already define a readiness probe or `preStop` when hooks for those
/// phases are declared.
pub fn apply_to_pod(
    spec: &mut PodSpec,
    container: &str,
    runner_image: &str,
    hooks: &[LifecycleHook],
) -> Result<(), String> {
    if hooks.is_empty() {
        return Ok(());
    }
    let grace = spec.termination_grace_period_seconds.unwrap_or(30).max(0) as u64;
    validate(hooks, grace).map_err(|e| e.to_string())?;
    let hooks_json = serde_json::to_string(hooks).map_err(|e| e.to_string())?;

    let mount = VolumeMount {
        name: HOOKS_VOLUME.into(),
        mount_path: HOOKS_DIR.into(),
        ..Default::default()
    };
    let env = vec![
        EnvVar {
            name: HOOKS_ENV.into(),
            value: Some(hooks_json),
            ..Default::default()
        },
        EnvVar {
            name: GRACE_ENV.into(),
            value: Some(grace.to_string()),
            ..Default::default()
        },
    ];

    let main = spec
        .containers
        .iter_mut()
        .find(|c| c.name == container)
        .ok_or_else(|| format!("container '{container}' not found"))?;
    let app_image = main.image.clone();

    if !ordered(hooks, HookPhase::Readiness).is_empty() {
        if main.readiness_probe.is_some() {
            return Err(format!(
                "container '{container}' already has a readinessProbe"
            ));
        }
        let total: u64 = ordered(hooks, HookPhase::Readiness)
            .iter()
            .map(|h| h.timeout_seconds)
            .sum();
        main.readiness_probe = Some(Probe {
            exec: Some(runner_exec(HookPhase::Readiness)),
            timeout_seconds: Some(total as i32 + 1),
            ..Default::default()
        });
    }
    if !ordered(hooks, HookPhase::Teardown).is_empty() {
        let lifecycle = main.lifecycle.get_or_insert_with(Lifecycle::default);
        if lifecycle.pre_stop.is_some() {
            return Err(format!(
                "container '{container}' already has a preStop hook"
            ));
        }
        lifecycle.pre_stop = Some(LifecycleHandler {
            exec: Some(runner_exec(HookPhase::Teardown)),
            ..Default::default()
        });
    }
    main.volume_mounts
        .get_or_insert_with(Vec::new)
        .push(mount.clone());
    main.env.get_or_insert_with(Vec::new).extend(env.clone());

    let init = spec.init_containers.get_or_insert_with(Vec::new);
    init.push(Container {
        name: "stellar-hooks-install".into(),
        image: Some(runner_image.into()),
        command: Some(vec![
            format!("/{RUNNER_NAME}"),
            "install".into(),
            format!("{HOOKS_DIR}/{RUNNER_NAME}"),
        ]),
        volume_mounts: Some(vec![mount.clone()]),
        ..Default::default()
    });
    if !ordered(hooks, HookPhase::Setup).is_empty() {
        init.push(Container {
            name: "stellar-hooks-setup".into(),
            image: app_image,
            command: runner_exec(HookPhase::Setup).command,
            env: Some(env),
            volume_mounts: Some(vec![mount]),
            ..Default::default()
        });
    }
    spec.volumes.get_or_insert_with(Vec::new).push(Volume {
        name: HOOKS_VOLUME.into(),
        empty_dir: Some(EmptyDirVolumeSource::default()),
        ..Default::default()
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn hook(name: &str, phase: HookPhase, order: i32) -> LifecycleHook {
        LifecycleHook {
            name: name.into(),
            phase,
            order,
            command: vec!["true".into()],
            timeout_seconds: 5,
            failure_policy: None,
            idempotent: true,
        }
    }

    /// Returns a scripted outcome per hook name; default success in 10ms.
    #[derive(Default)]
    struct Scripted {
        outcomes: HashMap<String, (HookOutcome, Duration)>,
        ran: Vec<(String, Duration)>,
    }

    impl HookExecutor for Scripted {
        fn exec(&mut self, hook: &LifecycleHook, timeout: Duration) -> ExecReport {
            self.ran.push((hook.name.clone(), timeout));
            let (outcome, elapsed) = self
                .outcomes
                .get(&hook.name)
                .copied()
                .unwrap_or((HookOutcome::Succeeded, Duration::from_millis(10)));
            ExecReport { outcome, elapsed }
        }
    }

    fn all_phases() -> Vec<LifecycleHook> {
        vec![
            hook("migrate", HookPhase::Setup, 1),
            hook("warm-cache", HookPhase::Setup, 2),
            hook("config", HookPhase::Setup, 1),
            hook("peers", HookPhase::Readiness, 0),
            hook("sync", HookPhase::Readiness, 1),
            hook("drain", HookPhase::Teardown, 0),
            hook("flush", HookPhase::Teardown, 1),
        ]
    }

    #[test]
    fn order_is_deterministic() {
        let hooks = all_phases();
        let names: Vec<_> = ordered(&hooks, HookPhase::Setup)
            .iter()
            .map(|h| h.name.as_str())
            .collect();
        assert_eq!(names, vec!["config", "migrate", "warm-cache"]);
    }

    #[test]
    fn validation_enforces_idempotency_timeouts_and_grace() {
        let mut h = all_phases();
        h[0].idempotent = false;
        assert_eq!(
            validate(&h, 30),
            Err(HookValidationError::NotIdempotent("migrate".into()))
        );
        let mut h = all_phases();
        h[1].timeout_seconds = 0;
        assert!(matches!(
            validate(&h, 30),
            Err(HookValidationError::ZeroTimeout(_))
        ));
        assert_eq!(
            validate(&all_phases(), 9),
            Err(HookValidationError::TeardownExceedsGrace {
                needed: 10,
                grace: 9
            })
        );
        assert!(validate(&all_phases(), 30).is_ok());
    }

    /// Failure matrix: each phase fails in isolation under each policy.
    #[test]
    fn failure_matrix() {
        for phase in [HookPhase::Setup, HookPhase::Readiness, HookPhase::Teardown] {
            for policy in [FailurePolicy::Block, FailurePolicy::Warn] {
                for outcome in [HookOutcome::Failed, HookOutcome::TimedOut] {
                    let mut hooks = all_phases();
                    let first = ordered(&hooks, phase)[0].name.clone();
                    hooks
                        .iter_mut()
                        .find(|h| h.name == first)
                        .unwrap()
                        .failure_policy = Some(policy);
                    let mut exec = Scripted::default();
                    exec.outcomes
                        .insert(first.clone(), (outcome, Duration::from_millis(250)));

                    for p in [HookPhase::Setup, HookPhase::Readiness, HookPhase::Teardown] {
                        let r = run_phase(&hooks, p, None, &mut exec);
                        let expect_block = p == phase && policy == FailurePolicy::Block;
                        assert_eq!(r.blocked, expect_block, "{phase:?}/{policy:?}/{p:?}");
                        let rest = &r.results[1..];
                        if expect_block {
                            assert!(rest.iter().all(|x| x.outcome == HookOutcome::Skipped));
                        } else {
                            assert!(rest.iter().all(|x| x.outcome == HookOutcome::Succeeded));
                        }
                        if p == phase {
                            assert_eq!(r.results[0].outcome, outcome);
                            assert_eq!(r.results[0].duration_ms, 250);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn teardown_stays_within_grace_period() {
        let mut hooks = all_phases();
        hooks.push(hook("zz-last", HookPhase::Teardown, 9));
        let mut exec = Scripted::default();
        exec.outcomes.insert(
            "drain".into(),
            (HookOutcome::Succeeded, Duration::from_secs(5)),
        );
        exec.outcomes.insert(
            "flush".into(),
            (HookOutcome::TimedOut, Duration::from_secs(3)),
        );
        let r = run_phase(
            &hooks,
            HookPhase::Teardown,
            Some(Duration::from_secs(8)),
            &mut exec,
        );
        assert!(!r.blocked, "teardown defaults to warn");
        assert_eq!(exec.ran[1], ("flush".into(), Duration::from_secs(3)));
        assert_eq!(r.results[2].outcome, HookOutcome::Skipped);
        let total: u128 = r.results.iter().map(|x| x.duration_ms).sum();
        assert!(total <= 8_000);
    }

    #[test]
    fn metrics_are_exported_per_hook() {
        let mut exec = Scripted::default();
        let r = run_phase(&all_phases(), HookPhase::Readiness, None, &mut exec);
        let text = render_metrics(&r);
        assert!(text.contains(
            "stellar_lifecycle_hook_duration_seconds{hook=\"peers\",phase=\"readiness\",outcome=\"succeeded\"} 0.010"
        ));
        assert!(text.contains("stellar_lifecycle_phase_blocked{phase=\"readiness\"} 0"));
    }

    #[test]
    fn apply_to_pod_wires_runner() {
        let mut spec = PodSpec {
            containers: vec![Container {
                name: "core".into(),
                image: Some("stellar/core:21".into()),
                ..Default::default()
            }],
            ..Default::default()
        };
        apply_to_pod(&mut spec, "core", "stellar/operator:1", &all_phases()).unwrap();
        let init = spec.init_containers.as_ref().unwrap();
        assert_eq!(init[0].name, "stellar-hooks-install");
        assert_eq!(init[1].image.as_deref(), Some("stellar/core:21"));
        let main = &spec.containers[0];
        assert!(main.readiness_probe.as_ref().unwrap().exec.is_some());
        assert!(main.lifecycle.as_ref().unwrap().pre_stop.is_some());
        assert!(main
            .env
            .as_ref()
            .unwrap()
            .iter()
            .any(|e| e.name == HOOKS_ENV));
        assert_eq!(spec.volumes.as_ref().unwrap()[0].name, HOOKS_VOLUME);

        let mut again = spec.clone();
        assert!(apply_to_pod(&mut again, "core", "x", &all_phases()).is_err());
    }
}

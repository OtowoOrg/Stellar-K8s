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
//! Shared lifecycle hooks runner (epic #1525).
//!
//! ```text
//! stellar-hooks install <dest>   # copy this binary into the shared volume
//! stellar-hooks run <phase>      # run hooks from $STELLAR_HOOKS for a phase
//! ```
//!
//! `run` exits non-zero when a blocking hook fails, writes Prometheus metrics
//! to `/stellar-hooks/metrics/<phase>.prom` and prints the phase result as
//! JSON. See `stellar_k8s::controller::lifecycle_hooks`.

use std::process::{Command, ExitCode};
use std::time::{Duration, Instant};
use stellar_k8s::controller::lifecycle_hooks::{
    render_metrics, run_phase, ExecReport, HookExecutor, HookOutcome, HookPhase, LifecycleHook,
    GRACE_ENV, HOOKS_DIR, HOOKS_ENV,
};

struct ProcessExecutor;

impl HookExecutor for ProcessExecutor {
    fn exec(&mut self, hook: &LifecycleHook, timeout: Duration) -> ExecReport {
        let start = Instant::now();
        let outcome = match Command::new(&hook.command[0])
            .args(&hook.command[1..])
            .spawn()
        {
            Err(e) => {
                eprintln!("hook {}: spawn failed: {e}", hook.name);
                HookOutcome::Failed
            }
            Ok(mut child) => loop {
                match child.try_wait() {
                    Ok(Some(status)) if status.success() => break HookOutcome::Succeeded,
                    Ok(Some(_)) | Err(_) => break HookOutcome::Failed,
                    Ok(None) if start.elapsed() >= timeout => {
                        let _ = child.kill();
                        let _ = child.wait();
                        break HookOutcome::TimedOut;
                    }
                    Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                }
            },
        };
        ExecReport {
            outcome,
            elapsed: start.elapsed(),
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("install") if args.len() == 3 => install(&args[2]),
        Some("run") if args.len() == 3 => match HookPhase::parse(&args[2]) {
            Some(phase) => run(phase),
            None => usage(),
        },
        _ => usage(),
    }
}

fn usage() -> ExitCode {
    eprintln!("usage: stellar-hooks install <dest> | run <setup|readiness|teardown>");
    ExitCode::from(2)
}

fn install(dest: &str) -> ExitCode {
    let result = std::env::current_exe().and_then(|src| std::fs::copy(src, dest));
    match result {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("install to {dest} failed: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(phase: HookPhase) -> ExitCode {
    let hooks: Vec<LifecycleHook> = match std::env::var(HOOKS_ENV)
        .map_err(|e| e.to_string())
        .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
    {
        Ok(h) => h,
        Err(e) => {
            eprintln!("invalid {HOOKS_ENV}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let budget = (phase == HookPhase::Teardown)
        .then(|| std::env::var(GRACE_ENV).ok()?.parse().ok())
        .flatten()
        .map(Duration::from_secs);

    let result = run_phase(&hooks, phase, budget, &mut ProcessExecutor);

    let metrics_dir = format!("{HOOKS_DIR}/metrics");
    if std::fs::create_dir_all(&metrics_dir).is_ok() {
        let _ = std::fs::write(
            format!("{metrics_dir}/{}.prom", phase.as_str()),
            render_metrics(&result),
        );
    }
    if let Ok(json) = serde_json::to_string(&result) {
        println!("{json}");
    }
    if result.blocked {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

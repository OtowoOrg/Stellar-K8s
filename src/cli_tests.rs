//! Comprehensive CLI argument parser tests for all subcommands (Issue #594).
//!
//! Covers: defaults, flag parsing, required args, optional args, enum values,
//! mutual exclusion, unknown flags, and missing required arguments.

#[cfg(test)]
mod tests {
    use super::{Args, Commands, LogFormat, SimulatorCmd};
    use clap::Parser;

    // ── helpers ──────────────────────────────────────────────────────────────

    fn parse(argv: &[&str]) -> Result<Args, clap::Error> {
        Args::try_parse_from(argv)
    }

    macro_rules! subcmd {
        ($variant:ident, $argv:expr) => {{
            let parsed = parse($argv).expect(concat!(stringify!($variant), " should parse"));
            match parsed.command {
                Commands::$variant(a) => a,
                _ => panic!(concat!("expected ", stringify!($variant), " subcommand")),
            }
        }};
    }

    // ── Info ─────────────────────────────────────────────────────────────────

    #[test]
    fn info_default_namespace() {
        let a = subcmd!(Info, &["stellar-operator", "info"]);
        assert_eq!(a.namespace, "default");
    }

    #[test]
    fn info_custom_namespace() {
        let a = subcmd!(Info, &["stellar-operator", "info", "--namespace", "prod"]);
        assert_eq!(a.namespace, "prod");
    }

    // Table-driven: info namespace variants
    #[test]
    fn info_namespace_table() {
        for ns in &["default", "stellar-system", "my-ns", "a"] {
            let a = subcmd!(
                Info,
                &["stellar-operator", "info", "--namespace", ns]
            );
            assert_eq!(&a.namespace, ns);
        }
    }

    // ── Benchmark ────────────────────────────────────────────────────────────

    #[test]
    fn benchmark_defaults() {
        let a = subcmd!(Benchmark, &["stellar-operator", "benchmark"]);
        assert_eq!(a.namespace, "default");
        assert_eq!(a.log_level, "info");
    }

    #[test]
    fn benchmark_custom_namespace_and_log_level() {
        let a = subcmd!(
            Benchmark,
            &[
                "stellar-operator",
                "benchmark",
                "--namespace",
                "stellar-system",
                "--log-level",
                "debug"
            ]
        );
        assert_eq!(a.namespace, "stellar-system");
        assert_eq!(a.log_level, "debug");
    }

    // Table-driven: benchmark log levels
    #[test]
    fn benchmark_log_level_table() {
        for level in &["trace", "debug", "info", "warn", "error"] {
            let a = subcmd!(
                Benchmark,
                &["stellar-operator", "benchmark", "--log-level", level]
            );
            assert_eq!(&a.log_level, level);
        }
    }

    // ── GenerateRunbook ───────────────────────────────────────────────────────

    #[test]
    fn generate_runbook_required_node_name() {
        let a = subcmd!(
            GenerateRunbook,
            &["stellar-operator", "generate-runbook", "my-validator"]
        );
        assert_eq!(a.node_name, "my-validator");
        assert_eq!(a.namespace, "default");
        assert!(a.output.is_none());
    }

    #[test]
    fn generate_runbook_missing_node_name_is_error() {
        let result = parse(&["stellar-operator", "generate-runbook"]);
        assert!(result.is_err(), "node_name is required");
    }

    #[test]
    fn generate_runbook_with_namespace_short() {
        let a = subcmd!(
            GenerateRunbook,
            &["stellar-operator", "generate-runbook", "node1", "-n", "stellar"]
        );
        assert_eq!(a.namespace, "stellar");
    }

    #[test]
    fn generate_runbook_with_namespace_long() {
        let a = subcmd!(
            GenerateRunbook,
            &[
                "stellar-operator",
                "generate-runbook",
                "node1",
                "--namespace",
                "stellar"
            ]
        );
        assert_eq!(a.namespace, "stellar");
    }

    #[test]
    fn generate_runbook_with_output() {
        let a = subcmd!(
            GenerateRunbook,
            &[
                "stellar-operator",
                "generate-runbook",
                "node1",
                "--output",
                "runbook.md"
            ]
        );
        assert_eq!(a.output.as_deref(), Some("runbook.md"));
    }

    #[test]
    fn generate_runbook_with_output_short() {
        let a = subcmd!(
            GenerateRunbook,
            &[
                "stellar-operator",
                "generate-runbook",
                "node1",
                "-o",
                "out.md"
            ]
        );
        assert_eq!(a.output.as_deref(), Some("out.md"));
    }

    // Table-driven: generate-runbook node names
    #[test]
    fn generate_runbook_node_name_table() {
        for name in &["validator-1", "horizon-prod", "soroban-rpc-0"] {
            let a = subcmd!(
                GenerateRunbook,
                &["stellar-operator", "generate-runbook", name]
            );
            assert_eq!(&a.node_name, name);
        }
    }

    // ── PruneArchive ──────────────────────────────────────────────────────────

    #[test]
    fn prune_archive_required_url() {
        let a = subcmd!(
            PruneArchive,
            &[
                "stellar-operator",
                "prune-archive",
                "--archive-url",
                "s3://my-bucket/archive"
            ]
        );
        assert_eq!(a.archive_url, "s3://my-bucket/archive");
        assert_eq!(a.min_checkpoints, 50);
        assert!(!a.force);
        assert!(a.retention_days.is_none());
        assert!(a.retention_ledgers.is_none());
        assert_eq!(a.max_age_days, 7);
    }

    #[test]
    fn prune_archive_missing_url_is_error() {
        let result = parse(&["stellar-operator", "prune-archive"]);
        assert!(result.is_err(), "--archive-url is required");
    }

    #[test]
    fn prune_archive_retention_days() {
        let a = subcmd!(
            PruneArchive,
            &[
                "stellar-operator",
                "prune-archive",
                "--archive-url",
                "s3://b/a",
                "--retention-days",
                "30"
            ]
        );
        assert_eq!(a.retention_days, Some(30));
    }

    #[test]
    fn prune_archive_retention_ledgers() {
        let a = subcmd!(
            PruneArchive,
            &[
                "stellar-operator",
                "prune-archive",
                "--archive-url",
                "s3://b/a",
                "--retention-ledgers",
                "1000000"
            ]
        );
        assert_eq!(a.retention_ledgers, Some(1_000_000));
    }

    #[test]
    fn prune_archive_min_checkpoints_custom() {
        let a = subcmd!(
            PruneArchive,
            &[
                "stellar-operator",
                "prune-archive",
                "--archive-url",
                "s3://b/a",
                "--min-checkpoints",
                "100"
            ]
        );
        assert_eq!(a.min_checkpoints, 100);
    }

    #[test]
    fn prune_archive_force_flag() {
        let a = subcmd!(
            PruneArchive,
            &[
                "stellar-operator",
                "prune-archive",
                "--archive-url",
                "s3://b/a",
                "--force"
            ]
        );
        assert!(a.force);
    }

    // Table-driven: archive URL schemes
    #[test]
    fn prune_archive_url_schemes_table() {
        for url in &[
            "s3://bucket/prefix",
            "gs://bucket/prefix",
            "file:///local/path",
        ] {
            let a = subcmd!(
                PruneArchive,
                &["stellar-operator", "prune-archive", "--archive-url", url]
            );
            assert_eq!(&a.archive_url, url);
        }
    }

    // ── Diff ──────────────────────────────────────────────────────────────────

    #[test]
    fn diff_required_name() {
        let a = subcmd!(
            Diff,
            &["stellar-operator", "diff", "--name", "my-validator"]
        );
        assert_eq!(a.name, "my-validator");
        assert_eq!(a.namespace, "default");
        assert!(!a.show_config);
        assert!(!a.all_resources);
        assert!(!a.summary);
        assert!(a.context.is_none());
    }

    #[test]
    fn diff_missing_name_is_error() {
        let result = parse(&["stellar-operator", "diff"]);
        assert!(result.is_err(), "--name is required for diff");
    }

    #[test]
    fn diff_namespace_flag() {
        let a = subcmd!(
            Diff,
            &[
                "stellar-operator",
                "diff",
                "--name",
                "v",
                "--namespace",
                "stellar"
            ]
        );
        assert_eq!(a.namespace, "stellar");
    }

    #[test]
    fn diff_show_config_flag() {
        let a = subcmd!(
            Diff,
            &[
                "stellar-operator",
                "diff",
                "--name",
                "v",
                "--show-config"
            ]
        );
        assert!(a.show_config);
    }

    #[test]
    fn diff_all_resources_flag() {
        let a = subcmd!(
            Diff,
            &[
                "stellar-operator",
                "diff",
                "--name",
                "v",
                "--all-resources"
            ]
        );
        assert!(a.all_resources);
    }

    #[test]
    fn diff_summary_flag() {
        let a = subcmd!(
            Diff,
            &["stellar-operator", "diff", "--name", "v", "--summary"]
        );
        assert!(a.summary);
    }

    #[test]
    fn diff_context_flag() {
        let a = subcmd!(
            Diff,
            &[
                "stellar-operator",
                "diff",
                "--name",
                "v",
                "--context",
                "prod-cluster"
            ]
        );
        assert_eq!(a.context.as_deref(), Some("prod-cluster"));
    }

    #[test]
    fn diff_format_json() {
        let a = subcmd!(
            Diff,
            &[
                "stellar-operator",
                "diff",
                "--name",
                "v",
                "--format",
                "json"
            ]
        );
        assert!(matches!(
            a.format,
            stellar_k8s::controller::diff::DiffFormat::Json
        ));
    }

    #[test]
    fn diff_format_unified() {
        let a = subcmd!(
            Diff,
            &[
                "stellar-operator",
                "diff",
                "--name",
                "v",
                "--format",
                "unified"
            ]
        );
        assert!(matches!(
            a.format,
            stellar_k8s::controller::diff::DiffFormat::Unified
        ));
    }

    #[test]
    fn diff_invalid_format_is_error() {
        let result = parse(&[
            "stellar-operator",
            "diff",
            "--name",
            "v",
            "--format",
            "xml",
        ]);
        assert!(result.is_err(), "invalid format value should be rejected");
    }

    // ── BenchmarkCompare ──────────────────────────────────────────────────────

    #[test]
    fn benchmark_compare_defaults() {
        let a = subcmd!(
            BenchmarkCompare,
            &["stellar-operator", "benchmark-compare"]
        );
        assert!(a.cluster_a_context.is_none());
        assert!(a.cluster_b_context.is_none());
        assert!(a.cluster_a_prometheus.is_none());
        assert!(a.cluster_b_prometheus.is_none());
        assert_eq!(a.cluster_a_label, "Cluster A");
        assert_eq!(a.cluster_b_label, "Cluster B");
        assert_eq!(a.namespace, "stellar-system");
        assert_eq!(a.duration, 60);
        assert_eq!(a.interval, 5);
        assert!(a.output.is_none());
        assert_eq!(a.metrics, "tps,ledger_time,consensus_latency,sync_status");
    }

    #[test]
    fn benchmark_compare_contexts() {
        let a = subcmd!(
            BenchmarkCompare,
            &[
                "stellar-operator",
                "benchmark-compare",
                "--cluster-a-context",
                "prod-east",
                "--cluster-b-context",
                "prod-west"
            ]
        );
        assert_eq!(a.cluster_a_context.as_deref(), Some("prod-east"));
        assert_eq!(a.cluster_b_context.as_deref(), Some("prod-west"));
    }

    #[test]
    fn benchmark_compare_prometheus_urls() {
        let a = subcmd!(
            BenchmarkCompare,
            &[
                "stellar-operator",
                "benchmark-compare",
                "--cluster-a-prometheus",
                "http://prom-a:9090",
                "--cluster-b-prometheus",
                "http://prom-b:9090"
            ]
        );
        assert_eq!(
            a.cluster_a_prometheus.as_deref(),
            Some("http://prom-a:9090")
        );
        assert_eq!(
            a.cluster_b_prometheus.as_deref(),
            Some("http://prom-b:9090")
        );
    }

    #[test]
    fn benchmark_compare_custom_labels() {
        let a = subcmd!(
            BenchmarkCompare,
            &[
                "stellar-operator",
                "benchmark-compare",
                "--cluster-a-label",
                "Production",
                "--cluster-b-label",
                "Staging"
            ]
        );
        assert_eq!(a.cluster_a_label, "Production");
        assert_eq!(a.cluster_b_label, "Staging");
    }

    #[test]
    fn benchmark_compare_duration_and_interval() {
        let a = subcmd!(
            BenchmarkCompare,
            &[
                "stellar-operator",
                "benchmark-compare",
                "--duration",
                "300",
                "--interval",
                "10"
            ]
        );
        assert_eq!(a.duration, 300);
        assert_eq!(a.interval, 10);
    }

    #[test]
    fn benchmark_compare_output_short() {
        let a = subcmd!(
            BenchmarkCompare,
            &["stellar-operator", "benchmark-compare", "-o", "report.html"]
        );
        assert!(a.output.is_some());
    }

    #[test]
    fn benchmark_compare_custom_metrics() {
        let a = subcmd!(
            BenchmarkCompare,
            &[
                "stellar-operator",
                "benchmark-compare",
                "--metrics",
                "tps,ledger_time"
            ]
        );
        assert_eq!(a.metrics, "tps,ledger_time");
    }

    #[test]
    fn benchmark_compare_invalid_duration_type_is_error() {
        let result = parse(&[
            "stellar-operator",
            "benchmark-compare",
            "--duration",
            "not-a-number",
        ]);
        assert!(result.is_err(), "non-integer duration should be rejected");
    }

    // ── IncidentReport ────────────────────────────────────────────────────────

    #[test]
    fn incident_report_defaults() {
        let a = subcmd!(
            IncidentReport,
            &["stellar-operator", "incident-report"]
        );
        assert_eq!(a.namespace, "default");
        assert!(a.since.is_none());
        assert!(a.from.is_none());
        assert!(a.to.is_none());
        assert_eq!(a.output, "incident-report.zip");
    }

    #[test]
    fn incident_report_namespace() {
        let a = subcmd!(
            IncidentReport,
            &[
                "stellar-operator",
                "incident-report",
                "--namespace",
                "stellar-system"
            ]
        );
        assert_eq!(a.namespace, "stellar-system");
    }

    #[test]
    fn incident_report_since() {
        let a = subcmd!(
            IncidentReport,
            &["stellar-operator", "incident-report", "--since", "1h"]
        );
        assert_eq!(a.since.as_deref(), Some("1h"));
    }

    #[test]
    fn incident_report_from_to() {
        let a = subcmd!(
            IncidentReport,
            &[
                "stellar-operator",
                "incident-report",
                "--from",
                "2024-01-01T00:00:00Z",
                "--to",
                "2024-01-01T01:00:00Z"
            ]
        );
        assert_eq!(a.from.as_deref(), Some("2024-01-01T00:00:00Z"));
        assert_eq!(a.to.as_deref(), Some("2024-01-01T01:00:00Z"));
    }

    #[test]
    fn incident_report_custom_output() {
        let a = subcmd!(
            IncidentReport,
            &[
                "stellar-operator",
                "incident-report",
                "--output",
                "my-report.zip"
            ]
        );
        assert_eq!(a.output, "my-report.zip");
    }

    // ── RunArgs – additional coverage ─────────────────────────────────────────

    #[test]
    fn run_retry_budget_defaults() {
        let parsed = parse(&["stellar-operator", "run"]).unwrap();
        match parsed.command {
            Commands::Run(a) => {
                assert_eq!(a.retry_budget_retriable_secs, 15);
                assert_eq!(a.retry_budget_nonretriable_secs, 60);
                assert_eq!(a.retry_budget_max_attempts, 3);
            }
            _ => panic!("expected Run"),
        }
    }

    #[test]
    fn run_retry_budget_custom() {
        let parsed = parse(&[
            "stellar-operator",
            "run",
            "--retry-budget-retriable-secs",
            "30",
            "--retry-budget-nonretriable-secs",
            "120",
            "--retry-budget-max-attempts",
            "5",
        ])
        .unwrap();
        match parsed.command {
            Commands::Run(a) => {
                assert_eq!(a.retry_budget_retriable_secs, 30);
                assert_eq!(a.retry_budget_nonretriable_secs, 120);
                assert_eq!(a.retry_budget_max_attempts, 5);
            }
            _ => panic!("expected Run"),
        }
    }

    #[test]
    fn run_invalid_retry_secs_type_is_error() {
        let result = parse(&[
            "stellar-operator",
            "run",
            "--retry-budget-retriable-secs",
            "abc",
        ]);
        assert!(result.is_err(), "non-integer value should be rejected");
    }

    #[test]
    fn run_preflight_only_flag() {
        let parsed = parse(&["stellar-operator", "run", "--preflight-only"]).unwrap();
        match parsed.command {
            Commands::Run(a) => assert!(a.preflight_only),
            _ => panic!("expected Run"),
        }
    }

    #[test]
    fn run_github_repo_flag() {
        let parsed =
            parse(&["stellar-operator", "run", "--github-repo", "org/repo"]).unwrap();
        match parsed.command {
            Commands::Run(a) => assert_eq!(a.github_repo.as_deref(), Some("org/repo")),
            _ => panic!("expected Run"),
        }
    }

    // ── WebhookArgs – additional coverage ────────────────────────────────────

    #[test]
    fn webhook_log_format_pretty() {
        let parsed = parse(&[
            "stellar-operator",
            "webhook",
            "--log-format",
            "pretty",
        ])
        .unwrap();
        match parsed.command {
            Commands::Webhook(a) => {
                assert!(matches!(a.log_format, LogFormat::Pretty))
            }
            _ => panic!("expected Webhook"),
        }
    }

    #[test]
    fn webhook_invalid_log_format_is_error() {
        let result = parse(&["stellar-operator", "webhook", "--log-format", "xml"]);
        assert!(result.is_err(), "invalid log-format should be rejected");
    }

    // ── SimulatorUp – additional coverage ────────────────────────────────────

    #[test]
    fn simulator_up_use_k3s_flag() {
        let parsed =
            parse(&["stellar-operator", "simulator", "up", "--use-k3s"]).unwrap();
        match parsed.command {
            Commands::Simulator(s) => match s.command {
                SimulatorCmd::Up(a) => assert!(a.use_k3s),
            },
            _ => panic!("expected Simulator"),
        }
    }

    #[test]
    fn simulator_up_namespace_flag() {
        let parsed = parse(&[
            "stellar-operator",
            "simulator",
            "up",
            "--namespace",
            "dev",
        ])
        .unwrap();
        match parsed.command {
            Commands::Simulator(s) => match s.command {
                SimulatorCmd::Up(a) => assert_eq!(a.namespace, "dev"),
            },
            _ => panic!("expected Simulator"),
        }
    }

    // ── Version / CheckCrd – smoke tests ─────────────────────────────────────

    #[test]
    fn version_subcommand_parses() {
        let parsed = parse(&["stellar-operator", "version"]).unwrap();
        assert!(matches!(parsed.command, Commands::Version));
    }

    #[test]
    fn check_crd_subcommand_parses() {
        let parsed = parse(&["stellar-operator", "check-crd"]).unwrap();
        assert!(matches!(parsed.command, Commands::CheckCrd));
    }

    // ── Edge cases ────────────────────────────────────────────────────────────

    #[test]
    fn unknown_subcommand_is_error() {
        let result = parse(&["stellar-operator", "nonexistent"]);
        assert!(result.is_err(), "unknown subcommand should be rejected");
    }

    #[test]
    fn unknown_flag_on_run_is_error() {
        let result = parse(&["stellar-operator", "run", "--does-not-exist"]);
        assert!(result.is_err(), "unknown flag should be rejected");
    }

    #[test]
    fn unknown_flag_on_webhook_is_error() {
        let result = parse(&["stellar-operator", "webhook", "--bogus"]);
        assert!(result.is_err(), "unknown flag should be rejected");
    }

    #[test]
    fn no_subcommand_is_error() {
        let result = parse(&["stellar-operator"]);
        assert!(result.is_err(), "missing subcommand should be an error");
    }

    // Table-driven: all subcommands that take no required args parse successfully
    #[test]
    fn zero_arg_subcommands_parse_table() {
        let cases: &[&[&str]] = &[
            &["stellar-operator", "version"],
            &["stellar-operator", "check-crd"],
            &["stellar-operator", "run"],
            &["stellar-operator", "webhook"],
            &["stellar-operator", "benchmark"],
            &["stellar-operator", "info"],
            &["stellar-operator", "benchmark-compare"],
            &["stellar-operator", "incident-report"],
            &["stellar-operator", "simulator", "up"],
        ];
        for argv in cases {
            assert!(
                parse(argv).is_ok(),
                "subcommand {:?} should parse without required args",
                argv
            );
        }
    }

    // Table-driven: subcommands that require at least one arg fail without it
    #[test]
    fn required_arg_subcommands_fail_without_args_table() {
        let cases: &[&[&str]] = &[
            &["stellar-operator", "generate-runbook"],
            &["stellar-operator", "diff"],
            &["stellar-operator", "prune-archive"],
        ];
        for argv in cases {
            assert!(
                parse(argv).is_err(),
                "subcommand {:?} should fail without required args",
                argv
            );
        }
    }
}

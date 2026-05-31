#!/usr/bin/env python3
"""
Benchmark comparison and baseline management script for Stellar-K8s operator.

This script helps with:
- Comparing benchmark results against baselines
- Creating new baselines from benchmark runs
- Detecting performance regressions
"""

import argparse
import json
import sys
from pathlib import Path
from typing import Dict, List, Tuple, Any


class BenchmarkComparator:
    """Compare benchmark results and detect regressions."""
    
    def __init__(self, threshold: float = 10.0, verbose: bool = False):
        """
        Initialize comparator.
        
        Args:
            threshold: Regression threshold percentage (default: 10%)
            verbose: Enable verbose output
        """
        self.threshold = threshold
        self.verbose = verbose
        self.regressions = []
        
    def load_benchmark(self, filepath: str) -> Dict[str, Any]:
        """Load benchmark JSON file."""
        with open(filepath, 'r') as f:
            return json.load(f)
    
    def compare_metric(self, name: str, current: float, baseline: float) -> Tuple[bool, float]:
        """
        Compare a metric against baseline.
        
        Args:
            name: Metric name
            current: Current value
            baseline: Baseline value
            
        Returns:
            Tuple of (is_regression, percentage_change)
        """
        if baseline == 0:
            return False, 0.0
            
        change_pct = ((current - baseline) / baseline) * 100
        
        # For latency metrics, increase is bad
        # For throughput metrics, decrease is bad
        is_latency = any(x in name.lower() for x in ['latency', 'duration', 'time', 'p95', 'p99'])
        
        if is_latency:
            is_regression = change_pct > self.threshold
        else:
            is_regression = change_pct < -self.threshold
            
        return is_regression, change_pct
    
    def compare(self, current_file: str, baseline_file: str) -> Dict[str, Any]:
        """
        Compare current benchmark against baseline.
        
        Args:
            current_file: Path to current benchmark results
            baseline_file: Path to baseline benchmark results
            
        Returns:
            Comparison report dictionary
        """
        try:
            current = self.load_benchmark(current_file)
            baseline = self.load_benchmark(baseline_file)
        except FileNotFoundError as e:
            print(f"Error: Could not find benchmark file: {e}", file=sys.stderr)
            return {
                "overall_passed": True,
                "regressions": [],
                "summary": "Baseline not found, skipping comparison"
            }
        
        metrics_current = current.get('metrics', {})
        metrics_baseline = baseline.get('metrics', {})
        
        report = {
            "overall_passed": True,
            "regressions": [],
            "improvements": [],
            "summary": ""
        }
        
        # Compare each metric
        for metric_name, metric_data in metrics_current.items():
            if metric_name not in metrics_baseline:
                if self.verbose:
                    print(f"Skipping new metric: {metric_name}")
                continue
                
            # Handle nested metric data (e.g., {"avg": 100, "p95": 200})
            if isinstance(metric_data, dict):
                for sub_metric, value in metric_data.items():
                    baseline_value = metrics_baseline[metric_name].get(sub_metric)
                    if baseline_value is None:
                        continue
                        
                    full_name = f"{metric_name}.{sub_metric}"
                    is_regression, change_pct = self.compare_metric(
                        full_name, value, baseline_value
                    )
                    
                    if is_regression:
                        report["overall_passed"] = False
                        self.regressions.append({
                            "metric": full_name,
                            "current": value,
                            "baseline": baseline_value,
                            "change_pct": round(change_pct, 2)
                        })
                        if self.verbose:
                            print(f"⚠️  Regression detected: {full_name}")
                            print(f"   Current: {value}, Baseline: {baseline_value}")
                            print(f"   Change: {change_pct:+.2f}%")
                    elif abs(change_pct) > 5:  # Notable improvement
                        report["improvements"].append({
                            "metric": full_name,
                            "current": value,
                            "baseline": baseline_value,
                            "change_pct": round(change_pct, 2)
                        })
                        if self.verbose:
                            print(f"✅ Improvement: {full_name} ({change_pct:+.2f}%)")
            else:
                # Simple scalar metric
                baseline_value = metrics_baseline.get(metric_name)
                if baseline_value is None:
                    continue
                    
                is_regression, change_pct = self.compare_metric(
                    metric_name, metric_data, baseline_value
                )
                
                if is_regression:
                    report["overall_passed"] = False
                    self.regressions.append({
                        "metric": metric_name,
                        "current": metric_data,
                        "baseline": baseline_value,
                        "change_pct": round(change_pct, 2)
                    })
        
        report["regressions"] = self.regressions
        
        # Generate summary
        if report["overall_passed"]:
            report["summary"] = f"✅ All metrics within {self.threshold}% threshold"
        else:
            report["summary"] = f"❌ {len(self.regressions)} regression(s) detected"
            
        return report


def create_baseline(input_file: str, output_file: str, version: str) -> None:
    """
    Create a new baseline from benchmark results.
    
    Args:
        input_file: Path to benchmark results
        output_file: Path to save baseline
        version: Version identifier for the baseline
    """
    try:
        with open(input_file, 'r') as f:
            data = json.load(f)
        
        baseline = {
            "version": version,
            "timestamp": data.get("timestamp", ""),
            "metrics": data.get("metrics", {}),
            "metadata": {
                "git_sha": data.get("git_sha", ""),
                "run_id": data.get("run_id", "")
            }
        }
        
        output_path = Path(output_file)
        output_path.parent.mkdir(parents=True, exist_ok=True)
        
        with open(output_file, 'w') as f:
            json.dump(baseline, f, indent=2)
        
        print(f"✅ Created baseline: {output_file}")
        print(f"   Version: {version}")
        
    except Exception as e:
        print(f"Error creating baseline: {e}", file=sys.stderr)
        sys.exit(1)


def main():
    parser = argparse.ArgumentParser(
        description="Compare benchmark results and manage baselines"
    )
    subparsers = parser.add_subparsers(dest="command", help="Command to run")
    
    # Compare command
    compare_parser = subparsers.add_parser("compare", help="Compare benchmarks")
    compare_parser.add_argument("--current", required=True, help="Current benchmark file")
    compare_parser.add_argument("--baseline", required=True, help="Baseline benchmark file")
    compare_parser.add_argument("--threshold", type=float, default=10.0,
                               help="Regression threshold percentage (default: 10)")
    compare_parser.add_argument("--output", help="Output file for comparison report")
    compare_parser.add_argument("--fail-on-regression", action="store_true",
                               help="Exit with code 1 if regressions detected")
    compare_parser.add_argument("--verbose", action="store_true", help="Verbose output")
    
    # Baseline command
    baseline_parser = subparsers.add_parser("baseline", help="Create new baseline")
    baseline_parser.add_argument("--input", required=True, help="Input benchmark file")
    baseline_parser.add_argument("--output", required=True, help="Output baseline file")
    baseline_parser.add_argument("--version", required=True, help="Version identifier")
    
    args = parser.parse_args()
    
    if args.command == "compare":
        comparator = BenchmarkComparator(
            threshold=args.threshold,
            verbose=args.verbose
        )
        
        report = comparator.compare(args.current, args.baseline)
        
        print(f"\n{report['summary']}")
        
        if args.output:
            with open(args.output, 'w') as f:
                json.dump(report, f, indent=2)
            print(f"Report saved to: {args.output}")
        
        if args.fail_on_regression and not report["overall_passed"]:
            print(f"\n❌ Performance regression detected!", file=sys.stderr)
            for regression in report["regressions"]:
                print(f"   - {regression['metric']}: {regression['change_pct']:+.2f}%",
                      file=sys.stderr)
            sys.exit(1)
            
    elif args.command == "baseline":
        create_baseline(args.input, args.output, args.version)
    
    else:
        parser.print_help()
        sys.exit(1)


if __name__ == "__main__":
    main()

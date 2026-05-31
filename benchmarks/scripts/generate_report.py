#!/usr/bin/env python3
"""
Performance report generator for Stellar-K8s.
Generates an HTML report from benchmark results.
"""

import argparse
import json
from datetime import datetime

def generate_html(results, comparison, output_path):
    """Generate HTML report."""
    now = datetime.now().strftime("%Y-%m-%d %H:%M:%S")
    
    html = f"""
<!DOCTYPE html>
<html>
<head>
    <title>Stellar-K8s Performance Report</title>
    <style>
        body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; background: #f8fafc; color: #1e293b; padding: 40px; }}
        .container {{ max-width: 900px; margin: 0 auto; background: white; padding: 40px; border-radius: 12px; box-shadow: 0 4px 6px -1px rgb(0 0 0 / 0.1); }}
        h1 {{ font-size: 24px; font-weight: 800; margin-bottom: 8px; color: #0f172a; }}
        .timestamp {{ color: #64748b; font-size: 14px; margin-bottom: 32px; }}
        .summary {{ padding: 16px; border-radius: 8px; margin-bottom: 32px; font-weight: 600; }}
        .summary.passed {{ background: #f0fdf4; color: #166534; border: 1px solid #bbf7d0; }}
        .summary.failed {{ background: #fef2f2; color: #991b1b; border: 1px solid #fecaca; }}
        table {{ width: 100%; border-collapse: collapse; margin-bottom: 32px; }}
        th {{ text-align: left; background: #f1f5f9; padding: 12px; font-size: 14px; color: #475569; }}
        td {{ padding: 12px; border-bottom: 1px solid #e2e8f0; font-size: 14px; }}
        .regression {{ color: #dc2626; font-weight: 600; }}
        .improvement {{ color: #16a34a; font-weight: 600; }}
        .metric-name {{ font-weight: 500; }}
    </style>
</head>
<body>
    <div class="container">
        <h1>Performance Benchmark Report</h1>
        <div class="timestamp">Generated on {now}</div>
        
        <div class="summary {'passed' if comparison['overall_passed'] else 'failed'}">
            {comparison['summary']}
        </div>
        
        <h2>Metrics Comparison</h2>
        <table>
            <thead>
                <tr>
                    <th>Metric</th>
                    <th>Current</th>
                    <th>Baseline</th>
                    <th>Change</th>
                </tr>
            </thead>
            <tbody>
    """
    
    # Combined list of regressions and improvements for the table
    all_metrics = comparison.get('regressions', []) + comparison.get('improvements', [])
    
    # Also add stable metrics from current results that aren't regressions/improvements
    # This is simplified for now
    
    for m in comparison.get('regressions', []):
        html += f"""
                <tr>
                    <td class="metric-name">{m['metric']}</td>
                    <td>{m['current']}</td>
                    <td>{m['baseline']}</td>
                    <td class="regression">{m['change_pct']}%</td>
                </tr>
        """
        
    for m in comparison.get('improvements', []):
        html += f"""
                <tr>
                    <td class="metric-name">{m['metric']}</td>
                    <td>{m['current']}</td>
                    <td>{m['baseline']}</td>
                    <td class="improvement">+{m['change_pct']}%</td>
                </tr>
        """
        
    html += """
            </tbody>
        </table>
        
        <p style="color: #64748b; font-size: 12px;">
            Note: Regressions are flagged if they exceed the configured threshold.
        </p>
    </div>
</body>
</html>
    """
    
    with open(output_path, 'w') as f:
        f.write(html)

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--results", required=True)
    parser.add_argument("--comparison", required=True)
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    
    with open(args.results, 'r') as f:
        results = json.load(f)
        
    with open(args.comparison, 'r') as f:
        comparison = json.load(f)
        
    generate_html(results, comparison, args.output)
    print(f"✅ Report generated: {args.output}")

if __name__ == "__main__":
    main()

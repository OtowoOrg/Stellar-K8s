#!/usr/bin/env python3
"""
Generate Soroban-specific Grafana Dashboard
This script creates a comprehensive monitoring dashboard for Soroban RPC nodes
"""

import json

def create_soroban_dashboard():
    """Create the complete Soroban dashboard JSON"""
    
    dashboard = {
        "annotations": {
            "list": [
                {
                    "builtIn": 1,
                    "datasource": {"type": "grafana", "uid": "-- Grafana --"},
                    "enable": True,
                    "hide": True,
                    "iconColor": "rgba(0, 211, 255, 1)",
                    "name": "Annotations & Alerts",
                    "type": "dashboard"
                }
            ]
        },
        "editable": True,
        "fiscalYearStartMonth": 0,
        "graphTooltip": 1,
        "id": None,
        "links": [],
        "liveNow": False,
        "panels": []
    }
    
    # Panel 1: Soroban RPC Health Status
    dashboard["panels"].append({
        "datasource": {"type": "prometheus", "uid": "${datasource}"},
        "description": "Health status of Soroban RPC nodes",
        "fieldConfig": {
            "defaults": {
                "color": {"mode": "thresholds"},
                "mappings": [{
                    "options": {
                        "0": {"color": "red", "index": 1, "text": "Down"},
                        "1": {"color": "green", "index": 0, "text": "Healthy"}
                    },
                    "type": "value"
                }],
                "thresholds": {
                    "mode": "absolute",
                    "steps": [
                        {"color": "red", "value": None},
                        {"color": "green", "value": 1}
                    ]
                }
            }
        },
        "gridPos": {"h": 4, "w": 6, "x": 0, "y": 0},
        "id": 1,
        "options": {
            "colorMode": "background",
            "graphMode": "none",
            "justifyMode": "auto",
            "orientation": "auto",
            "reduceOptions": {"calcs": ["lastNotNull"], "fields": "", "values": False},
            "textMode": "auto"
        },
        "pluginVersion": "10.0.0",
        "targets": [{
            "datasource": {"type": "prometheus", "uid": "${datasource}"},
            "expr": "up{job=\"soroban-rpc\"}",
            "instant": True,
            "legendFormat": "{{instance}}",
            "refId": "A"
        }],
        "title": "Soroban RPC Health",
        "type": "stat"
    })
    
    # Panel 2: Latest Ledger Ingested
    dashboard["panels"].append({
        "datasource": {"type": "prometheus", "uid": "${datasource}"},
        "description": "Latest ledger sequence ingested by Soroban RPC",
        "fieldConfig": {
            "defaults": {
                "color": {"mode": "palette-classic"},
                "mappings": [],
                "thresholds": {
                    "mode": "absolute",
                    "steps": [{"color": "green", "value": None}]
                }
            }
        },
        "gridPos": {"h": 4, "w": 6, "x": 6, "y": 0},
        "id": 2,
        "options": {
            "colorMode": "value",
            "graphMode": "area",
            "justifyMode": "auto",
            "orientation": "auto",
            "reduceOptions": {"calcs": ["lastNotNull"], "fields": "", "values": False},
            "textMode": "auto"
        },
        "pluginVersion": "10.0.0",
        "targets": [{
            "datasource": {"type": "prometheus", "uid": "${datasource}"},
            "expr": "soroban_rpc_ingest_local_latest_ledger",
            "legendFormat": "{{instance}}",
            "refId": "A"
        }],
        "title": "Latest Ledger Ingested",
        "type": "stat"
    })
    
    # Panel 3: Transaction Ingestion Rate
    dashboard["panels"].append({
        "datasource": {"type": "prometheus", "uid": "${datasource}"},
        "description": "Rate of Soroban transactions ingested (10m sliding window)",
        "fieldConfig": {
            "defaults": {
                "color": {"mode": "palette-classic"},
                "mappings": [],
                "thresholds": {
                    "mode": "absolute",
                    "steps": [{"color": "green", "value": None}]
                },
                "unit": "ops"
            }
        },
        "gridPos": {"h": 4, "w": 6, "x": 12, "y": 0},
        "id": 3,
        "options": {
            "colorMode": "value",
            "graphMode": "area",
            "justifyMode": "auto",
            "orientation": "auto",
            "reduceOptions": {"calcs": ["lastNotNull"], "fields": "", "values": False},
            "textMode": "auto"
        },
        "pluginVersion": "10.0.0",
        "targets": [{
            "datasource": {"type": "prometheus", "uid": "${datasource}"},
            "expr": "rate(soroban_rpc_transactions_count[5m])",
            "legendFormat": "{{instance}}",
            "refId": "A"
        }],
        "title": "Transaction Ingestion Rate",
        "type": "stat"
    })
    
    # Panel 4: Events Ingestion Rate
    dashboard["panels"].append({
        "datasource": {"type": "prometheus", "uid": "${datasource}"},
        "description": "Rate of Soroban events ingested (10m sliding window)",
        "fieldConfig": {
            "defaults": {
                "color": {"mode": "palette-classic"},
                "mappings": [],
                "thresholds": {
                    "mode": "absolute",
                    "steps": [{"color": "green", "value": None}]
                },
                "unit": "ops"
            }
        },
        "gridPos": {"h": 4, "w": 6, "x": 18, "y": 0},
        "id": 4,
        "options": {
            "colorMode": "value",
            "graphMode": "area",
            "justifyMode": "auto",
            "orientation": "auto",
            "reduceOptions": {"calcs": ["lastNotNull"], "fields": "", "values": False},
            "textMode": "auto"
        },
        "pluginVersion": "10.0.0",
        "targets": [{
            "datasource": {"type": "prometheus", "uid": "${datasource}"},
            "expr": "rate(soroban_rpc_events_count[5m])",
            "legendFormat": "{{instance}}",
            "refId": "A"
        }],
        "title": "Events Ingestion Rate",
        "type": "stat"
    })
    
    # Panel 5: Wasm Execution Time Histogram
    dashboard["panels"].append({
        "datasource": {"type": "prometheus", "uid": "${datasource}"},
        "description": "Distribution of Wasm host function execution times",
        "fieldConfig": {
            "defaults": {
                "color": {"mode": "palette-classic"},
                "custom": {
                    "axisBorderShow": False,
                    "axisCenteredZero": False,
                    "axisColorMode": "text",
                    "axisLabel": "",
                    "axisPlacement": "auto",
                    "barAlignment": 0,
                    "drawStyle": "bars",
                    "fillOpacity": 80,
                    "gradientMode": "none",
                    "hideFrom": {"legend": False, "tooltip": False, "viz": False},
                    "insertNulls": False,
                    "lineInterpolation": "linear",
                    "lineWidth": 1,
                    "pointSize": 5,
                    "scaleDistribution": {"type": "linear"},
                    "showPoints": "auto",
                    "spanNulls": False,
                    "stacking": {"group": "A", "mode": "none"},
                    "thresholdsStyle": {"mode": "off"}
                },
                "mappings": [],
                "thresholds": {
                    "mode": "absolute",
                    "steps": [{"color": "green", "value": None}]
                },
                "unit": "µs"
            }
        },
        "gridPos": {"h": 8, "w": 12, "x": 0, "y": 4},
        "id": 5,
        "options": {
            "legend": {
                "calcs": ["mean", "max", "min"],
                "displayMode": "table",
                "placement": "bottom",
                "showLegend": True
            },
            "tooltip": {"mode": "multi", "sort": "desc"}
        },
        "targets": [{
            "datasource": {"type": "prometheus", "uid": "${datasource}"},
            "expr": "histogram_quantile(0.50, sum(rate(soroban_rpc_wasm_execution_duration_microseconds_bucket[5m])) by (le, instance))",
            "legendFormat": "p50 - {{instance}}",
            "refId": "A"
        }, {
            "datasource": {"type": "prometheus", "uid": "${datasource}"},
            "expr": "histogram_quantile(0.95, sum(rate(soroban_rpc_wasm_execution_duration_microseconds_bucket[5m])) by (le, instance))",
            "legendFormat": "p95 - {{instance}}",
            "refId": "B"
        }, {
            "datasource": {"type": "prometheus", "uid": "${datasource}"},
            "expr": "histogram_quantile(0.99, sum(rate(soroban_rpc_wasm_execution_duration_microseconds_bucket[5m])) by (le, instance))",
            "legendFormat": "p99 - {{instance}}",
            "refId": "C"
        }],
        "title": "Wasm Execution Time (Histogram)",
        "type": "timeseries"
    })
    
    # Panel 6: Contract Storage Fee Distribution
    dashboard["panels"].append({
        "datasource": {"type": "prometheus", "uid": "${datasource}"},
        "description": "Distribution of storage fees charged for contract operations",
        "fieldConfig": {
            "defaults": {
                "color": {"mode": "palette-classic"},
                "custom": {
                    "axisBorderShow": False,
                    "axisCenteredZero": False,
                    "axisColorMode": "text",
                    "axisLabel": "",
                    "axisPlacement": "auto",
                    "barAlignment": 0,
                    "drawStyle": "line",
                    "fillOpacity": 10,
                    "gradientMode": "none",
                    "hideFrom": {"legend": False, "tooltip": False, "viz": False},
                    "insertNulls": False,
                    "lineInterpolation": "linear",
                    "lineWidth": 1,
                    "pointSize": 5,
                    "scaleDistribution": {"type": "linear"},
                    "showPoints": "auto",
                    "spanNulls": False,
                    "stacking": {"group": "A", "mode": "none"},
                    "thresholdsStyle": {"mode": "off"}
                },
                "mappings": [],
                "thresholds": {
                    "mode": "absolute",
                    "steps": [{"color": "green", "value": None}]
                },
                "unit": "stroops"
            }
        },
        "gridPos": {"h": 8, "w": 12, "x": 12, "y": 4},
        "id": 6,
        "options": {
            "legend": {
                "calcs": ["mean", "max", "sum"],
                "displayMode": "table",
                "placement": "bottom",
                "showLegend": True
            },
            "tooltip": {"mode": "multi", "sort": "desc"}
        },
        "targets": [{
            "datasource": {"type": "prometheus", "uid": "${datasource}"},
            "expr": "histogram_quantile(0.50, sum(rate(soroban_rpc_contract_storage_fee_stroops_bucket[5m])) by (le, instance))",
            "legendFormat": "p50 - {{instance}}",
            "refId": "A"
        }, {
            "datasource": {"type": "prometheus", "uid": "${datasource}"},
            "expr": "histogram_quantile(0.95, sum(rate(soroban_rpc_contract_storage_fee_stroops_bucket[5m])) by (le, instance))",
            "legendFormat": "p95 - {{instance}}",
            "refId": "B"
        }, {
            "datasource": {"type": "prometheus", "uid": "${datasource}"},
            "expr": "histogram_quantile(0.99, sum(rate(soroban_rpc_contract_storage_fee_stroops_bucket[5m])) by (le, instance))",
            "legendFormat": "p99 - {{instance}}",
            "refId": "C"
        }],
        "title": "Contract Storage Fee Distribution",
        "type": "timeseries"
    })
    
    # Panel 7: Resource Consumption per Contract Invocation - CPU
    dashboard["panels"].append({
        "datasource": {"type": "prometheus", "uid": "${datasource}"},
        "description": "CPU time consumed per contract invocation",
        "fieldConfig": {
            "defaults": {
                "color": {"mode": "palette-classic"},
                "custom": {
                    "axisBorderShow": False,
                    "axisCenteredZero": False,
                    "axisColorMode": "text",
                    "axisLabel": "",
                    "axisPlacement": "auto",
                    "barAlignment": 0,
                    "drawStyle": "line",
                    "fillOpacity": 10,
                    "gradientMode": "none",
                    "hideFrom": {"legend": False, "tooltip": False, "viz": False},
                    "insertNulls": False,
                    "lineInterpolation": "linear",
                    "lineWidth": 1,
                    "pointSize": 5,
                    "scaleDistribution": {"type": "linear"},
                    "showPoints": "auto",
                    "spanNulls": False,
                    "stacking": {"group": "A", "mode": "none"},
                    "thresholdsStyle": {"mode": "off"}
                },
                "mappings": [],
                "thresholds": {
                    "mode": "absolute",
                    "steps": [
                        {"color": "green", "value": None},
                        {"color": "yellow", "value": 70},
                        {"color": "red", "value": 90}
                    ]
                },
                "unit": "percent"
            }
        },
        "gridPos": {"h": 8, "w": 12, "x": 0, "y": 12},
        "id": 7,
        "options": {
            "legend": {
                "calcs": ["mean", "max"],
                "displayMode": "table",
                "placement": "bottom",
                "showLegend": True
            },
            "tooltip": {"mode": "multi", "sort": "desc"}
        },
        "targets": [{
            "datasource": {"type": "prometheus", "uid": "${datasource}"},
            "expr": "rate(process_cpu_seconds_total{job=\"soroban-rpc\"}[5m]) * 100",
            "legendFormat": "CPU - {{instance}}",
            "refId": "A"
        }, {
            "datasource": {"type": "prometheus", "uid": "${datasource}"},
            "expr": "avg(rate(soroban_rpc_contract_invocation_cpu_instructions[5m])) by (instance)",
            "legendFormat": "CPU Instructions - {{instance}}",
            "refId": "B"
        }],
        "title": "Resource Consumption - CPU per Invocation",
        "type": "timeseries"
    })
    
    # Panel 8: Resource Consumption per Contract Invocation - Memory
    dashboard["panels"].append({
        "datasource": {"type": "prometheus", "uid": "${datasource}"},
        "description": "Wasm VM memory usage per contract invocation",
        "fieldConfig": {
            "defaults": {
                "color": {"mode": "palette-classic"},
                "custom": {
                    "axisBorderShow": False,
                    "axisCenteredZero": False,
                    "axisColorMode": "text",
                    "axisLabel": "",
                    "axisPlacement": "auto",
                    "barAlignment": 0,
                    "drawStyle": "line",
                    "fillOpacity": 10,
                    "gradientMode": "none",
                    "hideFrom": {"legend": False, "tooltip": False, "viz": False},
                    "insertNulls": False,
                    "lineInterpolation": "linear",
                    "lineWidth": 1,
                    "pointSize": 5,
                    "scaleDistribution": {"type": "linear"},
                    "showPoints": "auto",
                    "spanNulls": False,
                    "stacking": {"group": "A", "mode": "none"},
                    "thresholdsStyle": {"mode": "off"}
                },
                "mappings": [],
                "thresholds": {
                    "mode": "absolute",
                    "steps": [
                        {"color": "green", "value": None},
                        {"color": "yellow", "value": 1073741824},
                        {"color": "red", "value": 2147483648}
                    ]
                },
                "unit": "bytes"
            }
        },
        "gridPos": {"h": 8, "w": 12, "x": 12, "y": 12},
        "id": 8,
        "options": {
            "legend": {
                "calcs": ["mean", "max"],
                "displayMode": "table",
                "placement": "bottom",
                "showLegend": True
            },
            "tooltip": {"mode": "multi", "sort": "desc"}
        },
        "targets": [{
            "datasource": {"type": "prometheus", "uid": "${datasource}"},
            "expr": "process_resident_memory_bytes{job=\"soroban-rpc\"}",
            "legendFormat": "Process Memory - {{instance}}",
            "refId": "A"
        }, {
            "datasource": {"type": "prometheus", "uid": "${datasource}"},
            "expr": "avg(soroban_rpc_wasm_vm_memory_bytes) by (instance)",
            "legendFormat": "Wasm VM Memory - {{instance}}",
            "refId": "B"
        }, {
            "datasource": {"type": "prometheus", "uid": "${datasource}"},
            "expr": "avg(soroban_rpc_contract_invocation_memory_bytes) by (instance)",
            "legendFormat": "Per Invocation - {{instance}}",
            "refId": "C"
        }],
        "title": "Resource Consumption - Memory per Invocation",
        "type": "timeseries"
    })
    
    # Panel 9: Soroban Transaction Success/Failure Rate
    dashboard["panels"].append({
        "datasource": {"type": "prometheus", "uid": "${datasource}"},
        "description": "Success and failure rates of Soroban transactions",
        "fieldConfig": {
            "defaults": {
                "color": {"mode": "palette-classic"},
                "custom": {
                    "axisBorderShow": False,
                    "axisCenteredZero": False,
                    "axisColorMode": "text",
                    "axisLabel": "",
                    "axisPlacement": "auto",
                    "barAlignment": 0,
                    "drawStyle": "line",
                    "fillOpacity": 10,
                    "gradientMode": "none",
                    "hideFrom": {"legend": False, "tooltip": False, "viz": False},
                    "insertNulls": False,
                    "lineInterpolation": "linear",
                    "lineWidth": 2,
                    "pointSize": 5,
                    "scaleDistribution": {"type": "linear"},
                    "showPoints": "auto",
                    "spanNulls": False,
                    "stacking": {"group": "A", "mode": "percent"},
                    "thresholdsStyle": {"mode": "off"}
                },
                "mappings": [],
                "thresholds": {
                    "mode": "absolute",
                    "steps": [{"color": "green", "value": None}]
                },
                "unit": "percentunit"
            }
        },
        "gridPos": {"h": 8, "w": 12, "x": 0, "y": 20},
        "id": 9,
        "options": {
            "legend": {
                "calcs": ["mean", "last"],
                "displayMode": "table",
                "placement": "bottom",
                "showLegend": True
            },
            "tooltip": {"mode": "multi", "sort": "desc"}
        },
        "targets": [{
            "datasource": {"type": "prometheus", "uid": "${datasource}"},
            "expr": "sum(rate(soroban_rpc_transaction_result_total{result=\"success\"}[5m])) by (instance) / sum(rate(soroban_rpc_transaction_result_total[5m])) by (instance)",
            "legendFormat": "Success Rate - {{instance}}",
            "refId": "A"
        }, {
            "datasource": {"type": "prometheus", "uid": "${datasource}"},
            "expr": "sum(rate(soroban_rpc_transaction_result_total{result=\"failed\"}[5m])) by (instance) / sum(rate(soroban_rpc_transaction_result_total[5m])) by (instance)",
            "legendFormat": "Failure Rate - {{instance}}",
            "refId": "B"
        }],
        "title": "Soroban Transaction Success/Failure Rate",
        "type": "timeseries"
    })
    
    # Panel 10: Contract Invocation Rate by Type
    dashboard["panels"].append({
        "datasource": {"type": "prometheus", "uid": "${datasource}"},
        "description": "Rate of contract invocations grouped by contract type",
        "fieldConfig": {
            "defaults": {
                "color": {"mode": "palette-classic"},
                "custom": {
                    "axisBorderShow": False,
                    "axisCenteredZero": False,
                    "axisColorMode": "text",
                    "axisLabel": "",
                    "axisPlacement": "auto",
                    "barAlignment": 0,
                    "drawStyle": "line",
                    "fillOpacity": 10,
                    "gradientMode": "none",
                    "hideFrom": {"legend": False, "tooltip": False, "viz": False},
                    "insertNulls": False,
                    "lineInterpolation": "linear",
                    "lineWidth": 1,
                    "pointSize": 5,
                    "scaleDistribution": {"type": "linear"},
                    "showPoints": "auto",
                    "spanNulls": False,
                    "stacking": {"group": "A", "mode": "normal"},
                    "thresholdsStyle": {"mode": "off"}
                },
                "mappings": [],
                "thresholds": {
                    "mode": "absolute",
                    "steps": [{"color": "green", "value": None}]
                },
                "unit": "ops"
            }
        },
        "gridPos": {"h": 8, "w": 12, "x": 12, "y": 20},
        "id": 10,
        "options": {
            "legend": {
                "calcs": ["mean", "max", "sum"],
                "displayMode": "table",
                "placement": "bottom",
                "showLegend": True
            },
            "tooltip": {"mode": "multi", "sort": "desc"}
        },
        "targets": [{
            "datasource": {"type": "prometheus", "uid": "${datasource}"},
            "expr": "sum(rate(soroban_rpc_contract_invocations_total[5m])) by (contract_type, instance)",
            "legendFormat": "{{contract_type}} - {{instance}}",
            "refId": "A"
        }],
        "title": "Contract Invocation Rate by Type",
        "type": "timeseries"
    })
    
    # Panel 11: Database Round Trip Time
    dashboard["panels"].append({
        "datasource": {"type": "prometheus", "uid": "${datasource}"},
        "description": "Time required to run SELECT 1 query in the database",
        "fieldConfig": {
            "defaults": {
                "color": {"mode": "palette-classic"},
                "custom": {
                    "axisBorderShow": False,
                    "axisCenteredZero": False,
                    "axisColorMode": "text",
                    "axisLabel": "",
                    "axisPlacement": "auto",
                    "barAlignment": 0,
                    "drawStyle": "line",
                    "fillOpacity": 10,
                    "gradientMode": "none",
                    "hideFrom": {"legend": False, "tooltip": False, "viz": False},
                    "insertNulls": False,
                    "lineInterpolation": "linear",
                    "lineWidth": 1,
                    "pointSize": 5,
                    "scaleDistribution": {"type": "linear"},
                    "showPoints": "auto",
                    "spanNulls": False,
                    "stacking": {"group": "A", "mode": "none"},
                    "thresholdsStyle": {"mode": "off"}
                },
                "mappings": [],
                "thresholds": {
                    "mode": "absolute",
                    "steps": [
                        {"color": "green", "value": None},
                        {"color": "yellow", "value": 0.1},
                        {"color": "red", "value": 0.5}
                    ]
                },
                "unit": "s"
            }
        },
        "gridPos": {"h": 8, "w": 12, "x": 0, "y": 28},
        "id": 11,
        "options": {
            "legend": {
                "calcs": ["mean", "max"],
                "displayMode": "table",
                "placement": "bottom",
                "showLegend": True
            },
            "tooltip": {"mode": "multi", "sort": "desc"}
        },
        "targets": [{
            "datasource": {"type": "prometheus", "uid": "${datasource}"},
            "expr": "soroban_rpc_db_round_trip_time_seconds",
            "legendFormat": "{{instance}}",
            "refId": "A"
        }],
        "title": "Database Round Trip Time",
        "type": "timeseries"
    })
    
    # Panel 12: Host Function Call Distribution
    dashboard["panels"].append({
        "datasource": {"type": "prometheus", "uid": "${datasource}"},
        "description": "Distribution of host function calls by function name",
        "fieldConfig": {
            "defaults": {
                "color": {"mode": "palette-classic"},
                "custom": {
                    "hideFrom": {"legend": False, "tooltip": False, "viz": False}
                },
                "mappings": []
            }
        },
        "gridPos": {"h": 8, "w": 12, "x": 12, "y": 28},
        "id": 12,
        "options": {
            "displayLabels": ["percent"],
            "legend": {
                "displayMode": "table",
                "placement": "right",
                "showLegend": True,
                "values": ["value", "percent"]
            },
            "pieType": "donut",
            "reduceOptions": {
                "calcs": ["lastNotNull"],
                "fields": "",
                "values": False
            },
            "tooltip": {"mode": "single", "sort": "none"}
        },
        "targets": [{
            "datasource": {"type": "prometheus", "uid": "${datasource}"},
            "expr": "sum(increase(soroban_rpc_host_function_calls_total[5m])) by (function_name)",
            "legendFormat": "{{function_name}}",
            "refId": "A"
        }],
        "title": "Host Function Call Distribution",
        "type": "piechart"
    })
    
    # Panel 13: RPC Request Latency
    dashboard["panels"].append({
        "datasource": {"type": "prometheus", "uid": "${datasource}"},
        "description": "Latency of JSON RPC requests by method",
        "fieldConfig": {
            "defaults": {
                "color": {"mode": "palette-classic"},
                "custom": {
                    "axisBorderShow": False,
                    "axisCenteredZero": False,
                    "axisColorMode": "text",
                    "axisLabel": "",
                    "axisPlacement": "auto",
                    "barAlignment": 0,
                    "drawStyle": "line",
                    "fillOpacity": 10,
                    "gradientMode": "none",
                    "hideFrom": {"legend": False, "tooltip": False, "viz": False},
                    "insertNulls": False,
                    "lineInterpolation": "linear",
                    "lineWidth": 1,
                    "pointSize": 5,
                    "scaleDistribution": {"type": "linear"},
                    "showPoints": "auto",
                    "spanNulls": False,
                    "stacking": {"group": "A", "mode": "none"},
                    "thresholdsStyle": {"mode": "off"}
                },
                "mappings": [],
                "thresholds": {
                    "mode": "absolute",
                    "steps": [
                        {"color": "green", "value": None},
                        {"color": "yellow", "value": 0.1},
                        {"color": "red", "value": 1}
                    ]
                },
                "unit": "s"
            }
        },
        "gridPos": {"h": 8, "w": 24, "x": 0, "y": 36},
        "id": 13,
        "options": {
            "legend": {
                "calcs": ["mean", "max", "min"],
                "displayMode": "table",
                "placement": "right",
                "showLegend": True
            },
            "tooltip": {"mode": "multi", "sort": "desc"}
        },
        "targets": [{
            "datasource": {"type": "prometheus", "uid": "${datasource}"},
            "expr": "histogram_quantile(0.50, sum(rate(soroban_rpc_request_duration_seconds_bucket[5m])) by (le, method, instance))",
            "legendFormat": "p50 - {{method}} - {{instance}}",
            "refId": "A"
        }, {
            "datasource": {"type": "prometheus", "uid": "${datasource}"},
            "expr": "histogram_quantile(0.95, sum(rate(soroban_rpc_request_duration_seconds_bucket[5m])) by (le, method, instance))",
            "legendFormat": "p95 - {{method}} - {{instance}}",
            "refId": "B"
        }, {
            "datasource": {"type": "prometheus", "uid": "${datasource}"},
            "expr": "histogram_quantile(0.99, sum(rate(soroban_rpc_request_duration_seconds_bucket[5m])) by (le, method, instance))",
            "legendFormat": "p99 - {{method}} - {{instance}}",
            "refId": "C"
        }],
        "title": "RPC Request Latency by Method",
        "type": "timeseries"
    })
    
    # Panel 14: Ledger Ingestion Lag
    dashboard["panels"].append({
        "datasource": {"type": "prometheus", "uid": "${datasource}"},
        "description": "Lag between network ledger and locally ingested ledger",
        "fieldConfig": {
            "defaults": {
                "color": {"mode": "thresholds"},
                "mappings": [],
                "thresholds": {
                    "mode": "absolute",
                    "steps": [
                        {"color": "green", "value": None},
                        {"color": "yellow", "value": 5},
                        {"color": "red", "value": 10}
                    ]
                },
                "unit": "ledgers"
            }
        },
        "gridPos": {"h": 4, "w": 6, "x": 0, "y": 44},
        "id": 14,
        "options": {
            "colorMode": "value",
            "graphMode": "area",
            "justifyMode": "auto",
            "orientation": "auto",
            "reduceOptions": {"calcs": ["lastNotNull"], "fields": "", "values": False},
            "textMode": "auto"
        },
        "pluginVersion": "10.0.0",
        "targets": [{
            "datasource": {"type": "prometheus", "uid": "${datasource}"},
            "expr": "soroban_rpc_ingest_ledger_lag",
            "legendFormat": "{{instance}}",
            "refId": "A"
        }],
        "title": "Ledger Ingestion Lag",
        "type": "stat"
    })
    
    # Panel 15: Active Goroutines
    dashboard["panels"].append({
        "datasource": {"type": "prometheus", "uid": "${datasource}"},
        "description": "Number of active goroutines in the Soroban RPC process",
        "fieldConfig": {
            "defaults": {
                "color": {"mode": "palette-classic"},
                "mappings": [],
                "thresholds": {
                    "mode": "absolute",
                    "steps": [
                        {"color": "green", "value": None},
                        {"color": "yellow", "value": 1000},
                        {"color": "red", "value": 5000}
                    ]
                }
            }
        },
        "gridPos": {"h": 4, "w": 6, "x": 6, "y": 44},
        "id": 15,
        "options": {
            "colorMode": "value",
            "graphMode": "area",
            "justifyMode": "auto",
            "orientation": "auto",
            "reduceOptions": {"calcs": ["lastNotNull"], "fields": "", "values": False},
            "textMode": "auto"
        },
        "pluginVersion": "10.0.0",
        "targets": [{
            "datasource": {"type": "prometheus", "uid": "${datasource}"},
            "expr": "go_goroutines{job=\"soroban-rpc\"}",
            "legendFormat": "{{instance}}",
            "refId": "A"
        }],
        "title": "Active Goroutines",
        "type": "stat"
    })
    
    # Panel 16: Memory Allocations
    dashboard["panels"].append({
        "datasource": {"type": "prometheus", "uid": "${datasource}"},
        "description": "Rate of memory allocations in the Go runtime",
        "fieldConfig": {
            "defaults": {
                "color": {"mode": "palette-classic"},
                "mappings": [],
                "thresholds": {
                    "mode": "absolute",
                    "steps": [{"color": "green", "value": None}]
                },
                "unit": "Bps"
            }
        },
        "gridPos": {"h": 4, "w": 6, "x": 12, "y": 44},
        "id": 16,
        "options": {
            "colorMode": "value",
            "graphMode": "area",
            "justifyMode": "auto",
            "orientation": "auto",
            "reduceOptions": {"calcs": ["lastNotNull"], "fields": "", "values": False},
            "textMode": "auto"
        },
        "pluginVersion": "10.0.0",
        "targets": [{
            "datasource": {"type": "prometheus", "uid": "${datasource}"},
            "expr": "rate(go_memstats_alloc_bytes_total{job=\"soroban-rpc\"}[5m])",
            "legendFormat": "{{instance}}",
            "refId": "A"
        }],
        "title": "Memory Allocation Rate",
        "type": "stat"
    })
    
    # Panel 17: GC Pause Time
    dashboard["panels"].append({
        "datasource": {"type": "prometheus", "uid": "${datasource}"},
        "description": "Go garbage collection pause time",
        "fieldConfig": {
            "defaults": {
                "color": {"mode": "thresholds"},
                "mappings": [],
                "thresholds": {
                    "mode": "absolute",
                    "steps": [
                        {"color": "green", "value": None},
                        {"color": "yellow", "value": 0.01},
                        {"color": "red", "value": 0.1}
                    ]
                },
                "unit": "s"
            }
        },
        "gridPos": {"h": 4, "w": 6, "x": 18, "y": 44},
        "id": 17,
        "options": {
            "colorMode": "value",
            "graphMode": "area",
            "justifyMode": "auto",
            "orientation": "auto",
            "reduceOptions": {"calcs": ["lastNotNull"], "fields": "", "values": False},
            "textMode": "auto"
        },
        "pluginVersion": "10.0.0",
        "targets": [{
            "datasource": {"type": "prometheus", "uid": "${datasource}"},
            "expr": "rate(go_gc_duration_seconds_sum{job=\"soroban-rpc\"}[5m]) / rate(go_gc_duration_seconds_count{job=\"soroban-rpc\"}[5m])",
            "legendFormat": "{{instance}}",
            "refId": "A"
        }],
        "title": "GC Pause Time (avg)",
        "type": "stat"
    })
    
    # Add dashboard metadata
    dashboard["refresh"] = "10s"
    dashboard["schemaVersion"] = 38
    dashboard["style"] = "dark"
    dashboard["tags"] = ["stellar", "soroban", "smart-contracts", "kubernetes"]
    dashboard["templating"] = {
        "list": [{
            "current": {"selected": False, "text": "Prometheus", "value": "prometheus"},
            "hide": 0,
            "includeAll": False,
            "label": "Datasource",
            "multi": False,
            "name": "datasource",
            "options": [],
            "query": "prometheus",
            "refresh": 1,
            "regex": "",
            "skipUrlSync": False,
            "type": "datasource"
        }]
    }
    dashboard["time"] = {"from": "now-1h", "to": "now"}
    dashboard["timepicker"] = {}
    dashboard["timezone"] = "browser"
    dashboard["title"] = "Soroban RPC - Smart Contract Monitoring"
    dashboard["uid"] = "soroban_rpc_monitoring"
    dashboard["version"] = 1
    dashboard["weekStart"] = ""
    
    return dashboard

if __name__ == "__main__":
    dashboard = create_soroban_dashboard()
    with open("monitoring/grafana-soroban.json", "w") as f:
        json.dump(dashboard, f, indent=2)
    print("✓ Soroban dashboard generated successfully")

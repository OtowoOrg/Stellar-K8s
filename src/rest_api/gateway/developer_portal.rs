//! Developer Portal HTML
//!
//! Interactive API explorer with Swagger UI

pub const DEVELOPER_PORTAL_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Stellar Operator API - Developer Portal</title>
    <script src="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5.10.5/swagger-ui-bundle.js"></script>
    <link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5.10.5/swagger-ui.css">
    <style>
        :root {
            --primary: #7c3aed;
            --primary-dark: #5b21b6;
            --secondary: #06b6d4;
            --bg: #0f172a;
            --surface: #1e293b;
            --surface-light: #334155;
            --text: #f1f5f9;
            --text-muted: #94a3b8;
            --border: #475569;
            --success: #10b981;
            --warning: #f59e0b;
            --error: #ef4444;
        }
        
        * { box-sizing: border-box; margin: 0; padding: 0; }
        
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: var(--bg);
            color: var(--text);
            min-height: 100vh;
        }
        
        header {
            background: var(--surface);
            border-bottom: 1px solid var(--border);
            padding: 1rem 2rem;
            display: flex;
            align-items: center;
            justify-content: space-between;
        }
        
        .logo {
            display: flex;
            align-items: center;
            gap: 1rem;
        }
        
        .logo h1 {
            font-size: 1.5rem;
            font-weight: 700;
            background: linear-gradient(135deg, var(--primary), var(--secondary));
            -webkit-background-clip: text;
            -webkit-text-fill-color: transparent;
        }
        
        .nav {
            display: flex;
            gap: 1.5rem;
        }
        
        .nav a {
            color: var(--text-muted);
            text-decoration: none;
            padding: 0.5rem 1rem;
            border-radius: 0.5rem;
            transition: all 0.2s;
        }
        
        .nav a:hover, .nav a.active {
            color: var(--text);
            background: var(--surface-light);
        }
        
        main {
            max-width: 1400px;
            margin: 0 auto;
            padding: 2rem;
        }
        
        .hero {
            text-align: center;
            padding: 3rem 0;
            margin-bottom: 2rem;
        }
        
        .hero h2 {
            font-size: 2.5rem;
            margin-bottom: 1rem;
        }
        
        .hero p {
            color: var(--text-muted);
            font-size: 1.25rem;
            max-width: 600px;
            margin: 0 auto;
        }
        
        .tabs {
            display: flex;
            gap: 0.5rem;
            margin-bottom: 1.5rem;
            border-bottom: 1px solid var(--border);
            padding-bottom: 1rem;
        }
        
        .tab {
            padding: 0.75rem 1.5rem;
            background: transparent;
            border: none;
            color: var(--text-muted);
            cursor: pointer;
            border-radius: 0.5rem;
            transition: all 0.2s;
            font-size: 1rem;
        }
        
        .tab:hover {
            background: var(--surface-light);
        }
        
        .tab.active {
            background: var(--primary);
            color: white;
        }
        
        .content-section {
            display: none;
        }
        
        .content-section.active {
            display: block;
        }
        
        .card {
            background: var(--surface);
            border: 1px solid var(--border);
            border-radius: 1rem;
            padding: 1.5rem;
            margin-bottom: 1rem;
        }
        
        .card h3 {
            margin-bottom: 0.5rem;
            color: var(--text);
        }
        
        .card p {
            color: var(--text-muted);
            margin-bottom: 1rem;
        }
        
        .stats-grid {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
            gap: 1rem;
            margin-bottom: 2rem;
        }
        
        .stat-card {
            background: var(--surface);
            border: 1px solid var(--border);
            border-radius: 1rem;
            padding: 1.5rem;
            text-align: center;
        }
        
        .stat-card .value {
            font-size: 2rem;
            font-weight: 700;
            color: var(--primary);
        }
        
        .stat-card .label {
            color: var(--text-muted);
            margin-top: 0.5rem;
        }
        
        code {
            background: var(--surface-light);
            padding: 0.25rem 0.5rem;
            border-radius: 0.25rem;
            font-family: 'Fira Code', monospace;
            font-size: 0.9em;
        }
        
        pre {
            background: var(--surface-light);
            padding: 1rem;
            border-radius: 0.5rem;
            overflow-x: auto;
            margin: 1rem 0;
        }
        
        .btn {
            display: inline-flex;
            align-items: center;
            gap: 0.5rem;
            padding: 0.75rem 1.5rem;
            background: var(--primary);
            color: white;
            border: none;
            border-radius: 0.5rem;
            cursor: pointer;
            font-size: 1rem;
            transition: all 0.2s;
        }
        
        .btn:hover {
            background: var(--primary-dark);
            transform: translateY(-1px);
        }
        
        .btn-secondary {
            background: var(--surface-light);
        }
        
        .endpoint-grid {
            display: grid;
            gap: 1rem;
        }
        
        .endpoint {
            background: var(--surface);
            border: 1px solid var(--border);
            border-radius: 0.75rem;
            padding: 1rem 1.5rem;
            display: flex;
            align-items: center;
            gap: 1rem;
        }
        
        .method {
            padding: 0.25rem 0.75rem;
            border-radius: 0.25rem;
            font-weight: 600;
            font-size: 0.8rem;
            text-transform: uppercase;
        }
        
        .method.get { background: #10b981; color: white; }
        .method.post { background: #3b82f6; color: white; }
        .method.put { background: #f59e0b; color: white; }
        .method.patch { background: #8b5cf6; color: white; }
        .method.delete { background: #ef4444; color: white; }
        
        .endpoint-path {
            flex: 1;
            font-family: 'Fira Code', monospace;
        }
        
        .endpoint-desc {
            color: var(--text-muted);
            font-size: 0.9rem;
        }
        
        footer {
            text-align: center;
            padding: 2rem;
            color: var(--text-muted);
            border-top: 1px solid var(--border);
            margin-top: 3rem;
        }
        
        #swagger-ui {
            background: var(--surface);
            border-radius: 1rem;
            padding: 1rem;
        }
        
        .api-key-form {
            display: flex;
            gap: 1rem;
            align-items: center;
            margin-bottom: 1rem;
        }
        
        .api-key-form input {
            flex: 1;
            padding: 0.75rem;
            background: var(--surface-light);
            border: 1px solid var(--border);
            border-radius: 0.5rem;
            color: var(--text);
            font-size: 1rem;
        }
        
        .api-key-form input::placeholder {
            color: var(--text-muted);
        }
        
        .badge {
            display: inline-block;
            padding: 0.25rem 0.5rem;
            background: var(--surface-light);
            border-radius: 0.25rem;
            font-size: 0.8rem;
            margin-left: 0.5rem;
        }
        
        .badge.auth { background: var(--secondary); }
        .badge.rate-limit { background: var(--warning); color: #000; }
    </style>
</head>
<body>
    <header>
        <div class="logo">
            <h1>🔮 Stellar Operator API</h1>
        </div>
        <nav class="nav">
            <a href="#" class="active" onclick="showTab('explorer')">API Explorer</a>
            <a href="#" onclick="showTab('docs')">Documentation</a>
            <a href="#" onclick="showTab('sdk')">SDK</a>
            <a href="#" onclick="showTab('status')">Status</a>
        </nav>
    </header>
    
    <main>
        <div class="hero">
            <h2>Developer Portal</h2>
            <p>Explore, test, and integrate with the Stellar Operator API. 
               Get started with authentication, explore endpoints, and build your integration.</p>
        </div>
        
        <div class="api-key-form">
            <input type="text" id="apiKeyInput" placeholder="Enter your API key to test authenticated endpoints">
            <button class="btn" onclick="setApiKey()">Set API Key</button>
        </div>
        
        <div class="tabs">
            <button class="tab active" onclick="showTab('explorer')">API Explorer</button>
            <button class="tab" onclick="showTab('docs')">Documentation</button>
            <button class="tab" onclick="showTab('sdk')">SDK & Code Samples</button>
            <button class="tab" onclick="showTab('status')">API Status</button>
        </div>
        
        <div id="explorer" class="content-section active">
            <div id="swagger-ui"></div>
        </div>
        
        <div id="docs" class="content-section">
            <div class="card">
                <h3>Getting Started</h3>
                <p>The Stellar Operator API provides programmatic access to manage Stellar validators on Kubernetes.</p>
                <pre><code># Example: List all StellarNodes
curl -H "Authorization: Bearer YOUR_TOKEN" \
  https://api.example.com/api/v1/nodes

# Response:
{
  "items": [
    {
      "metadata": {
        "name": "validator-1",
        "namespace": "stellar"
      },
      "spec": {
        "stellarImage": "stellar/stellar-core:19.0.0"
      }
    }
  ]
}</code></pre>
            </div>
            
            <div class="card">
                <h3>Authentication</h3>
                <p>The API supports multiple authentication methods:</p>
                <ul style="margin-left: 1.5rem; color: var(--text-muted);">
                    <li><strong>JWT Bearer Token</strong> - Include in Authorization header</li>
                    <li><strong>API Key</strong> - Include in X-API-Key header</li>
                    <li><strong>OAuth 2.0</strong> - For user-based authentication</li>
                </ul>
            </div>
            
            <div class="card">
                <h3>Rate Limiting</h3>
                <p>API requests are rate-limited to ensure fair usage:</p>
                <ul style="margin-left: 1.5rem; color: var(--text-muted);">
                    <li>100 requests per minute (default)</li>
                    <li>10,000 requests per hour</li>
                    <li>100,000 requests per day</li>
                </ul>
                <p style="margin-top: 1rem;">Rate limit information is returned in response headers:</p>
                <pre><code>X-RateLimit-Limit-Minute: 100
X-RateLimit-Remaining-Minute: 95
X-RateLimit-Reset: 1640000000</code></pre>
            </div>
        </div>
        
        <div id="sdk" class="content-section">
            <div class="card">
                <h3>Python SDK</h3>
                <pre><code>pip install stellar-operator-sdk

from stellar_operator import Client

client = Client(
    api_key="YOUR_API_KEY",
    base_url="https://api.example.com"
)

# List all validators
nodes = client.list_nodes()
for node in nodes.items:
    print(f"{node.metadata.name}: {node.status.phase}")</code></pre>
            </div>
            
            <div class="card">
                <h3>JavaScript/TypeScript SDK</h3>
                <pre><code>npm install @stellar/operator-sdk

import { Client } from '@stellar/operator-sdk';

const client = new Client({
  apiKey: process.env.STELLAR_API_KEY,
  baseUrl: 'https://api.example.com'
});

// Get a specific node
const node = await client.nodes.get('my-namespace', 'validator-1');
console.log(node.status.phase);</code></pre>
            </div>
            
            <div class="card">
                <h3>Go SDK</h3>
                <pre><code>go get github.com/stellar/operator-sdk

package main

import (
    "fmt"
    "github.com/stellar/operator-sdk/pkg/client"
)

func main() {
    c := client.New(client.Config{
        APIKey: "YOUR_API_KEY",
        BaseURL: "https://api.example.com",
    })
    
    nodes, _ := c.ListNodes(context.Background())
    for _, n := range nodes.Items {
        fmt.Printf("%s: %s\n", n.Name, n.Status.Phase)
    }
}</code></pre>
            </div>
        </div>
        
        <div id="status" class="content-section">
            <div class="stats-grid">
                <div class="stat-card">
                    <div class="value" id="uptime">--</div>
                    <div class="label">Uptime</div>
                </div>
                <div class="stat-card">
                    <div class="value" id="total-requests">--</div>
                    <div class="label">Total Requests</div>
                </div>
                <div class="stat-card">
                    <div class="value" id="avg-latency">--</div>
                    <div class="label">Avg Latency (ms)</div>
                </div>
                <div class="stat-card">
                    <div class="value" id="error-rate">--</div>
                    <div class="label">Error Rate</div>
                </div>
            </div>
            
            <div class="card">
                <h3>System Status</h3>
                <p id="status-text">Loading...</p>
            </div>
            
            <div class="card">
                <h3>Recent Incidents</h3>
                <p>No recent incidents reported.</p>
            </div>
        </div>
    </main>
    
    <footer>
        <p>Stellar Operator API &copy; 2024. Built with ❤️ for the Stellar community.</p>
    </footer>
    
    <script>
        let apiKey = localStorage.getItem('stellar_api_key') || '';
        if (apiKey) {
            document.getElementById('apiKeyInput').value = apiKey;
        }
        
        function setApiKey() {
            const input = document.getElementById('apiKeyInput');
            apiKey = input.value;
            localStorage.setItem('stellar_api_key', apiKey);
            
            // Update Swagger UI auth
            if (window.swaggerUi) {
                window.swaggerUi.authActions.authorize({
                    apiKey: {
                        name: 'apiKey',
                        value: apiKey,
                        type: 'apiKey'
                    }
                });
            }
            
            alert('API Key set! You can now test authenticated endpoints.');
        }
        
        function showTab(tabId) {
            document.querySelectorAll('.content-section').forEach(s => s.classList.remove('active'));
            document.querySelectorAll('.tab').forEach(t => t.classList.remove('active'));
            document.querySelectorAll('.nav a').forEach(a => a.classList.remove('active'));
            
            document.getElementById(tabId).classList.add('active');
            event.target.classList.add('active');
            
            // Update nav
            const navMap = { 'explorer': 0, 'docs': 1, 'sdk': 2, 'status': 3 };
            document.querySelectorAll('.nav a')[navMap[tabId]].classList.add('active');
            
            if (tabId === 'status') {
                loadStatus();
            }
        }
        
        async function loadStatus() {
            try {
                const res = await fetch('/api/v1/gateway/health');
                const data = await res.json();
                
                document.getElementById('uptime').textContent = formatUptime(data.uptime_seconds);
                document.getElementById('total-requests').textContent = data.total_requests?.toLocaleString() || '--';
                document.getElementById('avg-latency').textContent = Math.round(data.avg_latency_ms) + 'ms';
                document.getElementById('error-rate').textContent = (data.error_rate * 100).toFixed(2) + '%';
                
                const statusText = {
                    'Healthy': '✅ All systems operational',
                    'Degraded': '⚠️ Some degradation detected',
                    'Unhealthy': '❌ System issues detected'
                };
                document.getElementById('status-text').textContent = statusText[data.status] || 'Unknown';
            } catch (e) {
                document.getElementById('status-text').textContent = 'Unable to load status';
            }
        }
        
        function formatUptime(seconds) {
            const days = Math.floor(seconds / 86400);
            const hours = Math.floor((seconds % 86400) / 3600);
            if (days > 0) return `${days}d ${hours}h`;
            if (hours > 0) return `${hours}h`;
            return `${seconds}s`;
        }
        
        // Initialize Swagger UI
        window.onload = function() {
            const ui = SwaggerUIBundle({
                url: '/api/v1/gateway/openapi.json',
                dom_id: '#swagger-ui',
                deepLinking: true,
                presets: [
                    SwaggerUIBundle.presets.apis,
                    SwaggerUIBundle.SwaggerUIStandalonePreset
                ],
                layout: 'StandaloneLayout',
                requestInterceptor: function(req) {
                    if (apiKey) {
                        req.headers['X-API-Key'] = apiKey;
                    }
                    return req;
                }
            });
            
            window.swaggerUi = ui;
        };
    </script>
</body>
</html>"#;
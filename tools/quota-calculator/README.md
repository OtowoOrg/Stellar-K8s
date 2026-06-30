# Quota Calculator (web + CLI)

Scaffolded TypeScript/JavaScript quota calculator. It provides:

- Core calculation library (GCP-anchored sample constants)
- Browser-based UI using Tailwind CSS and vanilla ES modules
- Node.js CLI for quick JSON output
- Documentation describing formulas and assumptions

Quick start (requires Node.js):

Serve the web UI (from the `tools/quota-calculator` folder):

```bash
npm install -g http-server
npm run serve
# then open http://localhost:8080
```

CLI example:

```bash
node cli.mjs --replicas 10 --cpuPerPod 0.5 --memPerPodGb 0.5 --initialStorageGb 500 --monthlyGrowthPercent 4 --rps 50 --avgResponseKb 100
```

#!/usr/bin/env node
import { calculateCpuMemory, estimateStorage, estimateNetwork, estimateGcpCosts, runAll } from './lib/calculations.mjs';

function parseArgs(argv) {
    const args = {};
    for (let i = 2; i < argv.length; i++) {
        const a = argv[i];
        if (a === '--replicas') args.replicas = Number(argv[++i]);
        else if (a === '--cpuPerPod') args.cpuPerPod = Number(argv[++i]);
        else if (a === '--memPerPodGb') args.memPerPodGb = Number(argv[++i]);
        else if (a === '--initialStorageGb') args.initialStorageGb = Number(argv[++i]);
        else if (a === '--monthlyGrowthPercent') args.monthlyGrowthPercent = Number(argv[++i]);
        else if (a === '--months') args.months = Number(argv[++i]);
        else if (a === '--rps') args.rps = Number(argv[++i]);
        else if (a === '--avgResponseKb') args.avgResponseKb = Number(argv[++i]);
        else if (a === '--help') args.help = true;
    }
    return args;
}

function printHelp() {
    console.log(`quota-calculator CLI
Usage: node cli.mjs [options]

Options:
  --replicas <n>
  --cpuPerPod <vCPU>
  --memPerPodGb <GB>
  --initialStorageGb <GB>
  --monthlyGrowthPercent <%>
  --months <n>
  --rps <requests per second>
  --avgResponseKb <KB>
`);
}

async function main() {
    const args = parseArgs(process.argv);
    if (args.help) { printHelp(); return; }

    const cpuMem = calculateCpuMemory({ replicas: args.replicas || 3, cpuPerPod: args.cpuPerPod || 0.5, memPerPodGb: args.memPerPodGb || 0.5 });
    const storage = estimateStorage(args.initialStorageGb || 100, args.monthlyGrowthPercent || 5, args.months || 12);
    const network = estimateNetwork({ rps: args.rps || 10, avgResponseKb: args.avgResponseKb || 50 });
    const costs = estimateGcpCosts({ totalCpu: cpuMem.totalCpu, totalMemGb: cpuMem.totalMemGb, egressGbPerMonth: network.gbPerMonth, storageGbMonth: storage.avgGb });

    const out = { cpuMem, storage, network, costs };
    console.log(JSON.stringify(out, null, 2));
}

main();

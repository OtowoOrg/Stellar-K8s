// Browser copy of core calculation library (same as lib)
export const DEFAULT_PRICES = {
    vcpuHour: 0.04,
    memGbHour: 0.005,
    storagePerGbMonth: 0.02,
    egressPerGb: 0.12
};

export const NODE_TYPES = {
    "n2-standard-2": { cpu: 2, memGb: 8 },
    "n2-standard-4": { cpu: 4, memGb: 16 },
    "n2-highmem-8": { cpu: 8, memGb: 64 }
};

export function calculateCpuMemory({ replicas = 3, cpuPerPod = 0.5, memPerPodGb = 0.5, bufferPercent = 20, nodeType = 'n2-standard-4' } = {}) {
    const safety = 1 + bufferPercent / 100;
    const totalCpu = replicas * cpuPerPod * safety;
    const totalMemGb = replicas * memPerPodGb * safety;
    const spec = NODE_TYPES[nodeType] || NODE_TYPES['n2-standard-4'];
    const nodesByCpu = Math.ceil(totalCpu / spec.cpu);
    const nodesByMem = Math.ceil(totalMemGb / spec.memGb);
    const recommendedNodes = Math.max(nodesByCpu, nodesByMem, 1);
    return { totalCpu, totalMemGb, recommendedNodes, nodeSpec: spec };
}

export function estimateStorage(initialGb = 100, monthlyGrowthPercent = 5, months = 12) {
    const monthly = [];
    let current = initialGb;
    for (let m = 1; m <= months; m++) {
        current = current * (1 + monthlyGrowthPercent / 100);
        monthly.push({ month: m, gb: Number(current.toFixed(3)) });
    }
    const finalGb = Number(monthly[monthly.length - 1].gb.toFixed(3));
    const avgGb = Number((monthly.reduce((s, x) => s + x.gb, 0) / monthly.length).toFixed(3));
    return { monthly, finalGb, avgGb };
}

export function estimateNetwork({ rps = 10, avgResponseKb = 50, hoursPerDay = 24, daysPerMonth = 30 } = {}) {
    const secondsPerMonth = hoursPerDay * daysPerMonth * 3600;
    const gbPerMonth = (rps * avgResponseKb * secondsPerMonth) / (1024 * 1024);
    const peakMbps = (rps * avgResponseKb * 8) / 1024;
    return { gbPerMonth: Number(gbPerMonth.toFixed(3)), peakMbps: Number(peakMbps.toFixed(3)) };
}

export function estimateGcpCosts({ totalCpu = 4, totalMemGb = 16, egressGbPerMonth = 100, storageGbMonth = 200, prices = DEFAULT_PRICES, hoursPerMonth = 24 * 30 } = {}) {
    const vcpuHours = totalCpu * hoursPerMonth;
    const memGbHours = totalMemGb * hoursPerMonth;
    const computeCost = (vcpuHours * prices.vcpuHour) + (memGbHours * prices.memGbHour);
    const storageCost = storageGbMonth * prices.storagePerGbMonth;
    const networkCost = egressGbPerMonth * prices.egressPerGb;
    const total = Number((computeCost + storageCost + networkCost).toFixed(4));
    return { computeCost: Number(computeCost.toFixed(4)), storageCost: Number(storageCost.toFixed(4)), networkCost: Number(networkCost.toFixed(4)), total };
}

export function runAll(inputs = {}) {
    const cpuMem = calculateCpuMemory(inputs.cpuMem || {});
    const storage = estimateStorage(inputs.storageInitialGb || 100, inputs.monthlyGrowthPercent || 5, inputs.months || 12);
    const network = estimateNetwork(inputs.network || {});
    const costs = estimateGcpCosts({ totalCpu: cpuMem.totalCpu, totalMemGb: cpuMem.totalMemGb, egressGbPerMonth: network.gbPerMonth, storageGbMonth: storage.avgGb });
    return { cpuMem, storage, network, costs };
}

export default {
    calculateCpuMemory,
    estimateStorage,
    estimateNetwork,
    estimateGcpCosts,
    runAll,
    NODE_TYPES,
    DEFAULT_PRICES
};

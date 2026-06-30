import { runAll } from './calculations.mjs';

const $ = sel => document.querySelector(sel);

const controls = {
    replicas: $('#replicas'),
    cpuPerPod: $('#cpuPerPod'),
    memPerPodGb: $('#memPerPodGb'),
    initialStorageGb: $('#initialStorageGb'),
    monthlyGrowthPercent: $('#monthlyGrowthPercent'),
    rps: $('#rps'),
    avgResponseKb: $('#avgResponseKb')
};

const displays = {
    replicas: $('#replicas-val'),
    cpuPerPod: $('#cpuPerPod-val'),
    memPerPodGb: $('#memPerPodGb-val'),
    monthlyGrowthPercent: $('#monthlyGrowthPercent-val'),
    results: $('#results')
};

function readInputs() {
    return {
        cpuMem: {
            replicas: Number(controls.replicas.value),
            cpuPerPod: Number(controls.cpuPerPod.value),
            memPerPodGb: Number(controls.memPerPodGb.value)
        },
        storageInitialGb: Number(controls.initialStorageGb.value),
        monthlyGrowthPercent: Number(controls.monthlyGrowthPercent.value),
        months: 12,
        network: { rps: Number(controls.rps.value), avgResponseKb: Number(controls.avgResponseKb.value) }
    };
}

function render() {
    displays.replicas.textContent = controls.replicas.value;
    displays.cpuPerPod.textContent = Number(controls.cpuPerPod.value).toFixed(1);
    displays.memPerPodGb.textContent = Number(controls.memPerPodGb.value).toFixed(1);
    displays.monthlyGrowthPercent.textContent = `${controls.monthlyGrowthPercent.value}%`;

    const out = runAll(readInputs());
    displays.results.textContent = JSON.stringify(out, null, 2);
}

Object.values(controls).forEach(el => {
    el.addEventListener('input', render);
});

render();

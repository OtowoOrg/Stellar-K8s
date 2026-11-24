use prometheus::{HistogramVec, Opts, register_histogram_vec};
use lazy_static::lazy_static;

lazy_static! {
    pub static ref GAS_USAGE: HistogramVec = register_histogram_vec!(
        Opts::new("soroban_contract_cpu_instructions", "CPU instructions used by contract"),
        &["contract_id", "function_name"]
    ).unwrap();

    pub static ref MEM_USAGE: HistogramVec = register_histogram_vec!(
        Opts::new("soroban_contract_memory_bytes", "Memory bytes used by contract"),
        &["contract_id", "function_name"]
    ).unwrap();
}

pub fn record_metrics(contract_id: &str, function_name: &str, cpu: u64, mem: u64) {
    GAS_USAGE.with_label_values(&[contract_id, function_name]).observe(cpu as f64);
    MEM_USAGE.with_label_values(&[contract_id, function_name]).observe(mem as f64);
}

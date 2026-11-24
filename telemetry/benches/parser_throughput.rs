use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_parse_invocation_line(c: &mut Criterion) {
    let line = r#"{"timestamp":"2025-01-15T10:30:00Z","level":"info","msg":"contract_invocation","contract_id":"CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC","cpu_instructions":142000,"memory_bytes":524288,"wasm_execution_duration_us":1500,"storage_fee_stroops":100,"host_function":"invoke","success":true}"#;

    c.bench_function("parse_invocation_line", |b| {
        b.iter(|| {
            let record = stellar_telemetry::parser::parse_invocation_line(black_box(line));
            black_box(record);
        });
    });
}

fn bench_parse_minimal_line(c: &mut Criterion) {
    let line = r#"{"contract_id":"C1","cpu_instructions":50000,"memory_bytes":262144}"#;

    c.bench_function("parse_minimal_line", |b| {
        b.iter(|| {
            let record = stellar_telemetry::parser::parse_invocation_line(black_box(line));
            black_box(record);
        });
    });
}

fn bench_parse_invalid_line(c: &mut Criterion) {
    let line = "this is not json at all";

    c.bench_function("parse_invalid_line", |b| {
        b.iter(|| {
            let _ = stellar_telemetry::parser::parse_invocation_line(black_box(line));
        });
    });
}

fn bench_parse_then_record(c: &mut Criterion) {
    let line = r#"{"timestamp":"2025-01-15T10:30:00Z","contract_id":"CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC","cpu_instructions":142000,"memory_bytes":524288,"wasm_execution_duration_us":1500,"storage_fee_stroops":100,"host_function":"invoke","success":true}"#;
    let exporter = stellar_telemetry::exporter::MetricsExporter::new();

    c.bench_function("parse_then_record", |b| {
        b.iter(|| {
            let record = stellar_telemetry::parser::parse_invocation_line(black_box(line)).unwrap();
            exporter.record_invocation(&record);
        });
    });
}

fn bench_high_throughput(c: &mut Criterion) {
    let contracts = [
        r#"{"contract_id":"C_ALPHA","cpu_instructions":120000,"memory_bytes":524288,"host_function":"invoke","success":true}"#,
        r#"{"contract_id":"C_BETA","cpu_instructions":85000,"memory_bytes":262144,"host_function":"invoke","success":true}"#,
        r#"{"contract_id":"C_GAMMA","cpu_instructions":200000,"memory_bytes":1048576,"host_function":"upload_wasm","success":false}"#,
    ];

    let exporter = stellar_telemetry::exporter::MetricsExporter::new();

    c.bench_function("parse_1000_invocations", |b| {
        b.iter(|| {
            for i in 0..1000 {
                let line = contracts[i % contracts.len()];
                let record =
                    stellar_telemetry::parser::parse_invocation_line(black_box(line)).unwrap();
                exporter.record_invocation(&record);
            }
        });
    });
}

criterion_group!(
    benches,
    bench_parse_invocation_line,
    bench_parse_minimal_line,
    bench_parse_invalid_line,
    bench_parse_then_record,
    bench_high_throughput,
);
criterion_main!(benches);

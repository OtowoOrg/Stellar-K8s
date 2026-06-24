//! stellar-scaffold: CLI for scaffolding Stellar-K8s operators

use clap::Parser;
use stellar_k8s::sdk::codegen::{generate_controller_stub, render_controller_source};

#[derive(Parser)]
#[command(name = "stellar-scaffold")]
#[command(about = "Scaffold a new Stellar-K8s operator controller from a CRD kind")]
struct Args {
    /// CRD API group
    #[arg(long, default_value = "stellar.org")]
    group: String,

    /// CRD API version
    #[arg(long, default_value = "v1alpha1")]
    version: String,

    /// CRD Kind name (PascalCase)
    kind: String,

    /// Print generated Rust source to stdout
    #[arg(long)]
    print: bool,
}

fn main() {
    let args = Args::parse();
    let stub = generate_controller_stub(&args.group, &args.version, &args.kind);
    if args.print {
        print!("{}", render_controller_source(&stub));
    } else {
        println!("Controller stub: {}", stub.reconciler_fn);
        println!("Module: src/controller/{}.rs", stub.module_name);
        println!("Run with --print to emit reconciler source");
    }
}

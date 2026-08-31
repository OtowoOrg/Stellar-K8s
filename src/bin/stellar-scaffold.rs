// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
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

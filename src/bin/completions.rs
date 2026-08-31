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
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{generate, Shell};
use std::io;

#[derive(Parser)]
#[command(name = "stellar-operator")]
#[command(about = "Generate shell completion scripts for stellar-operator CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate shell completion scripts
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
    },
}

// Minimal Args structure matching main.rs for completion generation
#[derive(Parser)]
#[command(
    author,
    version,
    about = "Stellar-K8s: Cloud-Native Kubernetes Operator for Stellar Infrastructure"
)]
struct Args {
    #[command(subcommand)]
    command: MainCommands,
}

#[derive(Subcommand)]
enum MainCommands {
    /// Run the operator reconciliation loop
    Run,
    /// Run the admission webhook server
    Webhook,
    /// Show version and build information
    Version,
    /// Show cluster information (node count) for a namespace
    Info,
    /// Local simulator (kind/k3s + operator + demo validators)
    Simulator,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Completions { shell } => {
            let mut cmd = Args::command();
            let bin_name = "stellar-operator";
            generate(shell, &mut cmd, bin_name, &mut io::stdout());
        }
    }
}

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
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use clap::Parser;
use serde::Deserialize;
use serde_yaml::Value;

mod validator;
use validator::validate_document;

/// Simple manifest validator for StellarNode Kubernetes manifests
#[derive(Parser)]
struct Args {
    /// Files or directories to validate
    #[arg(required = true)]
    inputs: Vec<PathBuf>,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let mut had_error = false;

    for input in &args.inputs {
        if input.is_dir() {
            for entry in std::fs::read_dir(input)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_file() {
                    validate_file(&path, &mut had_error)?;
                }
            }
        } else {
            validate_file(input, &mut had_error)?;
        }
    }

    if had_error {
        std::process::exit(2);
    }

    Ok(())
}

fn validate_file(path: &PathBuf, had_error: &mut bool) -> anyhow::Result<()> {
    let f = File::open(path)?;
    let reader = BufReader::new(f);

    let docs = serde_yaml::Deserializer::from_reader(reader);
    for (i, doc) in docs.enumerate() {
        let v: Value = Value::deserialize(doc)?;
        let errors = validate_document(&v);
        if errors.is_empty() {
            println!("{}[doc {}]: OK", path.display(), i + 1);
        } else {
            *had_error = true;
            println!("{}[doc {}]:", path.display(), i + 1);
            for e in errors {
                println!("  - {}", e);
            }
        }
    }

    Ok(())
}

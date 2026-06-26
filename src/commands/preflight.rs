use crate::cli::PreflightArgs;
use stellar_k8s::{preflight, Error};

pub async fn run_preflight(args: PreflightArgs) -> Result<(), Error> {
    println!("=== Stellar Preflight Checks ===");
    println!();

    // Run local tool checks
    println!("--- Local Tool Checks ---");
    match preflight::run_local_preflight() {
        Ok(_) => println!("✓ All required local tools are installed"),
        Err(e) => {
            eprintln!("✗ Local tool checks failed: {}", e);
            return Err(e);
        }
    }

    // Run GitHub label checks if requested
    if let Some(github_repo) = args.github_repo {
        println!();
        println!("--- GitHub Label Checks ({}) ---", github_repo);
        match preflight::run_gh_label_preflight(Some(&github_repo)) {
            Ok(_) => println!("✓ All required GitHub labels are present"),
            Err(e) => {
                eprintln!("✗ GitHub label checks failed: {}", e);
                return Err(e);
            }
        }
    }

    println!();
    println!("=== All preflight checks passed! ===");
    Ok(())
}

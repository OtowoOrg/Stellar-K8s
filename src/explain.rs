//! Module for explaining common Stellar error codes
//! Reference: https://developers.stellar.org/docs/learn/glossary/errors

pub struct ErrorExplanation {
    pub summary: &'static str,
    pub description: &'static str,
    pub doc_url: &'static str,
}

pub fn explain_error(code: &str) {
    let explanation = match code {
        "tx_success" => Some(ErrorExplanation {
            summary: "Transaction Succeeded",
            description: "The transaction was successfully applied to the ledger.",
            doc_url: "https://developers.stellar.org/docs/learn/glossary/errors",
        }),
        "tx_failed" => Some(ErrorExplanation {
            summary: "Transaction Failed",
            description: "One or more of the operations within the transaction failed (none were applied).",
            doc_url: "https://developers.stellar.org/docs/learn/glossary/errors",
        }),
        "tx_too_early" => Some(ErrorExplanation {
            summary: "Transaction Too Early",
            description: "The ledger close time was before the transaction's minTime.",
            doc_url: "https://developers.stellar.org/docs/learn/glossary/errors",
        }),
        "tx_too_late" => Some(ErrorExplanation {
            summary: "Transaction Too Late",
            description: "The ledger close time was after the transaction's maxTime.",
            doc_url: "https://developers.stellar.org/docs/learn/glossary/errors",
        }),
        "tx_missing_operation" => Some(ErrorExplanation {
            summary: "Missing Operation",
            description: "No operation was specified in the transaction.",
            doc_url: "https://developers.stellar.org/docs/learn/glossary/errors",
        }),
        "tx_bad_seq" => Some(ErrorExplanation {
            summary: "Bad Sequence Number",
            description: "The sequence number used in the transaction does not match the source account's current sequence number.",
            doc_url: "https://developers.stellar.org/docs/learn/glossary/errors",
        }),
        "tx_bad_auth" => Some(ErrorExplanation {
            summary: "Bad Authentication",
            description: "Insufficient valid signatures or incorrect network used for the transaction.",
            doc_url: "https://developers.stellar.org/docs/learn/glossary/errors",
        }),
        "tx_insufficient_balance" => Some(ErrorExplanation {
            summary: "Insufficient Balance",
            description: "The transaction fee would cause the account to fall below its minimum reserve.",
            doc_url: "https://developers.stellar.org/docs/learn/glossary/errors",
        }),
        "tx_no_source_account" => Some(ErrorExplanation {
            summary: "No Source Account",
            description: "The source account specified for the transaction was not found.",
            doc_url: "https://developers.stellar.org/docs/learn/glossary/errors",
        }),
        "tx_insufficient_fee" => Some(ErrorExplanation {
            summary: "Insufficient Fee",
            description: "The transaction fee is too small to be accepted by the network.",
            doc_url: "https://developers.stellar.org/docs/learn/glossary/errors",
        }),
        "tx_bad_auth_extra" => Some(ErrorExplanation {
            summary: "Bad Authentication (Extra)",
            description: "Unused signatures were attached to the transaction.",
            doc_url: "https://developers.stellar.org/docs/learn/glossary/errors",
        }),
        "tx_internal_error" => Some(ErrorExplanation {
            summary: "Internal Error",
            description: "An unknown internal error occurred.",
            doc_url: "https://developers.stellar.org/docs/learn/glossary/errors",
        }),
        "op_underfunded" => Some(ErrorExplanation {
            summary: "Operation Underfunded",
            description: "The source account does not have enough funds to complete the operation.",
            doc_url: "https://developers.stellar.org/docs/learn/glossary/errors",
        }),
        "op_bad_auth" => Some(ErrorExplanation {
            summary: "Operation Bad Authentication",
            description: "Insufficient valid signatures for the specific operation.",
            doc_url: "https://developers.stellar.org/docs/learn/glossary/errors",
        }),
        "op_no_destination" => Some(ErrorExplanation {
            summary: "No Destination Account",
            description: "The destination account specified in the operation does not exist.",
            doc_url: "https://developers.stellar.org/docs/learn/glossary/errors",
        }),
        "op_not_supported" => Some(ErrorExplanation {
            summary: "Operation Not Supported",
            description: "The operation is not supported by the network or is invalid.",
            doc_url: "https://developers.stellar.org/docs/learn/glossary/errors",
        }),
        "op_too_many_subentries" => Some(ErrorExplanation {
            summary: "Too Many Subentries",
            description: "The account has reached the maximum allowed number of subentries (trustlines, offers, data entries, etc.).",
            doc_url: "https://developers.stellar.org/docs/learn/glossary/errors",
        }),
        "op_cross_self" => Some(ErrorExplanation {
            summary: "Cross Self Offer",
            description: "An offer operation would cross against another offer placed by the same account.",
            doc_url: "https://developers.stellar.org/docs/learn/glossary/errors",
        }),
        "op_line_full" => Some(ErrorExplanation {
            summary: "Trustline Full",
            description: "The destination account's trustline limits have been reached and cannot receive more of the asset.",
            doc_url: "https://developers.stellar.org/docs/learn/glossary/errors",
        }),
        _ => None,
    };

    println!("\nStellar Error Code: {}", code);
    println!("{}", "=".repeat(code.len() + 20));

    match explanation {
        Some(exp) => {
            println!("Summary:      {}", exp.summary);
            println!("Description:  {}", exp.description);
            println!("Documentation: {}", exp.doc_url);
        }
        None => {
            println!("Status:       Unknown Error Code");
            println!("Description:  This code was not found in the local dictionary. It might be a less common or newer error.");
            println!(
                "Tip:          Check the official documentation or search on the Horizon API."
            );
            println!("Documentation: https://developers.stellar.org/docs/learn/glossary/errors");
        }
    }
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_explain_error_known() {
        // Output tests are tricky in Rust without capturing stdout,
        // but we can just run it to ensure no panics.
        explain_error("tx_bad_auth");
    }

    #[test]
    fn test_explain_error_unknown() {
        explain_error("some_unknown_code");
    }
}

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DocEntry {
    pub path: String,
    pub title: String,
    pub content: String,
}

pub const SEARCH_INDEX_JSON: &str = include_str!(concat!(env!("OUT_DIR"), "/search_index.json"));

pub fn get_all_docs() -> Vec<DocEntry> {
    serde_json::from_str(SEARCH_INDEX_JSON).unwrap_or_default()
}

pub fn search(query: &str) -> Vec<(DocEntry, Vec<String>)> {
    let docs = get_all_docs();
    let query = query.to_lowercase();
    let mut results = Vec::new();

    for doc in docs {
        let mut matches = Vec::new();
        let title_lower = doc.title.to_lowercase();
        let content_lower = doc.content.to_lowercase();

        if title_lower.contains(&query) || content_lower.contains(&query) {
            // Find context for the match in content
            if let Some(pos) = content_lower.find(&query) {
                let start = pos.saturating_sub(40);
                let end = (pos + query.len() + 40).min(doc.content.len());
                let snippet = &doc.content[start..end];
                matches.push(format!("...{}...", snippet.replace('\n', " ")));
            } else if title_lower.contains(&query) {
                matches.push("Match in title".to_string());
            }
            results.push((doc, matches));
        }
    }

    results
}

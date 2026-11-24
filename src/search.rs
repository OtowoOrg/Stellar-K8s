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
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DocEntry {
    pub path: String,
    pub title: String,
    pub content: String,
}

pub const SEARCH_INDEX_JSON: &str = include_str!(concat!(env!("OUT_DIR"), "/search_index.json"));

pub fn search(query: &str) -> Vec<(DocEntry, Vec<String>)> {
    let docs: Vec<DocEntry> = serde_json::from_str(SEARCH_INDEX_JSON).unwrap_or_default();
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

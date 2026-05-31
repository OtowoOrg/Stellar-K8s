fn main() {
    let known_peers_toml = r#"KNOWN_PEERS = ["1.2.3.4:11625", "example.com:11625"]"#;
    let qs_toml = r#"[VALIDATORS]
"5.6.7.8" = "G..."
"G..." = "G..."
"#;

    let mut peers = Vec::new();

    if let Ok(value) = known_peers_toml.parse::<toml::Value>() {
        if let Some(kp_array) = value.as_array() {
            for v in kp_array {
                if let Some(s) = v.as_str() {
                    let peer = s.split(':').next().unwrap_or(s);
                    peers.push(peer.to_string());
                }
            }
        } else if let Some(kp_table) = value.get("KNOWN_PEERS").and_then(|v| v.as_array()) {
            for v in kp_table {
                if let Some(s) = v.as_str() {
                    let peer = s.split(':').next().unwrap_or(s);
                    peers.push(peer.to_string());
                }
            }
        }
    }

    if let Ok(value) = qs_toml.parse::<toml::Value>() {
        if let Some(validators) = value.get("VALIDATORS").and_then(|v| v.as_table()) {
            for key in validators.keys() {
                if !key.starts_with('G') && key.contains('.') {
                    peers.push(key.clone());
                }
            }
        }
    }

    println!("Peers: {:?}", peers);
    
    for peer in peers {
        println!("parsing: {:?}", peer.parse::<std::net::IpAddr>());
    }
}

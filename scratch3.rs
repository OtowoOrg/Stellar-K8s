fn main() {
    let known_peers_toml = r#"KNOWN_PEERS = ["1.2.3.4:11625", "example.com:11625"]"#;
    println!("{:?}", known_peers_toml.parse::<toml::Value>());
}

fn main() {
    let qs_toml = r#"[VALIDATORS]
"5.6.7.8" = "G..."
"G..." = "G..."
"#;
    println!("{:?}", qs_toml.parse::<toml::Value>());
}

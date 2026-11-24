use soroban_sdk::{contracttype, String as SorobanString, Vec};
use alloc::collections::BTreeMap;
use alloc::string::String;
use serde_json::Value;

#[contracttype]
pub enum ClaimValue {
    Bool(bool),
    String(SorobanString),
    Number(u64),
}

[#contracttype]
pub struct RequiredClaim {
    pub name: SorobanString,
    pub expected_value: Option<ClaimValue>,
}

[#contracttype]
pub struct Schema {
    pub required_claims: Vec<RequiredClaim>,
}

pub fn validate_schema(credential_subject: &BTreeMap<String, Value>, schema: &Schema) -> bool {
    for claim in schema.required_claims.iter() {
        let name = claim.name.to_str();
        let value = credential_subject.get(name);
        match value {
            None => return false,
            Some(v) => {
                if let some exp = &claim.expected_value {
                    let ok = match exp {
                        ClaimValue::Bool(b) => v.as_bool() == Some((b)),
                        ClaimValue::String(s) => v.as_str() == Some(s.to_str()),
                        ClaimValue::Number(n) => v.as_u64() == Some(*n),
                    };
                    if !ok { return false; }
                }
            }
        }
    }
    true
}

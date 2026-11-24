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
//! Network-aware defaults for the generated `stellar-core.cfg` (#1551).
//!
//! # Why this exists
//!
//! `stellar-core` refuses to start without a network passphrase and a quorum
//! set, but the operator used to emit neither: a `StellarNode` with
//! `spec.network: Testnet` and no `spec.validatorConfig.quorumSet` produced a
//! `stellar-core.cfg` that was missing `NETWORK_PASSPHRASE`, `KNOWN_PEERS` and
//! the validator list. The user had to hand-craft the SDF quorum set, and a
//! single typo in the TOML produced a validator that silently never reached
//! quorum.
//!
//! This module turns `spec.network` into a complete, ready-to-run
//! configuration:
//!
//! | Network | Generated content |
//! |---------|-------------------|
//! | `Testnet` | SDF testnet passphrase, peers, home domain and validators |
//! | `Mainnet` | public passphrase, peers, home domains and validators |
//! | `Futurenet` | public passphrase (the network has no published quorum) |
//! | `Custom(..)` | standalone profile for a private network |
//!
//! # How user configuration wins
//!
//! Generation never overwrites what the user wrote. The document is split in
//! two independent groups:
//!
//! * `NETWORK_PASSPHRASE` is a *root key* and is only generated when the user
//!   did not declare it.
//! * The quorum group (`KNOWN_PEERS`, `QUORUM_SET`, `HOME_DOMAINS`,
//!   `VALIDATORS`) is *atomic*: `stellar-core` rejects a configuration that
//!   mixes a `[QUORUM_SET]` with `[[HOME_DOMAINS]]`/`[[VALIDATORS]]`, so the
//!   whole group is generated or none of it is. As soon as the user declares
//!   any part of the quorum, the operator keeps the user's quorum and only
//!   fills in the parts that are independent of it.
//!
//! # Where the generated content goes
//!
//! Root keys are rendered *before* the user content, table sections *after*
//! it. TOML scopes every key written after a table header into that table, so
//! a generated `[[VALIDATORS]]` block placed in front of the user content
//! would capture the user's root keys (`NODE_SEED`, ...) and silently disable
//! them. See [`crate::controller::config_scope`] for the structural checks
//! that prove the assembled document is well formed.

use std::collections::BTreeSet;

use crate::crd::StellarNetwork;

/// The `stellar-core` peer port used when an address carries no port.
pub const DEFAULT_PEER_PORT: u16 = 11625;

/// Root key holding the network passphrase.
pub const NETWORK_PASSPHRASE_KEY: &str = "NETWORK_PASSPHRASE";

/// Root key holding the peer list.
pub const KNOWN_PEERS_KEY: &str = "KNOWN_PEERS";

/// A home domain of the generated quorum.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HomeDomain {
    /// The home domain, e.g. `testnet.stellar.org`.
    pub domain: &'static str,
    /// The quorum quality, e.g. `HIGH`.
    pub quality: &'static str,
}

/// A validator of the generated quorum.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Validator {
    /// The validator name.
    pub name: &'static str,
    /// The home domain the validator belongs to.
    pub home_domain: &'static str,
    /// The validator public key.
    pub public_key: &'static str,
    /// The peer address, with or without a port.
    pub address: &'static str,
    /// The history archive command template.
    pub history: &'static str,
}

/// The network profile the operator synthesizes defaults for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkProfile {
    /// The public main network.
    Mainnet,
    /// The SDF test network.
    Testnet,
    /// The SDF future network.
    Futurenet,
    /// A private network running a single validator.
    Standalone,
}

impl NetworkProfile {
    /// The profile that matches `network`.
    ///
    /// A custom network has no published quorum, so it maps to the standalone
    /// profile: one validator, threshold 100%, no external peers.
    pub fn for_network(network: &StellarNetwork) -> Self {
        match network {
            StellarNetwork::Mainnet => Self::Mainnet,
            StellarNetwork::Testnet => Self::Testnet,
            StellarNetwork::Futurenet => Self::Futurenet,
            StellarNetwork::Custom(_) => Self::Standalone,
        }
    }

    /// Stable identifier of the profile, used in log messages and notes.
    pub fn id(&self) -> &'static str {
        match self {
            Self::Mainnet => "mainnet",
            Self::Testnet => "testnet",
            Self::Futurenet => "futurenet",
            Self::Standalone => "standalone",
        }
    }

    /// The home domains of the profile.
    pub fn home_domains(&self) -> &'static [HomeDomain] {
        match self {
            Self::Mainnet => MAINNET_HOME_DOMAINS,
            Self::Testnet => TESTNET_HOME_DOMAINS,
            Self::Futurenet => &[],
            Self::Standalone => &[],
        }
    }

    /// The validators of the profile.
    pub fn validators(&self) -> &'static [Validator] {
        match self {
            Self::Mainnet => MAINNET_VALIDATORS,
            Self::Testnet => TESTNET_VALIDATORS,
            Self::Futurenet => &[],
            Self::Standalone => &[],
        }
    }

    /// True for the standalone profile of a private network.
    pub fn is_standalone(&self) -> bool {
        matches!(self, Self::Standalone)
    }

    /// The peer list derived from the validators of the profile.
    ///
    /// Addresses without a port get the default `stellar-core` peer port, so
    /// the generated `KNOWN_PEERS` entry is always a `host:port` endpoint.
    pub fn known_peers(&self) -> Vec<String> {
        self.validators()
            .iter()
            .map(|validator| peer_address(validator.address))
            .collect()
    }
}

/// Everything the generator needs to know about the node it configures.
#[derive(Clone, Copy, Debug)]
pub struct GenerationRequest<'a> {
    /// The target network of the node.
    pub network: &'a StellarNetwork,
    /// The passphrase of a custom network, when it is known to the operator.
    pub custom_passphrase: Option<&'a str>,
    /// True when the passphrase is stored in a Secret.
    ///
    /// A secret-backed passphrase is never rendered into the ConfigMap.
    pub passphrase_in_secret: bool,
    /// The user supplied `stellar-core.cfg` content.
    pub user_config: &'a str,
}

/// The generated parts of a `stellar-core.cfg`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GeneratedConfig {
    /// Root level assignments, rendered before the user content.
    pub root_keys: Vec<String>,
    /// Table sections, rendered after the user content.
    pub table_sections: Vec<String>,
    /// Notes describing what was generated and what was skipped.
    pub notes: Vec<String>,
}

impl GeneratedConfig {
    /// True when nothing was generated.
    pub fn is_empty(&self) -> bool {
        self.root_keys.is_empty() && self.table_sections.is_empty()
    }

    /// Render the root keys, newline terminated.
    pub fn render_root_keys(&self) -> String {
        render_lines(&self.root_keys)
    }

    /// Render the table sections, newline terminated.
    pub fn render_table_sections(&self) -> String {
        render_lines(&self.table_sections)
    }

    /// A one line summary suitable for logs.
    pub fn summary(&self) -> String {
        format!(
            "root_keys={} table_sections={} notes={}",
            self.root_keys.len(),
            self.table_sections.len(),
            self.notes.len()
        )
    }
}

/// Generate the network defaults for a `stellar-core.cfg`.
///
/// The result is split into root keys and table sections so the caller can
/// place them on the correct side of the user content; see the module
/// documentation for the reasoning.
pub fn generate(request: &GenerationRequest<'_>) -> GeneratedConfig {
    let mut generated = GeneratedConfig::default();
    let profile = NetworkProfile::for_network(request.network);
    generated.notes.push(format!("profile={}", profile.id()));

    let declarations = match UserDeclarations::parse(request.user_config) {
        Ok(declarations) => declarations,
        Err(message) => {
            generated.notes.push(format!(
                "user configuration is not valid TOML ({message}); no network defaults were generated"
            ));
            return generated;
        }
    };

    // The passphrase is independent of the quorum: the user value always
    // wins, but the rest of the defaults are still generated.
    if declarations.declares_root_key(NETWORK_PASSPHRASE_KEY) {
        generated
            .notes
            .push("NETWORK_PASSPHRASE is provided by the user".to_string());
    } else if let Some(passphrase) = passphrase_for(request) {
        generated
            .root_keys
            .push(format!("{NETWORK_PASSPHRASE_KEY}={}", quote(&passphrase)));
    } else {
        generated.notes.push(
            "no NETWORK_PASSPHRASE could be generated; provide one in spec.validatorConfig"
                .to_string(),
        );
    }

    // The quorum is atomic: stellar-core rejects a configuration that mixes
    // [QUORUM_SET] with [[HOME_DOMAINS]]/[[VALIDATORS]].
    if declarations.declares_quorum() {
        generated.notes.push(
            "the user configuration declares a quorum; the generated quorum was skipped"
                .to_string(),
        );
        return generated;
    }

    let peers = profile.known_peers();
    if !peers.is_empty() {
        generated
            .root_keys
            .push(format!("{KNOWN_PEERS_KEY}={}", toml_array(&peers)));
    }

    match profile {
        NetworkProfile::Standalone => {
            generated.root_keys.push("RUN_STANDALONE=true".to_string());
            generated.root_keys.push("FAILURE_SAFETY=0".to_string());
            generated.root_keys.push("UNSAFE_QUORUM=true".to_string());
            generated
                .table_sections
                .push("[QUORUM_SET]\nTHRESHOLD_PERCENT=100\nVALIDATORS=[\"$self\"]".to_string());
        }
        NetworkProfile::Mainnet | NetworkProfile::Testnet => {
            for home_domain in profile.home_domains() {
                generated.table_sections.push(format!(
                    "[[HOME_DOMAINS]]\nHOME_DOMAIN={}\nQUALITY={}\n",
                    quote(home_domain.domain),
                    quote(home_domain.quality)
                ));
            }
            for validator in profile.validators() {
                generated.table_sections.push(format!(
                    "[[VALIDATORS]]\nNAME={}\nHOME_DOMAIN={}\nPUBLIC_KEY={}\nADDRESS={}\nHISTORY={}\n",
                    quote(validator.name),
                    quote(validator.home_domain),
                    quote(validator.public_key),
                    quote(validator.address),
                    quote(validator.history)
                ));
            }
        }
        NetworkProfile::Futurenet => {
            generated.notes.push(
                "the future network publishes no validator list; only the passphrase was generated"
                    .to_string(),
            );
        }
    }

    generated
}

/// The network passphrase to render, if it may be written to the ConfigMap.
///
/// The passphrase of the public networks is a published constant. A custom
/// network passphrase may be secret, in which case it is never rendered.
fn passphrase_for(request: &GenerationRequest<'_>) -> Option<String> {
    match request.network {
        StellarNetwork::Mainnet | StellarNetwork::Testnet | StellarNetwork::Futurenet => {
            Some(request.network.passphrase(&None).to_string())
        }
        StellarNetwork::Custom(_) => {
            if request.passphrase_in_secret {
                return None;
            }
            request
                .custom_passphrase
                .map(str::trim)
                .filter(|passphrase| !passphrase.is_empty())
                .map(str::to_string)
        }
    }
}

/// The root keys and tables a user configuration declares.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct UserDeclarations {
    root_keys: BTreeSet<String>,
    tables: BTreeSet<String>,
}

impl UserDeclarations {
    /// Parse the user configuration and collect the keys it declares.
    ///
    /// Returns `Err` when the content is not valid TOML; the caller then skips
    /// generation entirely instead of writing defaults into a document that
    /// cannot be parsed.
    fn parse(user_config: &str) -> Result<Self, String> {
        let trimmed = user_config.trim();
        if trimmed.is_empty() {
            return Ok(Self::default());
        }

        let value: toml::Value = trimmed.parse().map_err(|error| format!("{error}"))?;

        let root = match value {
            toml::Value::Table(root) => root,
            _ => return Err("the configuration is not a TOML table".to_string()),
        };

        let mut declarations = Self::default();
        for (key, child) in &root {
            if is_table_value(child) {
                declarations.tables.insert(key.clone());
            } else {
                declarations.root_keys.insert(key.clone());
            }
        }

        Ok(declarations)
    }

    /// True when the user declared `key` at the document root.
    fn declares_root_key(&self, key: &str) -> bool {
        self.root_keys.contains(key)
    }

    /// True when the user declared the `table` section.
    fn declares_table(&self, table: &str) -> bool {
        self.tables.contains(table)
    }

    /// True when the user declared any part of the quorum.
    fn declares_quorum(&self) -> bool {
        self.declares_root_key(KNOWN_PEERS_KEY)
            || self.declares_table("QUORUM_SET")
            || self.declares_table("HOME_DOMAINS")
            || self.declares_table("VALIDATORS")
    }
}

/// True when a TOML value describes a table (or an array of tables).
fn is_table_value(value: &toml::Value) -> bool {
    match value {
        toml::Value::Table(_) => true,
        toml::Value::Array(items) => items
            .first()
            .is_some_and(|item| matches!(item, toml::Value::Table(_))),
        _ => false,
    }
}

/// Render a peer address, appending the default port when it carries none.
fn peer_address(address: &str) -> String {
    let has_port = address
        .rsplit(':')
        .next()
        .is_some_and(|port| !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()));

    if has_port {
        address.to_string()
    } else {
        format!("{address}:{DEFAULT_PEER_PORT}")
    }
}

/// Render a TOML string literal.
fn quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

/// Render a TOML array of strings.
fn toml_array(values: &[String]) -> String {
    let rendered: Vec<String> = values.iter().map(|value| quote(value)).collect();
    format!("[{}]", rendered.join(", "))
}

/// Join rendered lines into a newline terminated block.
fn render_lines(lines: &[String]) -> String {
    if lines.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    for line in lines {
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// The SDF testnet home domain.
const TESTNET_HOME_DOMAINS: &[HomeDomain] = &[HomeDomain {
    domain: "testnet.stellar.org",
    quality: "HIGH",
}];

/// The SDF testnet validators.
const TESTNET_VALIDATORS: &[Validator] = &[
    Validator {
        name: "sdftest1",
        home_domain: "testnet.stellar.org",
        public_key: "GDKXE2OZMJIPOSLNA6N6F2BVCI3O777I2OOC4BV7VOYUEHYX7RTRYA7Y",
        address: "core-testnet1.stellar.org",
        history: "curl -sf http://history.stellar.org/prd/core-testnet/core_testnet_001/{0} -o {1}",
    },
    Validator {
        name: "sdftest2",
        home_domain: "testnet.stellar.org",
        public_key: "GCUCJTIYXSOXKBSNFGNFWW5MUQ54HKRPGJUTQFJ5RQXZXNOLNXYDHRAP",
        address: "core-testnet2.stellar.org",
        history: "curl -sf http://history.stellar.org/prd/core-testnet/core_testnet_002/{0} -o {1}",
    },
    Validator {
        name: "sdftest3",
        home_domain: "testnet.stellar.org",
        public_key: "GC2V2EFSXN6SQTWVYA5EPJPBWWIMSD2XQNKUOHGEKB535AQE2I6IXV2Z",
        address: "core-testnet3.stellar.org",
        history: "curl -sf http://history.stellar.org/prd/core-testnet/core_testnet_003/{0} -o {1}",
    },
];

/// The home domains of the published mainnet sample.
const MAINNET_HOME_DOMAINS: &[HomeDomain] = &[
    HomeDomain {
        domain: "publicnode.org",
        quality: "HIGH",
    },
    HomeDomain {
        domain: "lobstr.co",
        quality: "HIGH",
    },
    HomeDomain {
        domain: "www.franklintempleton.com",
        quality: "HIGH",
    },
    HomeDomain {
        domain: "stellar.withobsrvr.com",
        quality: "HIGH",
    },
    HomeDomain {
        domain: "stellar.creit.tech",
        quality: "HIGH",
    },
    HomeDomain {
        domain: "www.stellar.org",
        quality: "HIGH",
    },
    HomeDomain {
        domain: "stellar.blockdaemon.com",
        quality: "HIGH",
    },
    HomeDomain {
        domain: "www.ylds.com",
        quality: "HIGH",
    },
    HomeDomain {
        domain: "moneygram.com",
        quality: "HIGH",
    },
    HomeDomain {
        domain: "stellar.validator.range.org",
        quality: "HIGH",
    },
];

/// The validators of the published mainnet sample.
const MAINNET_VALIDATORS: &[Validator] = &[
    Validator {
        name: "Boötes",
        home_domain: "publicnode.org",
        public_key: "GCVJ4Z6TI6Z2SOGENSPXDQ2U4RKH3CNQKYUHNSSPYFPNWTLGS6EBH7I2",
        address: "bootes.publicnode.org:11625",
        history: "curl -sf https://bootes-history.publicnode.org/{0} -o {1}",
    },
    Validator {
        name: "Lyra by BP Ventures",
        home_domain: "publicnode.org",
        public_key: "GCIXVKNFPKWVMKJKVK2V4NK7D4TC6W3BUMXSIJ365QUAXWBRPPJXIR2Z",
        address: "lyra.publicnode.org:11625",
        history: "curl -sf https://lyra-history.publicnode.org/{0} -o {1}",
    },
    Validator {
        name: "Hercules by OG Technologies",
        home_domain: "publicnode.org",
        public_key: "GBLJNN3AVZZPG2FYAYTYQKECNWTQYYUUY2KVFN2OUKZKBULXIXBZ4FCT",
        address: "hercules.publicnode.org:11625",
        history: "curl -sf https://hercules-history.publicnode.org/{0} -o {1}",
    },
    Validator {
        name: "LOBSTR 1 (Europe)",
        home_domain: "lobstr.co",
        public_key: "GCFONE23AB7Y6C5YZOMKUKGETPIAJA4QOYLS5VNS4JHBGKRZCPYHDLW7",
        address: "v1.stellar.lobstr.co:11625",
        history: "curl -sf https://archive.v1.stellar.lobstr.co/{0} -o {1}",
    },
    Validator {
        name: "LOBSTR 2 (Europe)",
        home_domain: "lobstr.co",
        public_key: "GCB2VSADESRV2DDTIVTFLBDI562K6KE3KMKILBHUHUWFXCUBHGQDI7VL",
        address: "v2.stellar.lobstr.co:11625",
        history: "curl -sf https://archive.v2.stellar.lobstr.co/{0} -o {1}",
    },
    Validator {
        name: "LOBSTR 5 (India)",
        home_domain: "lobstr.co",
        public_key: "GA5STBMV6QDXFDGD62MEHLLHZTPDI77U3PFOD2SELU5RJDHQWBR5NNK7",
        address: "v5.stellar.lobstr.co:11625",
        history: "curl -sf https://archive.v5.stellar.lobstr.co/{0} -o {1}",
    },
    Validator {
        name: "FT SCV 2",
        home_domain: "www.franklintempleton.com",
        public_key: "GCMSM2VFZGRPTZKPH5OABHGH4F3AVS6XTNJXDGCZ3MKCOSUBH3FL6DOB",
        address: "stellar2.franklintempleton.com:11625",
        history:
            "curl -sf https://stellar-history-usc.franklintempleton.com/azuscshf401/{0} -o {1}",
    },
    Validator {
        name: "FT SCV 3",
        home_domain: "www.franklintempleton.com",
        public_key: "GA7DV63PBUUWNUFAF4GAZVXU2OZMYRATDLKTC7VTCG7AU4XUPN5VRX4A",
        address: "stellar3.franklintempleton.com:11625",
        history:
            "curl -sf https://stellar-history-ins.franklintempleton.com/azinsshf401/{0} -o {1}",
    },
    Validator {
        name: "FT SCV 1",
        home_domain: "www.franklintempleton.com",
        public_key: "GARYGQ5F2IJEBCZJCBNPWNWVDOFK7IBOHLJKKSG2TMHDQKEEC6P4PE4V",
        address: "stellar1.franklintempleton.com:11625",
        history:
            "curl -sf https://stellar-history-usw.franklintempleton.com/azuswshf401/{0} -o {1}",
    },
    Validator {
        name: "OBSRVR Validator 1",
        home_domain: "stellar.withobsrvr.com",
        public_key: "GDRCZ4IPJR7V3HK4GR45CRTE72SDAOZUF2TDBQ5E5IGWC4KM5TSKU2LS",
        address: "core-live-1.nodeswithobsrvr.co:11625",
        history: "curl -sf https://history-1.nodeswithobsrvr.co/obsrvr-core-1/{0} -o {1}",
    },
    Validator {
        name: "OBSRVR Validator 2",
        home_domain: "stellar.withobsrvr.com",
        public_key: "GA2PU4UGMLSFUXGZATHPTDXXX7FOHBAQC57RSJCQUN72WFKTD6CEPQSF",
        address: "core-live-2.nodeswithobsrvr.co:11625",
        history: "curl -sf https://history-2.nodeswithobsrvr.co/obsrvr-core-2/{0} -o {1}",
    },
    Validator {
        name: "OBSRVR Validator 3",
        home_domain: "stellar.withobsrvr.com",
        public_key: "GACM6GIRMLXBBZIYJXBDTAEYZ2GJP3JJP5G5K4WDPBS6QFHPJNK6S2FB",
        address: "core-live-3.nodeswithobsrvr.co:11625",
        history: "curl -sf https://history-3.nodeswithobsrvr.co/obsrvr-core-3/{0} -o {1}",
    },
    Validator {
        name: "Creit Alpha",
        home_domain: "stellar.creit.tech",
        public_key: "GBPLJDBFZO2H7QQH7YFCH3HFT6EMC42Z2DNJ2QFROCKETAPY54V4DCZD",
        address: "alpha.validator.stellar.creit.tech:11625",
        history: "curl -sf https://alpha-history.validator.stellar.creit.tech/{0} -o {1}",
    },
    Validator {
        name: "Creit Beta",
        home_domain: "stellar.creit.tech",
        public_key: "GDDANSYOYSY5EPSFHBRPCLX6XMHPPLIMHVIDXG6IPQLVVLRI2BN4HMH3",
        address: "beta.validator.stellar.creit.tech:11625",
        history: "curl -sf https://beta-history.validator.stellar.creit.tech/{0} -o {1}",
    },
    Validator {
        name: "Creit Gamma",
        home_domain: "stellar.creit.tech",
        public_key: "GBF7QOLFPTHUEDUPTT4ZTULDTA3QXDIO75JHKJN2IYD7YGQLYUTR75BT",
        address: "gamma.validator.stellar.creit.tech:11625",
        history: "curl -sf https://gamma-history.validator.stellar.creit.tech/{0} -o {1}",
    },
    Validator {
        name: "SDF 2",
        home_domain: "www.stellar.org",
        public_key: "GCM6QMP3DLRPTAZW2UZPCPX2LF3SXWXKPMP3GKFZBDSF3QZGV2G5QSTK",
        address: "core-live-b.stellar.org:11625",
        history: "curl -sf http://history.stellar.org/prd/core-live/core_live_002/{0} -o {1}",
    },
    Validator {
        name: "SDF 1",
        home_domain: "www.stellar.org",
        public_key: "GCGB2S2KGYARPVIA37HYZXVRM2YZUEXA6S33ZU5BUDC6THSB62LZSTYH",
        address: "core-live-a.stellar.org:11625",
        history: "curl -sf http://history.stellar.org/prd/core-live/core_live_001/{0} -o {1}",
    },
    Validator {
        name: "SDF 3",
        home_domain: "www.stellar.org",
        public_key: "GABMKJM6I25XI4K7U6XWMULOUQIQ27BCTMLS6BYYSOWKTBUXVRJSXHYQ",
        address: "core-live-c.stellar.org:11625",
        history: "curl -sf http://history.stellar.org/prd/core-live/core_live_003/{0} -o {1}",
    },
    Validator {
        name: "Blockdaemon Validator 3",
        home_domain: "stellar.blockdaemon.com",
        public_key: "GAYXZ4PZ7P6QOX7EBHPIZXNWY4KCOBYWJCA4WKWRKC7XIUS3UJPT6EZ4",
        address: "stellar-full-validator3.bdnodes.net:11625",
        history: "curl -sf https://stellar-full-history3.bdnodes.net/{0} -o {1}",
    },
    Validator {
        name: "Blockdaemon Validator 2",
        home_domain: "stellar.blockdaemon.com",
        public_key: "GAVXB7SBJRYHSG6KSQHY74N7JAFRL4PFVZCNWW2ARI6ZEKNBJSMSKW7C",
        address: "stellar-full-validator2.bdnodes.net:11625",
        history: "curl -sf https://stellar-full-history2.bdnodes.net/{0} -o {1}",
    },
    Validator {
        name: "Blockdaemon Validator 1",
        home_domain: "stellar.blockdaemon.com",
        public_key: "GAAV2GCVFLNN522ORUYFV33E76VPC22E72S75AQ6MBR5V45Z5DWVPWEU",
        address: "stellar-full-validator1.bdnodes.net:11625",
        history: "curl -sf https://stellar-full-history1.bdnodes.net/{0} -o {1}",
    },
    Validator {
        name: "YLDS Central",
        home_domain: "www.ylds.com",
        public_key: "GDSQ7BEI6NCDHDOPABHTFDDOXCUVN4Z5MGYYMBVYW3G5CNHQMQM2RPE5",
        address: "stellar-central.ylds.com:11625",
        history: "curl -sf http://stellar-history.ylds.com/validator-us-central1-0/{0} -o {1}",
    },
    Validator {
        name: "YLDS East",
        home_domain: "www.ylds.com",
        public_key: "GB3BMQGHJDJ3H2HJATFSXMHMZVXXJT5QCOQDGO3JHH6EISLY3J44CMZK",
        address: "stellar-east.ylds.com:11625",
        history: "curl -sf http://stellar-history.ylds.com/validator-us-east1-0/{0} -o {1}",
    },
    Validator {
        name: "YLDS West",
        home_domain: "www.ylds.com",
        public_key: "GBRMVYACW2T5MVFKZPAJF6PE5KLLRMLYPSPTRJ4SHAIDRSVXYMQNGWB4",
        address: "stellar-west.ylds.com:11625",
        history: "curl -sf http://stellar-history.ylds.com/validator-us-west1-0/{0} -o {1}",
    },
    Validator {
        name: "MoneyGram Validator 1",
        home_domain: "moneygram.com",
        public_key: "GAVEOQE4O53I7OLY43HIUVKLLMALHSCNPVBKXB46FBDVRTDXRBIPHTFP",
        address: "v1-stellar.moneygram.com:11625",
        history: "curl -sf https://stellar-history.moneygram.com/v1/{0} -o {1}",
    },
    Validator {
        name: "MoneyGram Validator 2",
        home_domain: "moneygram.com",
        public_key: "GBPBEOYMDXXBP2IG7LDM64YMLVTU3G567HWOKMMDOOCFFDFKAFVRDYM7",
        address: "v2-stellar.moneygram.com:11625",
        history: "curl -sf https://stellar-history.moneygram.com/v2/{0} -o {1}",
    },
    Validator {
        name: "MoneyGram Validator 3",
        home_domain: "moneygram.com",
        public_key: "GAW3OEQRVXR3XFVLTUPZMLDQHKUSXECDCJYCR3MEAATC2QJ2ELLPXORD",
        address: "v3-stellar.moneygram.com:11625",
        history: "curl -sf https://stellar-history.moneygram.com/v3/{0} -o {1}",
    },
    Validator {
        name: "Range 1",
        home_domain: "stellar.validator.range.org",
        public_key: "GA3NWOLWTIFBYUNDXNE3E7TZSYNQQCKLHZQZFV7KSK7XPDU2ESVGISKT",
        address: "core-1.stellar.range.org:11625",
        history: "curl -sf https://history-1.stellar.range.org/{0} -o {1}",
    },
    Validator {
        name: "Range 2",
        home_domain: "stellar.validator.range.org",
        public_key: "GA5MQHKWKZF24MWTDXVJE6UYI5QVBYFIVUP24RWYAXQIT5AIMZ6TMUXC",
        address: "core-2.stellar.range.org:11625",
        history: "curl -sf https://history-2.stellar.range.org/{0} -o {1}",
    },
    Validator {
        name: "Range 3",
        home_domain: "stellar.validator.range.org",
        public_key: "GCSXADVBPPKIDUW6FDJZR25IX4ZA7B7YH63U2AZK7LVHSMLFGCKDBTBI",
        address: "core-3.stellar.range.org:11625",
        history: "curl -sf https://history-3.stellar.range.org/{0} -o {1}",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The SDF published testnet configuration, used as the reference.
    const SDF_TESTNET_CFG: &str =
        include_str!("../../tests/fixtures/stellar_core_cfg/sdf/stellar-core_testnet.cfg");

    /// The SDF published mainnet sample, used as the reference.
    const SDF_MAINNET_CFG: &str =
        include_str!("../../tests/fixtures/stellar_core_cfg/sdf/stellar-core_mainnet.cfg");

    fn request<'a>(network: &'a StellarNetwork, user_config: &'a str) -> GenerationRequest<'a> {
        GenerationRequest {
            network,
            custom_passphrase: None,
            passphrase_in_secret: false,
            user_config,
        }
    }

    /// Parse a generated configuration into a TOML value.
    fn parse(cfg: &str) -> toml::Value {
        cfg.parse::<toml::Value>()
            .expect("generated configuration must be valid TOML")
    }

    #[test]
    fn testnet_profile_matches_the_published_sdf_configuration() {
        let network = StellarNetwork::Testnet;
        let generated = generate(&request(&network, ""));

        let cfg = parse(&format!(
            "{}{}",
            generated.render_root_keys(),
            generated.render_table_sections()
        ));
        let reference = parse(SDF_TESTNET_CFG);

        assert_eq!(
            cfg.get("NETWORK_PASSPHRASE").and_then(toml::Value::as_str),
            reference
                .get("NETWORK_PASSPHRASE")
                .and_then(toml::Value::as_str)
        );

        let generated_validators = cfg
            .get("VALIDATORS")
            .and_then(toml::Value::as_array)
            .expect("generated validators");
        let reference_validators = reference
            .get("VALIDATORS")
            .and_then(toml::Value::as_array)
            .expect("reference validators");

        assert_eq!(generated_validators.len(), reference_validators.len());
        for (generated, reference) in generated_validators.iter().zip(reference_validators) {
            assert_eq!(
                generated.get("NAME").and_then(toml::Value::as_str),
                reference.get("NAME").and_then(toml::Value::as_str)
            );
            assert_eq!(
                generated.get("PUBLIC_KEY").and_then(toml::Value::as_str),
                reference.get("PUBLIC_KEY").and_then(toml::Value::as_str)
            );
            assert_eq!(
                generated.get("ADDRESS").and_then(toml::Value::as_str),
                reference.get("ADDRESS").and_then(toml::Value::as_str)
            );
            assert_eq!(
                generated.get("HISTORY").and_then(toml::Value::as_str),
                reference.get("HISTORY").and_then(toml::Value::as_str)
            );
        }

        let generated_domains = cfg
            .get("HOME_DOMAINS")
            .and_then(toml::Value::as_array)
            .expect("generated home domains");
        let reference_domains = reference
            .get("HOME_DOMAINS")
            .and_then(toml::Value::as_array)
            .expect("reference home domains");

        assert_eq!(generated_domains.len(), reference_domains.len());
        for (generated, reference) in generated_domains.iter().zip(reference_domains) {
            assert_eq!(
                generated.get("HOME_DOMAIN").and_then(toml::Value::as_str),
                reference.get("HOME_DOMAIN").and_then(toml::Value::as_str)
            );
            assert_eq!(
                generated.get("QUALITY").and_then(toml::Value::as_str),
                reference.get("QUALITY").and_then(toml::Value::as_str)
            );
        }
    }

    #[test]
    fn mainnet_profile_matches_the_published_sdf_sample() {
        let network = StellarNetwork::Mainnet;
        let generated = generate(&request(&network, ""));

        let cfg = parse(&format!(
            "{}{}",
            generated.render_root_keys(),
            generated.render_table_sections()
        ));
        let reference = parse(SDF_MAINNET_CFG);

        assert_eq!(
            cfg.get("NETWORK_PASSPHRASE").and_then(toml::Value::as_str),
            reference
                .get("NETWORK_PASSPHRASE")
                .and_then(toml::Value::as_str)
        );

        let generated_validators = cfg
            .get("VALIDATORS")
            .and_then(toml::Value::as_array)
            .expect("generated validators");
        let reference_validators = reference
            .get("VALIDATORS")
            .and_then(toml::Value::as_array)
            .expect("reference validators");

        assert_eq!(generated_validators.len(), reference_validators.len());
        for (generated, reference) in generated_validators.iter().zip(reference_validators) {
            assert_eq!(
                generated.get("PUBLIC_KEY").and_then(toml::Value::as_str),
                reference.get("PUBLIC_KEY").and_then(toml::Value::as_str)
            );
            assert_eq!(
                generated.get("ADDRESS").and_then(toml::Value::as_str),
                reference.get("ADDRESS").and_then(toml::Value::as_str)
            );
        }
    }

    #[test]
    fn known_peers_are_derived_from_the_validator_addresses() {
        let network = StellarNetwork::Testnet;
        let generated = generate(&request(&network, ""));

        let peers = generated
            .root_keys
            .iter()
            .find(|line| line.starts_with("KNOWN_PEERS="))
            .expect("KNOWN_PEERS must be generated");
        assert!(peers.contains("core-testnet1.stellar.org:11625"));
        assert!(peers.contains("core-testnet2.stellar.org:11625"));
        assert!(peers.contains("core-testnet3.stellar.org:11625"));
    }

    #[test]
    fn user_quorum_suppresses_the_generated_quorum() {
        let network = StellarNetwork::Testnet;
        let user_config = "[QUORUM_SET]\nTHRESHOLD_PERCENT=67\nVALIDATORS=[\"GCEZW7\"]\n";
        let generated = generate(&request(&network, user_config));

        assert!(!generated
            .root_keys
            .iter()
            .any(|line| line.starts_with("KNOWN_PEERS=")));
        assert!(generated.table_sections.is_empty());
        // The passphrase is independent of the quorum and is still generated.
        assert!(generated
            .root_keys
            .iter()
            .any(|line| line.starts_with("NETWORK_PASSPHRASE=")));
    }

    #[test]
    fn user_known_peers_suppress_the_generated_quorum() {
        let network = StellarNetwork::Testnet;
        let user_config = "KNOWN_PEERS=[\"10.0.0.11:11625\"]\n";
        let generated = generate(&request(&network, user_config));

        assert!(generated.table_sections.is_empty());
        assert!(!generated
            .root_keys
            .iter()
            .any(|line| line.starts_with("KNOWN_PEERS=")));
    }

    #[test]
    fn user_passphrase_wins_over_the_generated_one() {
        let network = StellarNetwork::Testnet;
        let user_config = "NETWORK_PASSPHRASE=\"My Own Network\"\n";
        let generated = generate(&request(&network, user_config));

        assert!(!generated
            .root_keys
            .iter()
            .any(|line| line.starts_with("NETWORK_PASSPHRASE=")));
        // The quorum is still generated: the passphrase is not part of it.
        assert!(!generated.table_sections.is_empty());
    }

    #[test]
    fn custom_network_gets_the_standalone_profile() {
        let network = StellarNetwork::Custom("private".to_string());
        let mut request = request(&network, "");
        request.custom_passphrase = Some("Private Network ; 2026");
        let generated = generate(&request);

        assert!(generated
            .root_keys
            .iter()
            .any(|line| line == "RUN_STANDALONE=true"));
        assert!(generated
            .root_keys
            .iter()
            .any(|line| line == "UNSAFE_QUORUM=true"));
        assert!(generated
            .root_keys
            .iter()
            .any(|line| line == "FAILURE_SAFETY=0"));
        assert!(generated
            .root_keys
            .iter()
            .any(|line| line == "NETWORK_PASSPHRASE=\"Private Network ; 2026\""));
        assert_eq!(generated.table_sections.len(), 1);
        assert!(generated.table_sections[0].contains("THRESHOLD_PERCENT=100"));
        assert!(generated.table_sections[0].contains("VALIDATORS=[\"$self\"]"));
    }

    #[test]
    fn secret_backed_custom_passphrase_is_never_rendered() {
        let network = StellarNetwork::Custom("private".to_string());
        let mut request = request(&network, "");
        request.custom_passphrase = Some("Private Network ; 2026");
        request.passphrase_in_secret = true;
        let generated = generate(&request);

        assert!(!generated
            .root_keys
            .iter()
            .any(|line| line.starts_with("NETWORK_PASSPHRASE=")));
        assert!(generated
            .notes
            .iter()
            .any(|note| note.contains("NETWORK_PASSPHRASE")));
    }

    #[test]
    fn futurenet_only_generates_the_passphrase() {
        let network = StellarNetwork::Futurenet;
        let generated = generate(&request(&network, ""));

        assert_eq!(generated.root_keys.len(), 1);
        assert!(generated.table_sections.is_empty());
        assert!(generated.root_keys[0].contains("Test SDF Future Network"));
    }

    #[test]
    fn invalid_user_configuration_skips_generation() {
        let network = StellarNetwork::Testnet;
        let generated = generate(&request(&network, "this is = = not toml"));

        assert!(generated.is_empty());
        assert!(generated
            .notes
            .iter()
            .any(|note| note.contains("not valid TOML")));
    }

    #[test]
    fn generated_configuration_is_valid_toml() {
        for network in [
            StellarNetwork::Testnet,
            StellarNetwork::Mainnet,
            StellarNetwork::Futurenet,
            StellarNetwork::Custom("private".to_string()),
        ] {
            let generated = generate(&request(&network, ""));
            let cfg = format!(
                "{}{}",
                generated.render_root_keys(),
                generated.render_table_sections()
            );
            parse(&cfg);
        }
    }
}

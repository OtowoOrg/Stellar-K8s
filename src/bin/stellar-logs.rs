//! stellar-logs — CLI to fetch and search archived logs from S3
//!
//! # Usage
//!
//! ```text
//! # List available log archives for a node
//! stellar-logs list --bucket my-logs --node my-validator --date 2026-04-25
//!
//! # Fetch and print logs (decompresses gzip on the fly)
//! stellar-logs fetch --bucket my-logs --node my-validator --date 2026-04-25
//!
//! # Search logs for a pattern
//! stellar-logs search --bucket my-logs --node my-validator --date 2026-04-25 --pattern "ERROR"
//!
//! # Fetch a specific archive file
//! stellar-logs fetch --bucket my-logs --key stellar-logs/my-validator/2026-04-25/12-00-00-000001.log.gz
//! ```

use std::io::Read;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use flate2::read::GzDecoder;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

// ---------------------------------------------------------------------------
// CLI definition
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(
    name = "stellar-logs",
    about = "Fetch and search Stellar node logs archived in S3",
    version
)]
struct Cli {
    /// AWS region (overrides AWS_DEFAULT_REGION)
    #[arg(long, env = "S3_REGION", default_value = "us-east-1")]
    region: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List archived log files for a node on a given date
    List(ListArgs),
    /// Fetch and print archived logs (decompresses gzip)
    Fetch(FetchArgs),
    /// Search archived logs for a regex pattern
    Search(SearchArgs),
}

#[derive(Parser)]
struct ListArgs {
    /// S3 bucket name
    #[arg(long, env = "S3_BUCKET")]
    bucket: String,

    /// Key prefix (default: "stellar-logs")
    #[arg(long, default_value = "stellar-logs")]
    prefix: String,

    /// Node name (pod name or StellarNode name)
    #[arg(long)]
    node: String,

    /// Date to list (YYYY-MM-DD). Defaults to today.
    #[arg(long)]
    date: Option<String>,
}

#[derive(Parser)]
struct FetchArgs {
    /// S3 bucket name
    #[arg(long, env = "S3_BUCKET")]
    bucket: String,

    /// Key prefix (default: "stellar-logs")
    #[arg(long, default_value = "stellar-logs")]
    prefix: String,

    /// Node name
    #[arg(long)]
    node: Option<String>,

    /// Date to fetch (YYYY-MM-DD). Defaults to today.
    #[arg(long)]
    date: Option<String>,

    /// Fetch a specific S3 key instead of listing by node/date
    #[arg(long)]
    key: Option<String>,
}

#[derive(Parser)]
struct SearchArgs {
    /// S3 bucket name
    #[arg(long, env = "S3_BUCKET")]
    bucket: String,

    /// Key prefix (default: "stellar-logs")
    #[arg(long, default_value = "stellar-logs")]
    prefix: String,

    /// Node name
    #[arg(long)]
    node: String,

    /// Date to search (YYYY-MM-DD). Defaults to today.
    #[arg(long)]
    date: Option<String>,

    /// Pattern to search for (substring match, case-insensitive)
    #[arg(long, short)]
    pattern: String,

    /// Print N lines of context around each match
    #[arg(long, default_value = "0")]
    context: usize,
}

// ---------------------------------------------------------------------------
// S3 helpers (unsigned GET — relies on bucket policy or pre-signed URLs)
// For authenticated access, AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY must be set.
// ---------------------------------------------------------------------------

struct S3Client {
    http: reqwest::Client,
    region: String,
}

impl S3Client {
    fn new(region: &str) -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(60))
                .build()?,
            region: region.to_string(),
        })
    }

    /// List objects under `prefix` in `bucket`.
    async fn list_objects(&self, bucket: &str, prefix: &str) -> Result<Vec<String>> {
        let url = format!(
            "https://s3.{}.amazonaws.com/{}?list-type=2&prefix={}&delimiter=/",
            self.region,
            bucket,
            urlencoding::encode(prefix)
        );

        let resp = self
            .signed_get(&url)
            .await
            .context("S3 ListObjectsV2 failed")?;

        // Parse the XML response to extract <Key> elements.
        let body = resp.text().await?;
        let keys = parse_list_keys(&body);
        Ok(keys)
    }

    /// Download an object and return its raw bytes.
    async fn get_object(&self, bucket: &str, key: &str) -> Result<Vec<u8>> {
        let url = format!(
            "https://s3.{}.amazonaws.com/{}/{}",
            self.region, bucket, key
        );
        let resp = self.signed_get(&url).await.context("S3 GET failed")?;
        if !resp.status().is_success() {
            anyhow::bail!("S3 GET {key}: HTTP {}", resp.status());
        }
        Ok(resp.bytes().await?.to_vec())
    }

    async fn signed_get(&self, url: &str) -> Result<reqwest::Response> {
        use std::env;
        let access_key = env::var("AWS_ACCESS_KEY_ID").unwrap_or_default();
        let secret_key = env::var("AWS_SECRET_ACCESS_KEY").unwrap_or_default();

        if access_key.is_empty() {
            // No credentials — try unsigned (public bucket or pre-signed URL).
            return Ok(self.http.get(url).send().await?);
        }

        let now = chrono::Utc::now();
        let date_str = now.format("%Y%m%d").to_string();
        let datetime_str = now.format("%Y%m%dT%H%M%SZ").to_string();
        let session_token = env::var("AWS_SESSION_TOKEN").ok();

        // Parse host from URL
        let parsed = url::Url::parse(url)?;
        let host = parsed.host_str().unwrap_or("").to_string();
        let path_and_query = format!(
            "{}{}",
            parsed.path(),
            parsed.query().map(|q| format!("?{q}")).unwrap_or_default()
        );

        let payload_hash = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"; // SHA256("")

        let mut canonical_headers = format!(
            "host:{host}\nx-amz-content-sha256:{payload_hash}\nx-amz-date:{datetime_str}\n"
        );
        let mut signed_headers = "host;x-amz-content-sha256;x-amz-date".to_string();

        if let Some(ref tok) = session_token {
            canonical_headers.push_str(&format!("x-amz-security-token:{tok}\n"));
            signed_headers.push_str(";x-amz-security-token");
        }

        let (path, query) = path_and_query
            .split_once('?')
            .map(|(p, q)| (p.to_string(), q.to_string()))
            .unwrap_or_else(|| (path_and_query.clone(), String::new()));

        let canonical_request =
            format!("GET\n{path}\n{query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}");

        let scope = format!("{date_str}/{}/s3/aws4_request", self.region);
        let string_to_sign = {
            use sha2::{Digest, Sha256};
            let mut h = Sha256::new();
            h.update(canonical_request.as_bytes());
            format!(
                "AWS4-HMAC-SHA256\n{datetime_str}\n{scope}\n{}",
                hex::encode(h.finalize())
            )
        };

        let signing_key = {
            use hmac::{Hmac, Mac};
            use sha2::Sha256;
            type HmacSha256 = Hmac<Sha256>;
            let k_date = {
                let mut m =
                    HmacSha256::new_from_slice(format!("AWS4{secret_key}").as_bytes()).unwrap();
                m.update(date_str.as_bytes());
                m.finalize().into_bytes()
            };
            let k_region = {
                let mut m = HmacSha256::new_from_slice(&k_date).unwrap();
                m.update(self.region.as_bytes());
                m.finalize().into_bytes()
            };
            let k_service = {
                let mut m = HmacSha256::new_from_slice(&k_region).unwrap();
                m.update(b"s3");
                m.finalize().into_bytes()
            };
            let mut m = HmacSha256::new_from_slice(&k_service).unwrap();
            m.update(b"aws4_request");
            m.finalize().into_bytes()
        };

        let signature = {
            use hmac::{Hmac, Mac};
            use sha2::Sha256;
            type HmacSha256 = Hmac<Sha256>;
            let mut m = HmacSha256::new_from_slice(&signing_key).unwrap();
            m.update(string_to_sign.as_bytes());
            hex::encode(m.finalize().into_bytes())
        };

        let auth = format!(
            "AWS4-HMAC-SHA256 Credential={access_key}/{scope}, SignedHeaders={signed_headers}, Signature={signature}"
        );

        let mut req = self
            .http
            .get(url)
            .header("x-amz-date", &datetime_str)
            .header("x-amz-content-sha256", payload_hash)
            .header("Authorization", auth);

        if let Some(tok) = session_token {
            req = req.header("x-amz-security-token", tok);
        }

        Ok(req.send().await?)
    }
}

/// Extract `<Key>` values from an S3 ListObjectsV2 XML response.
fn parse_list_keys(xml: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<Key>") {
        rest = &rest[start + 5..];
        if let Some(end) = rest.find("</Key>") {
            keys.push(rest[..end].to_string());
            rest = &rest[end + 6..];
        }
    }
    keys
}

/// Decompress a gzip byte slice into a UTF-8 string.
fn decompress(data: &[u8]) -> Result<String> {
    let mut decoder = GzDecoder::new(data);
    let mut out = String::new();
    decoder.read_to_string(&mut out)?;
    Ok(out)
}

fn today() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

// ---------------------------------------------------------------------------
// Command implementations
// ---------------------------------------------------------------------------

async fn cmd_list(region: &str, args: &ListArgs) -> Result<()> {
    let date = args.date.clone().unwrap_or_else(today);
    let prefix = format!("{}/{}/{}/", args.prefix, args.node, date);
    let s3 = S3Client::new(region)?;
    let keys = s3.list_objects(&args.bucket, &prefix).await?;
    if keys.is_empty() {
        println!("No archives found for node '{}' on {date}", args.node);
    } else {
        for key in &keys {
            println!("{key}");
        }
        println!("\n{} archive(s) found.", keys.len());
    }
    Ok(())
}

async fn cmd_fetch(region: &str, args: &FetchArgs) -> Result<()> {
    let s3 = S3Client::new(region)?;

    let keys: Vec<String> = if let Some(key) = &args.key {
        vec![key.clone()]
    } else {
        let node = args
            .node
            .as_deref()
            .context("--node required when --key is not set")?;
        let date = args.date.clone().unwrap_or_else(today);
        let prefix = format!("{}/{}/{}/", args.prefix, date, node);
        s3.list_objects(&args.bucket, &prefix).await?
    };

    for key in &keys {
        eprintln!("--- {key} ---");
        let data = s3.get_object(&args.bucket, key).await?;
        let text = decompress(&data)?;
        print!("{text}");
    }
    Ok(())
}

async fn cmd_search(region: &str, args: &SearchArgs) -> Result<()> {
    let date = args.date.clone().unwrap_or_else(today);
    let prefix = format!("{}/{}/{}/", args.prefix, args.node, date);
    let s3 = S3Client::new(region)?;
    let keys = s3.list_objects(&args.bucket, &prefix).await?;

    let pattern_lower = args.pattern.to_lowercase();
    let mut total_matches = 0usize;

    for key in &keys {
        let data = s3.get_object(&args.bucket, key).await?;
        let text = decompress(&data)?;
        let lines: Vec<&str> = text.lines().collect();

        for (i, line) in lines.iter().enumerate() {
            if line.to_lowercase().contains(&pattern_lower) {
                total_matches += 1;
                // Print context lines before
                let start = i.saturating_sub(args.context);
                for ctx_line in &lines[start..i] {
                    println!("  {ctx_line}");
                }
                // Print the matching line highlighted
                println!("→ {line}");
                // Print context lines after
                let end = (i + 1 + args.context).min(lines.len());
                for ctx_line in &lines[i + 1..end] {
                    println!("  {ctx_line}");
                }
                if args.context > 0 {
                    println!("--");
                }
            }
        }
    }

    eprintln!(
        "\n{total_matches} match(es) across {} archive(s).",
        keys.len()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    match &cli.command {
        Commands::List(args) => cmd_list(&cli.region, args).await,
        Commands::Fetch(args) => cmd_fetch(&cli.region, args).await,
        Commands::Search(args) => cmd_search(&cli.region, args).await,
    }
}

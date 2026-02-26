use axum::{
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use once_cell::sync::Lazy;
use pqcrypto_dilithium::dilithium2::{
    detached_sign, keypair as dilithium2_keypair, verify_detached_signature, DetachedSignature,
    PublicKey as DilithiumPublicKey, SecretKey as DilithiumSecretKey,
};
use pqcrypto_kyber::kyber512::{decapsulate, encapsulate, keypair as kyber512_keypair};
use pqcrypto_traits::sign::{DetachedSignature as _, PublicKey as _};
use serde::{Deserialize, Serialize};
use std::time::Instant;
use tokio::net::TcpListener;
use tracing::info;

static KEYPAIR: Lazy<(DilithiumPublicKey, DilithiumSecretKey)> = Lazy::new(dilithium2_keypair);

#[derive(Serialize)]
struct BenchmarkResult {
    kyber512_keygen_ms: f64,
    kyber512_encaps_ms: f64,
    kyber512_decaps_ms: f64,
    dilithium2_keygen_ms: f64,
    dilithium2_sign_ms: f64,
    dilithium2_verify_ms: f64,
}

#[derive(Deserialize)]
struct SignRequest {
    message: String,
}

#[derive(Serialize)]
struct SignResponse {
    message: String,
    signature_b64: String,
    public_key_b64: String,
}

#[derive(Deserialize)]
struct VerifyRequest {
    message: String,
    signature_b64: String,
    public_key_b64: String,
}

#[derive(Serialize)]
struct VerifyResponse {
    valid: bool,
}

async fn benchmark() -> Json<BenchmarkResult> {
    let start = Instant::now();
    let (k_pk, k_sk) = kyber512_keypair();
    let kyber512_keygen_ms = start.elapsed().as_secs_f64() * 1000.0;

    let start = Instant::now();
    let (_ss, ct) = encapsulate(&k_pk);
    let kyber512_encaps_ms = start.elapsed().as_secs_f64() * 1000.0;

    let start = Instant::now();
    let _ss2 = decapsulate(&ct, &k_sk);
    let kyber512_decaps_ms = start.elapsed().as_secs_f64() * 1000.0;

    let start = Instant::now();
    let (d_pk, d_sk) = dilithium2_keypair();
    let dilithium2_keygen_ms = start.elapsed().as_secs_f64() * 1000.0;

    let msg = b"benchmark message for pqc audit within k8s cluster operator";
    let start = Instant::now();
    let sig = detached_sign(msg, &d_sk);
    let dilithium2_sign_ms = start.elapsed().as_secs_f64() * 1000.0;

    let start = Instant::now();
    let _ = verify_detached_signature(&sig, msg, &d_pk);
    let dilithium2_verify_ms = start.elapsed().as_secs_f64() * 1000.0;

    Json(BenchmarkResult {
        kyber512_keygen_ms,
        kyber512_encaps_ms,
        kyber512_decaps_ms,
        dilithium2_keygen_ms,
        dilithium2_sign_ms,
        dilithium2_verify_ms,
    })
}

async fn sign(Json(payload): Json<SignRequest>) -> Json<SignResponse> {
    let (pk, sk) = &*KEYPAIR;
    let sig = detached_sign(payload.message.as_bytes(), sk);
    Json(SignResponse {
        message: payload.message,
        signature_b64: STANDARD.encode(sig.as_bytes()),
        public_key_b64: STANDARD.encode(pk.as_bytes()),
    })
}

async fn verify(Json(payload): Json<VerifyRequest>) -> Json<VerifyResponse> {
    let mut valid = false;
    if let (Ok(sig_bytes), Ok(pk_bytes)) = (
        STANDARD.decode(&payload.signature_b64),
        STANDARD.decode(&payload.public_key_b64),
    ) {
        if let (Ok(sig), Ok(pk)) = (
            DetachedSignature::from_bytes(&sig_bytes),
            DilithiumPublicKey::from_bytes(&pk_bytes),
        ) {
            valid = verify_detached_signature(&sig, payload.message.as_bytes(), &pk).is_ok();
        }
    }
    Json(VerifyResponse { valid })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    // ensure keypair is generated early
    let _ = &*KEYPAIR;

    let app = Router::new()
        .route("/benchmark", get(benchmark))
        .route("/sign", post(sign))
        .route("/verify", post(verify));

    let addr = "0.0.0.0:8080";
    info!("Starting PQC Sidecar on {}", addr);
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

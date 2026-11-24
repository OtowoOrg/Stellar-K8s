/**
 * Mock certificate data fixtures for the Ingress TLS Certificate dashboard.
 *
 * Current reference time: the tests use Date.now() for relative calculations,
 * so these fixtures express expiresAt as an offset from NOW. At module load the
 * absolute dates are computed once so tests and the demo UI see consistent values.
 *
 * Categories covered:
 *  - healthy:    > 30 days remaining
 *  - warning:    1 – 29 days remaining (< 30 d)
 *  - critical:   0 – 6 days remaining  (< 7 d)
 *  - expired:    already past expiry (< 0 days)
 */

const ONE_DAY_MS = 24 * 60 * 60 * 1000;

/** Build an ISO-8601 date string offset from now by `deltaDays`. */
function daysFromNow(deltaDays) {
  return new Date(Date.now() + deltaDays * ONE_DAY_MS).toISOString();
}

/** @type {import('./certUtils.js').IngressCertRecord[]} */
export const MOCK_CERTS = [
  // ── Healthy (> 30 d) ─────────────────────────────────────────────────────
  {
    id: 'horizon-mainnet-tls',
    namespace: 'stellar',
    ingressName: 'horizon-ingress',
    host: 'horizon.stellar.example.com',
    secretName: 'horizon-mainnet-tls',
    issuer: 'Let\'s Encrypt Authority X3',
    issuerOrg: 'Let\'s Encrypt',
    subject: 'CN=horizon.stellar.example.com',
    sans: ['horizon.stellar.example.com', 'api.stellar.example.com'],
    notBefore: daysFromNow(-300),
    expiresAt: daysFromNow(65),
    certManagerManaged: true,
    renewalTriggered: false,
  },
  {
    id: 'soroban-rpc-tls',
    namespace: 'stellar',
    ingressName: 'soroban-rpc-ingress',
    host: 'rpc.stellar.example.com',
    secretName: 'soroban-rpc-tls',
    issuer: 'Let\'s Encrypt Authority X3',
    issuerOrg: 'Let\'s Encrypt',
    subject: 'CN=rpc.stellar.example.com',
    sans: ['rpc.stellar.example.com'],
    notBefore: daysFromNow(-200),
    expiresAt: daysFromNow(90),
    certManagerManaged: true,
    renewalTriggered: false,
  },
  {
    id: 'grafana-tls',
    namespace: 'monitoring',
    ingressName: 'grafana-ingress',
    host: 'grafana.ops.example.com',
    secretName: 'grafana-tls',
    issuer: 'DigiCert SHA2 Secure Server CA',
    issuerOrg: 'DigiCert Inc',
    subject: 'CN=grafana.ops.example.com',
    sans: ['grafana.ops.example.com', 'metrics.ops.example.com'],
    notBefore: daysFromNow(-150),
    expiresAt: daysFromNow(215),
    certManagerManaged: false,
    renewalTriggered: false,
  },
  {
    id: 'argocd-tls',
    namespace: 'argocd',
    ingressName: 'argocd-server-ingress',
    host: 'argocd.infra.example.com',
    secretName: 'argocd-tls',
    issuer: 'Let\'s Encrypt Authority X3',
    issuerOrg: 'Let\'s Encrypt',
    subject: 'CN=argocd.infra.example.com',
    sans: ['argocd.infra.example.com'],
    notBefore: daysFromNow(-60),
    expiresAt: daysFromNow(30),
    certManagerManaged: true,
    renewalTriggered: false,
  },

  // ── Warning (< 30 d but >= 7 d) ──────────────────────────────────────────
  {
    id: 'validator-api-tls',
    namespace: 'stellar',
    ingressName: 'validator-api-ingress',
    host: 'validator.stellar.example.com',
    secretName: 'validator-api-tls',
    issuer: 'Let\'s Encrypt Authority X3',
    issuerOrg: 'Let\'s Encrypt',
    subject: 'CN=validator.stellar.example.com',
    sans: ['validator.stellar.example.com'],
    notBefore: daysFromNow(-340),
    expiresAt: daysFromNow(25),
    certManagerManaged: true,
    renewalTriggered: false,
  },
  {
    id: 'webhook-tls',
    namespace: 'stellar-system',
    ingressName: 'webhook-ingress',
    host: 'webhook.stellar-system.example.com',
    secretName: 'webhook-tls',
    issuer: 'ZeroSSL',
    issuerOrg: 'ZeroSSL',
    subject: 'CN=webhook.stellar-system.example.com',
    sans: ['webhook.stellar-system.example.com', 'admission.stellar-system.example.com'],
    notBefore: daysFromNow(-345),
    expiresAt: daysFromNow(15),
    certManagerManaged: false,
    renewalTriggered: false,
  },
  {
    id: 'dashboard-tls',
    namespace: 'stellar-system',
    ingressName: 'dashboard-ingress',
    host: 'dashboard.stellar-system.example.com',
    secretName: 'dashboard-tls',
    issuer: 'Let\'s Encrypt Authority X3',
    issuerOrg: 'Let\'s Encrypt',
    subject: 'CN=dashboard.stellar-system.example.com',
    sans: ['dashboard.stellar-system.example.com'],
    notBefore: daysFromNow(-340),
    expiresAt: daysFromNow(10),
    certManagerManaged: true,
    renewalTriggered: true,
  },

  // ── Critical (< 7 d) ─────────────────────────────────────────────────────
  {
    id: 'horizon-testnet-tls',
    namespace: 'stellar-testnet',
    ingressName: 'horizon-testnet-ingress',
    host: 'horizon-testnet.stellar.example.com',
    secretName: 'horizon-testnet-tls',
    issuer: 'Let\'s Encrypt Authority X3',
    issuerOrg: 'Let\'s Encrypt',
    subject: 'CN=horizon-testnet.stellar.example.com',
    sans: ['horizon-testnet.stellar.example.com'],
    notBefore: daysFromNow(-359),
    expiresAt: daysFromNow(6),
    certManagerManaged: true,
    renewalTriggered: false,
  },
  {
    id: 'soroban-testnet-tls',
    namespace: 'stellar-testnet',
    ingressName: 'soroban-testnet-ingress',
    host: 'rpc-testnet.stellar.example.com',
    secretName: 'soroban-testnet-tls',
    issuer: 'ZeroSSL',
    issuerOrg: 'ZeroSSL',
    subject: 'CN=rpc-testnet.stellar.example.com',
    sans: ['rpc-testnet.stellar.example.com', 'soroban.testnet.stellar.example.com'],
    notBefore: daysFromNow(-358),
    expiresAt: daysFromNow(3),
    certManagerManaged: false,
    renewalTriggered: false,
  },
  {
    id: 'prometheus-tls',
    namespace: 'monitoring',
    ingressName: 'prometheus-ingress',
    host: 'prometheus.ops.example.com',
    secretName: 'prometheus-tls',
    issuer: 'Let\'s Encrypt Authority X3',
    issuerOrg: 'Let\'s Encrypt',
    subject: 'CN=prometheus.ops.example.com',
    sans: ['prometheus.ops.example.com'],
    notBefore: daysFromNow(-364),
    expiresAt: daysFromNow(1),
    certManagerManaged: true,
    renewalTriggered: false,
  },

  // ── Expired ───────────────────────────────────────────────────────────────
  {
    id: 'legacy-api-tls',
    namespace: 'legacy',
    ingressName: 'legacy-api-ingress',
    host: 'legacy-api.stellar.example.com',
    secretName: 'legacy-api-tls',
    issuer: 'Let\'s Encrypt Authority X3',
    issuerOrg: 'Let\'s Encrypt',
    subject: 'CN=legacy-api.stellar.example.com',
    sans: ['legacy-api.stellar.example.com'],
    notBefore: daysFromNow(-395),
    expiresAt: daysFromNow(-30),
    certManagerManaged: false,
    renewalTriggered: false,
  },
  {
    id: 'staging-tls',
    namespace: 'staging',
    ingressName: 'staging-ingress',
    host: 'staging.stellar.example.com',
    secretName: 'staging-tls',
    issuer: 'DigiCert SHA2 Secure Server CA',
    issuerOrg: 'DigiCert Inc',
    subject: 'CN=staging.stellar.example.com',
    sans: ['staging.stellar.example.com', '*.staging.stellar.example.com'],
    notBefore: daysFromNow(-400),
    expiresAt: daysFromNow(-5),
    certManagerManaged: false,
    renewalTriggered: false,
  },
];

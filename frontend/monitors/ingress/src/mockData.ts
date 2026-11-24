import { calculateDaysRemaining, getCertUrgency } from "./certUtils.ts";
import type { CertificateInfo, RouteType } from "./types.ts";

interface RawCertDef {
    id: string;
    name: string;
    namespace: string;
    cluster: string;
    routeType: RouteType;
    hostname: string;
    serviceEndpoint: string;
    issuer: string;
    issuerType: string;
    sans: string[];
    daysOffset: number; // positive = future days, negative = expired days
    validityTotalDays: number;
    serialNumber: string;
    signatureAlgorithm: string;
    autoRenewal: boolean;
    secretName: string;
    renewalStatus?: "idle" | "renewing" | "renewed" | "failed";
    annotations?: Record<string, string>;
}

const RAW_MOCK_DEFINITIONS: RawCertDef[] = [
    // --- CRITICAL: Expired & Near-Expiry (< 7 days) ---
    {
        id: "cert-horizon-mainnet-primary",
        name: "horizon-mainnet-ingress",
        namespace: "stellar-mainnet",
        cluster: "us-east-prod",
        routeType: "Ingress",
        hostname: "horizon.stellar.org",
        serviceEndpoint: "horizon-service.stellar-mainnet:8000",
        issuer: "Let's Encrypt Authority X3",
        issuerType: "Let's Encrypt",
        sans: [
            "horizon.stellar.org",
            "api.horizon.stellar.org",
            "horizon-us.stellar.org",
        ],
        daysOffset: -2, // EXPIRED
        validityTotalDays: 90,
        serialNumber: "04:7A:B9:E1:92:4C:6F:02:18:9B",
        signatureAlgorithm: "ECDSA-SHA256",
        autoRenewal: false,
        secretName: "horizon-mainnet-tls",
        annotations: {
            "cert-manager.io/cluster-issuer": "letsencrypt-prod",
            "ingress.class": "nginx",
            "stellar.org/criticality": "high",
        },
    },
    {
        id: "cert-soroban-rpc-mainnet-us",
        name: "soroban-rpc-mainnet-route",
        namespace: "stellar-mainnet",
        cluster: "us-east-prod",
        routeType: "HTTPRoute",
        hostname: "soroban-rpc.stellar.org",
        serviceEndpoint: "soroban-rpc.stellar-mainnet:8000",
        issuer: "DigiCert TLS RSA SHA256 2020 CA1",
        issuerType: "DigiCert",
        sans: [
            "soroban-rpc.stellar.org",
            "rpc.soroban.stellar.org",
            "*.soroban.stellar.org",
        ],
        daysOffset: 1, // CRITICAL: 1 day
        validityTotalDays: 365,
        serialNumber: "08:E4:11:F2:A3:88:9C:61:55:0D",
        signatureAlgorithm: "RSA-4096",
        autoRenewal: true,
        secretName: "soroban-rpc-tls-cert",
        annotations: {
            "gateway.networking.k8s.io/gateway-name": "stellar-gateway",
            "stellar.org/service-tier": "tier-1",
        },
    },
    {
        id: "cert-horizon-testnet-public",
        name: "horizon-testnet-ingress",
        namespace: "stellar-testnet",
        cluster: "us-west-testnet",
        routeType: "Ingress",
        hostname: "horizon-testnet.stellar.org",
        serviceEndpoint: "horizon-testnet-svc.stellar-testnet:8000",
        issuer: "Let's Encrypt Authority X3",
        issuerType: "Let's Encrypt",
        sans: ["horizon-testnet.stellar.org", "api-testnet.stellar.org"],
        daysOffset: 3, // CRITICAL: 3 days
        validityTotalDays: 90,
        serialNumber: "03:19:FA:CC:41:88:02:DF:39:AA",
        signatureAlgorithm: "ECDSA-SHA256",
        autoRenewal: true,
        secretName: "horizon-testnet-tls",
        annotations: {
            "cert-manager.io/cluster-issuer": "letsencrypt-prod",
            "ingress.class": "traefik",
        },
    },
    {
        id: "cert-soroban-rpc-testnet",
        name: "soroban-rpc-testnet-route",
        namespace: "stellar-testnet",
        cluster: "us-west-testnet",
        routeType: "HTTPRoute",
        hostname: "soroban-testnet.stellar.org",
        serviceEndpoint: "soroban-rpc-testnet.stellar-testnet:8000",
        issuer: "Let's Encrypt Authority X3",
        issuerType: "Let's Encrypt",
        sans: [
            "soroban-testnet.stellar.org",
            "rpc-testnet.soroban.stellar.org",
        ],
        daysOffset: 5, // CRITICAL: 5 days
        validityTotalDays: 90,
        serialNumber: "02:77:88:99:AA:BB:CC:DD:EE:FF",
        signatureAlgorithm: "ECDSA-SHA256",
        autoRenewal: false,
        secretName: "soroban-testnet-tls-secret",
        annotations: {
            "cert-manager.io/issue-temporary-certificate": "true",
        },
    },
    {
        id: "cert-friendbot-testnet",
        name: "friendbot-ingress",
        namespace: "stellar-testnet",
        cluster: "us-west-testnet",
        routeType: "Ingress",
        hostname: "friendbot.stellar.org",
        serviceEndpoint: "friendbot-svc.stellar-testnet:8000",
        issuer: "ZeroSSL RSA Domain Secure Site CA",
        issuerType: "ZeroSSL",
        sans: ["friendbot.stellar.org", "friendbot-testnet.stellar.org"],
        daysOffset: 6, // CRITICAL: 6 days
        validityTotalDays: 90,
        serialNumber: "09:88:77:66:55:44:33:22:11:00",
        signatureAlgorithm: "RSA-2048",
        autoRenewal: true,
        secretName: "friendbot-tls",
        annotations: {
            "cert-manager.io/cluster-issuer": "zerossl-prod",
        },
    },

    // --- WARNING: Expiring Soon (7 to 30 days) ---
    {
        id: "cert-horizon-futurenet",
        name: "horizon-futurenet-ingress",
        namespace: "stellar-futurenet",
        cluster: "eu-central-futurenet",
        routeType: "Ingress",
        hostname: "horizon-futurenet.stellar.org",
        serviceEndpoint: "horizon-futurenet.stellar-futurenet:8000",
        issuer: "Let's Encrypt Authority X3",
        issuerType: "Let's Encrypt",
        sans: ["horizon-futurenet.stellar.org", "api-futurenet.stellar.org"],
        daysOffset: 8,
        validityTotalDays: 90,
        serialNumber: "05:B1:32:89:FE:44:01:A2:CC:11",
        signatureAlgorithm: "ECDSA-SHA256",
        autoRenewal: true,
        secretName: "horizon-futurenet-tls",
        annotations: {
            "cert-manager.io/cluster-issuer": "letsencrypt-staging",
        },
    },
    {
        id: "cert-soroban-futurenet-rpc",
        name: "soroban-futurenet-route",
        namespace: "stellar-futurenet",
        cluster: "eu-central-futurenet",
        routeType: "HTTPRoute",
        hostname: "rpc-futurenet.stellar.org",
        serviceEndpoint: "soroban-rpc-futurenet.stellar-futurenet:8000",
        issuer: "Vault Enterprise CA",
        issuerType: "Vault",
        sans: [
            "rpc-futurenet.stellar.org",
            "soroban-rpc-futurenet.stellar.org",
        ],
        daysOffset: 12,
        validityTotalDays: 60,
        serialNumber: "06:12:44:66:88:AA:CC:EE:11:33",
        signatureAlgorithm: "ECDSA-SHA256",
        autoRenewal: true,
        secretName: "futurenet-rpc-vault-tls",
        annotations: { "vault.hashicorp.com/role": "stellar-futurenet" },
    },
    {
        id: "cert-horizon-eu-replica",
        name: "horizon-eu-ingress",
        namespace: "stellar-mainnet",
        cluster: "eu-central-prod",
        routeType: "Ingress",
        hostname: "horizon-eu.stellar.org",
        serviceEndpoint: "horizon-eu-svc.stellar-mainnet:8000",
        issuer: "Let's Encrypt Authority X3",
        issuerType: "Let's Encrypt",
        sans: ["horizon-eu.stellar.org", "api-eu.stellar.org"],
        daysOffset: 15,
        validityTotalDays: 90,
        serialNumber: "07:33:55:77:99:BB:DD:FF:22:44",
        signatureAlgorithm: "ECDSA-SHA256",
        autoRenewal: true,
        secretName: "horizon-eu-tls",
        annotations: { "ingress.class": "nginx" },
    },
    {
        id: "cert-soroban-rpc-eu-prod",
        name: "soroban-rpc-eu-route",
        namespace: "stellar-mainnet",
        cluster: "eu-central-prod",
        routeType: "HTTPRoute",
        hostname: "soroban-eu.stellar.org",
        serviceEndpoint: "soroban-eu.stellar-mainnet:8000",
        issuer: "DigiCert TLS RSA SHA256 2020 CA1",
        issuerType: "DigiCert",
        sans: ["soroban-eu.stellar.org", "rpc-eu.soroban.stellar.org"],
        daysOffset: 19,
        validityTotalDays: 365,
        serialNumber: "01:AA:BB:CC:DD:EE:FF:00:11:22",
        signatureAlgorithm: "RSA-4096",
        autoRenewal: true,
        secretName: "soroban-eu-tls-secret",
        annotations: {
            "gateway.networking.k8s.io/gateway-name": "stellar-eu-gw",
        },
    },
    {
        id: "cert-history-archive-us",
        name: "history-archive-ingress",
        namespace: "stellar-system",
        cluster: "us-east-prod",
        routeType: "Ingress",
        hostname: "history.stellar.org",
        serviceEndpoint: "history-cache.stellar-system:80",
        issuer: "Let's Encrypt Authority X3",
        issuerType: "Let's Encrypt",
        sans: ["history.stellar.org", "history-archive.stellar.org"],
        daysOffset: 22,
        validityTotalDays: 90,
        serialNumber: "04:EE:AA:77:11:88:22:99:33:55",
        signatureAlgorithm: "ECDSA-SHA256",
        autoRenewal: true,
        secretName: "history-archive-tls",
        annotations: { "cert-manager.io/cluster-issuer": "letsencrypt-prod" },
    },
    {
        id: "cert-validator-dashboard-mainnet",
        name: "validator-dashboard-ingress",
        namespace: "stellar-system",
        cluster: "us-east-prod",
        routeType: "Ingress",
        hostname: "status.stellar.org",
        serviceEndpoint: "status-dashboard.stellar-system:3000",
        issuer: "ZeroSSL RSA Domain Secure Site CA",
        issuerType: "ZeroSSL",
        sans: ["status.stellar.org", "health.stellar.org"],
        daysOffset: 28,
        validityTotalDays: 90,
        serialNumber: "09:11:22:33:44:55:66:77:88:99",
        signatureAlgorithm: "RSA-2048",
        autoRenewal: false,
        secretName: "status-dashboard-tls",
        annotations: { "ingress.class": "nginx" },
    },

    // --- HEALTHY: (> 30 days) ---
    {
        id: "cert-horizon-ap-east",
        name: "horizon-ap-ingress",
        namespace: "stellar-mainnet",
        cluster: "ap-southeast-prod",
        routeType: "Ingress",
        hostname: "horizon-ap.stellar.org",
        serviceEndpoint: "horizon-ap.stellar-mainnet:8000",
        issuer: "Let's Encrypt Authority X3",
        issuerType: "Let's Encrypt",
        sans: ["horizon-ap.stellar.org", "api-ap.stellar.org"],
        daysOffset: 45,
        validityTotalDays: 90,
        serialNumber: "10:01:02:03:04:05:06:07:08:09",
        signatureAlgorithm: "ECDSA-SHA256",
        autoRenewal: true,
        secretName: "horizon-ap-tls",
    },
    {
        id: "cert-soroban-rpc-ap-east",
        name: "soroban-rpc-ap-route",
        namespace: "stellar-mainnet",
        cluster: "ap-southeast-prod",
        routeType: "HTTPRoute",
        hostname: "soroban-ap.stellar.org",
        serviceEndpoint: "soroban-ap.stellar-mainnet:8000",
        issuer: "DigiCert TLS RSA SHA256 2020 CA1",
        issuerType: "DigiCert",
        sans: ["soroban-ap.stellar.org", "rpc-ap.soroban.stellar.org"],
        daysOffset: 62,
        validityTotalDays: 365,
        serialNumber: "11:12:13:14:15:16:17:18:19:20",
        signatureAlgorithm: "RSA-4096",
        autoRenewal: true,
        secretName: "soroban-ap-tls-secret",
    },
    {
        id: "cert-archive-node-1",
        name: "archive-node-1-ingress",
        namespace: "stellar-nodes",
        cluster: "us-east-prod",
        routeType: "Ingress",
        hostname: "archive-1.stellar.org",
        serviceEndpoint: "stellar-archive-1.stellar-nodes:11626",
        issuer: "Vault Enterprise CA",
        issuerType: "Vault",
        sans: ["archive-1.stellar.org"],
        daysOffset: 74,
        validityTotalDays: 180,
        serialNumber: "12:22:32:42:52:62:72:82:92:02",
        signatureAlgorithm: "ECDSA-SHA256",
        autoRenewal: true,
        secretName: "archive-node-1-tls",
    },
    {
        id: "cert-archive-node-2",
        name: "archive-node-2-ingress",
        namespace: "stellar-nodes",
        cluster: "eu-central-prod",
        routeType: "Ingress",
        hostname: "archive-2.stellar.org",
        serviceEndpoint: "stellar-archive-2.stellar-nodes:11626",
        issuer: "Vault Enterprise CA",
        issuerType: "Vault",
        sans: ["archive-2.stellar.org"],
        daysOffset: 89,
        validityTotalDays: 180,
        serialNumber: "13:33:43:53:63:73:83:93:03:13",
        signatureAlgorithm: "ECDSA-SHA256",
        autoRenewal: true,
        secretName: "archive-node-2-tls",
    },
    {
        id: "cert-validator-us-node-1",
        name: "validator-us-1-route",
        namespace: "stellar-nodes",
        cluster: "us-east-prod",
        routeType: "HTTPRoute",
        hostname: "validator-us1.stellar.org",
        serviceEndpoint: "stellar-core-us1.stellar-nodes:11625",
        issuer: "Internal Cluster CA",
        issuerType: "Internal CA",
        sans: ["validator-us1.stellar.org", "v-us1.stellar.org"],
        daysOffset: 120,
        validityTotalDays: 365,
        serialNumber: "14:44:54:64:74:84:94:04:14:24",
        signatureAlgorithm: "ECDSA-SHA384",
        autoRenewal: true,
        secretName: "validator-us1-mtls-cert",
    },
    {
        id: "cert-validator-us-node-2",
        name: "validator-us-2-route",
        namespace: "stellar-nodes",
        cluster: "us-east-prod",
        routeType: "HTTPRoute",
        hostname: "validator-us2.stellar.org",
        serviceEndpoint: "stellar-core-us2.stellar-nodes:11625",
        issuer: "Internal Cluster CA",
        issuerType: "Internal CA",
        sans: ["validator-us2.stellar.org", "v-us2.stellar.org"],
        daysOffset: 135,
        validityTotalDays: 365,
        serialNumber: "15:55:65:75:85:95:05:15:25:35",
        signatureAlgorithm: "ECDSA-SHA384",
        autoRenewal: true,
        secretName: "validator-us2-mtls-cert",
    },
    {
        id: "cert-validator-eu-node-1",
        name: "validator-eu-1-route",
        namespace: "stellar-nodes",
        cluster: "eu-central-prod",
        routeType: "HTTPRoute",
        hostname: "validator-eu1.stellar.org",
        serviceEndpoint: "stellar-core-eu1.stellar-nodes:11625",
        issuer: "Internal Cluster CA",
        issuerType: "Internal CA",
        sans: ["validator-eu1.stellar.org", "v-eu1.stellar.org"],
        daysOffset: 150,
        validityTotalDays: 365,
        serialNumber: "16:66:76:86:96:06:16:26:36:46",
        signatureAlgorithm: "ECDSA-SHA384",
        autoRenewal: true,
        secretName: "validator-eu1-mtls-cert",
    },
    {
        id: "cert-validator-eu-node-2",
        name: "validator-eu-2-route",
        namespace: "stellar-nodes",
        cluster: "eu-central-prod",
        routeType: "HTTPRoute",
        hostname: "validator-eu2.stellar.org",
        serviceEndpoint: "stellar-core-eu2.stellar-nodes:11625",
        issuer: "Internal Cluster CA",
        issuerType: "Internal CA",
        sans: ["validator-eu2.stellar.org", "v-eu2.stellar.org"],
        daysOffset: 180,
        validityTotalDays: 365,
        serialNumber: "17:77:87:97:07:17:27:37:47:57",
        signatureAlgorithm: "ECDSA-SHA384",
        autoRenewal: true,
        secretName: "validator-eu2-mtls-cert",
    },
    {
        id: "cert-operator-metrics-api",
        name: "operator-metrics-ingress",
        namespace: "stellar-system",
        cluster: "us-east-prod",
        routeType: "Ingress",
        hostname: "metrics.stellar-k8s.internal",
        serviceEndpoint: "stellar-operator.stellar-system:9090",
        issuer: "Internal Cluster CA",
        issuerType: "Internal CA",
        sans: [
            "metrics.stellar-k8s.internal",
            "telemetry.stellar-k8s.internal",
        ],
        daysOffset: 240,
        validityTotalDays: 365,
        serialNumber: "18:88:98:08:18:28:38:48:58:68",
        signatureAlgorithm: "ECDSA-SHA256",
        autoRenewal: true,
        secretName: "operator-metrics-tls",
    },
    {
        id: "cert-grafana-observability",
        name: "grafana-observability-ingress",
        namespace: "monitoring",
        cluster: "us-east-prod",
        routeType: "Ingress",
        hostname: "dashboards.stellar.org",
        serviceEndpoint: "grafana.monitoring:3000",
        issuer: "Let's Encrypt Authority X3",
        issuerType: "Let's Encrypt",
        sans: ["dashboards.stellar.org", "grafana.stellar.org"],
        daysOffset: 65,
        validityTotalDays: 90,
        serialNumber: "19:99:09:19:29:39:49:59:69:79",
        signatureAlgorithm: "ECDSA-SHA256",
        autoRenewal: true,
        secretName: "grafana-tls-secret",
    },
];

// Generate an extended fleet of 52 realistic routes for stress testing and 50+ route verification
function generateExtendedMockFleet(): RawCertDef[] {
    const list: RawCertDef[] = [...RAW_MOCK_DEFINITIONS];
    const regions = [
        "us-east",
        "us-west",
        "eu-west",
        "eu-central",
        "ap-south",
        "ap-northeast",
        "sa-east",
        "ca-central",
    ];
    const namespaces = [
        "stellar-mainnet",
        "stellar-testnet",
        "stellar-futurenet",
        "stellar-nodes",
        "stellar-system",
    ];
    const issuers = [
        { name: "Let's Encrypt Authority X3", type: "Let's Encrypt" },
        { name: "DigiCert TLS RSA SHA256 2020 CA1", type: "DigiCert" },
        { name: "ZeroSSL RSA Domain Secure Site CA", type: "ZeroSSL" },
        { name: "Vault Enterprise CA", type: "Vault" },
    ];

    let idCounter = 20;
    for (const region of regions) {
        for (let index = 1; index <= 4; index++) {
            idCounter++;
            const ns = namespaces[(idCounter + index) % namespaces.length];
            const issuerObj = issuers[idCounter % issuers.length];
            const isIngress = idCounter % 2 === 0;
            const days = 14 + ((idCounter * 17) % 320); // distributed expiration days
            const isSoroban = index % 2 === 0;
            const servicePrefix = isSoroban ? "soroban-rpc" : "horizon";
            const hostname = `${servicePrefix}-${region}-node-${index}.stellar.org`;

            list.push({
                id: `cert-fleet-${region}-${index}-${idCounter}`,
                name: `${servicePrefix}-${region}-${index}-${isIngress ? "ingress" : "route"}`,
                namespace: ns,
                cluster: `${region}-prod`,
                routeType: isIngress ? "Ingress" : "HTTPRoute",
                hostname,
                serviceEndpoint: `${servicePrefix}-svc.${ns}:8000`,
                issuer: issuerObj.name,
                issuerType: issuerObj.type,
                sans: [
                    hostname,
                    `${servicePrefix}.${region}.stellar.org`,
                    `*.${region}.stellar.org`,
                ],
                daysOffset: days,
                validityTotalDays: days > 90 ? 365 : 90,
                serialNumber: `20:${((idCounter % 90) + 10).toString(16).toUpperCase()}:${(((idCounter * 3) % 90) + 10).toString(16).toUpperCase()}:AA:BB:CC:DD`,
                signatureAlgorithm:
                    idCounter % 3 === 0 ? "RSA-2048" : "ECDSA-SHA256",
                autoRenewal: idCounter % 5 !== 0,
                secretName: `${servicePrefix}-${region}-${index}-tls`,
                annotations: {
                    "cert-manager.io/cluster-issuer": "letsencrypt-prod",
                    "stellar.org/region": region,
                },
            });
        }
    }

    return list;
}

/**
 * Materializes raw mock certificate definitions into fully calculated `CertificateInfo` records.
 * Reference date defaults to `new Date()`.
 */
export function generateMockCertificates(
    referenceDate: Date = new Date(),
): CertificateInfo[] {
    const rawFleet = generateExtendedMockFleet();

    return rawFleet.map((raw) => {
        const expiryDate = new Date(
            referenceDate.getTime() + raw.daysOffset * 24 * 60 * 60 * 1000,
        );
        const startDate = new Date(
            expiryDate.getTime() - raw.validityTotalDays * 24 * 60 * 60 * 1000,
        );
        const daysRemaining = calculateDaysRemaining(expiryDate, referenceDate);
        const urgency = getCertUrgency(daysRemaining);
        const isExpired = daysRemaining <= 0;

        return {
            id: raw.id,
            name: raw.name,
            namespace: raw.namespace,
            cluster: raw.cluster,
            routeType: raw.routeType,
            hostname: raw.hostname,
            serviceEndpoint: raw.serviceEndpoint,
            issuer: raw.issuer,
            issuerType: raw.issuerType,
            sans: raw.sans,
            notBefore: startDate.toISOString(),
            notAfter: expiryDate.toISOString(),
            daysRemaining,
            urgency,
            isExpired,
            serialNumber: raw.serialNumber,
            signatureAlgorithm: raw.signatureAlgorithm,
            autoRenewal: raw.autoRenewal,
            secretName: raw.secretName,
            renewalStatus: raw.renewalStatus || "idle",
            annotations: raw.annotations,
        };
    });
}

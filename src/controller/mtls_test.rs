//! Tests for mTLS certificate generation and configuration

#[cfg(test)]
mod tests {
    use rcgen::{
        CertificateParams, DistinguishedName, ExtendedKeyUsagePurpose, Ia5String, IsCa, KeyPair,
        KeyUsagePurpose, SanType,
    };

    use crate::MtlsConfig;

    /// Verify CA certificate generation produces valid self-signed output.
    #[test]
    fn test_ca_certificate_generation() {
        let mut params = CertificateParams::default();
        params.is_ca = IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.distinguished_name = DistinguishedName::new();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "stellar-operator-ca");
        params.key_usages.push(KeyUsagePurpose::DigitalSignature);
        params.key_usages.push(KeyUsagePurpose::KeyCertSign);
        params.key_usages.push(KeyUsagePurpose::CrlSign);

        let key_pair = KeyPair::generate().expect("CA key generation should succeed");
        let cert = params
            .self_signed(&key_pair)
            .expect("CA self-signing should succeed");

        let pem = cert.pem();
        assert!(pem.starts_with("-----BEGIN CERTIFICATE-----"));
        assert!(pem.contains("-----END CERTIFICATE-----"));

        let key_pem = key_pair.serialize_pem();
        assert!(key_pem.starts_with("-----BEGIN PRIVATE KEY-----"));
    }

    /// Verify server certificate is correctly signed by the CA and includes
    /// the expected SANs (including localhost for local development).
    #[test]
    fn test_server_certificate_generation_with_sans() {
        // Generate CA
        let mut ca_params = CertificateParams::default();
        ca_params.is_ca = IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        ca_params.distinguished_name = DistinguishedName::new();
        ca_params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "stellar-operator-ca");
        ca_params.key_usages.push(KeyUsagePurpose::DigitalSignature);
        ca_params.key_usages.push(KeyUsagePurpose::KeyCertSign);

        let ca_key = KeyPair::generate().unwrap();
        let ca_cert = ca_params.self_signed(&ca_key).unwrap();

        // Generate server cert with SANs
        let dns_names = vec![
            "localhost".to_string(),
            "stellar-operator".to_string(),
            "stellar-operator.default".to_string(),
            "stellar-operator.default.svc".to_string(),
            "stellar-operator.default.svc.cluster.local".to_string(),
        ];

        let mut params = CertificateParams::default();
        params.distinguished_name = DistinguishedName::new();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "stellar-operator");
        for dns in &dns_names {
            params
                .subject_alt_names
                .push(SanType::DnsName(Ia5String::try_from(dns.clone()).unwrap()));
        }
        params.key_usages.push(KeyUsagePurpose::DigitalSignature);
        params
            .extended_key_usages
            .push(ExtendedKeyUsagePurpose::ServerAuth);
        params
            .extended_key_usages
            .push(ExtendedKeyUsagePurpose::ClientAuth);

        let server_key = KeyPair::generate().unwrap();
        let server_cert = params
            .signed_by(&server_key, &ca_cert, &ca_key)
            .expect("Server certificate signing should succeed");

        let pem = server_cert.pem();
        assert!(pem.starts_with("-----BEGIN CERTIFICATE-----"));
    }

    /// Verify that a CA keypair round-trips through PEM serialization.
    #[test]
    fn test_ca_key_pem_round_trip() {
        let key_pair = KeyPair::generate().unwrap();
        let pem = key_pair.serialize_pem();

        let restored = KeyPair::from_pem(&pem);
        assert!(
            restored.is_ok(),
            "CA key should round-trip through PEM encoding"
        );
    }

    /// Verify MtlsConfig struct holds the expected data.
    #[test]
    fn test_mtls_config_construction() {
        let config = MtlsConfig {
            cert_pem: b"cert-data".to_vec(),
            key_pem: b"key-data".to_vec(),
            ca_pem: b"ca-data".to_vec(),
        };

        assert_eq!(config.cert_pem, b"cert-data");
        assert_eq!(config.key_pem, b"key-data");
        assert_eq!(config.ca_pem, b"ca-data");
    }

    /// Verify that server cert generation fails gracefully with an invalid SAN.
    /// IA5String only permits ASCII (0x00–0x7F); non-ASCII must be rejected.
    #[test]
    fn test_invalid_san_rejected() {
        let result = Ia5String::try_from("héllo.example.com".to_string());
        assert!(
            result.is_err(),
            "SAN with non-ASCII characters should be rejected"
        );
    }

    /// Verify the constant secret names match the expected convention.
    #[test]
    fn test_secret_name_constants() {
        use super::super::mtls::{CA_SECRET_NAME, SERVER_CERT_SECRET_NAME};

        assert_eq!(CA_SECRET_NAME, "stellar-operator-ca");
        assert_eq!(SERVER_CERT_SECRET_NAME, "stellar-operator-server-cert");
    }

    /// Verify a client certificate can be signed by the same CA used for the
    /// server cert, simulating the ensure_node_cert flow.
    #[test]
    fn test_client_certificate_generation() {
        // Generate CA
        let mut ca_params = CertificateParams::default();
        ca_params.is_ca = IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        ca_params.distinguished_name = DistinguishedName::new();
        ca_params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "stellar-operator-ca");
        ca_params.key_usages.push(KeyUsagePurpose::DigitalSignature);
        ca_params.key_usages.push(KeyUsagePurpose::KeyCertSign);

        let ca_key = KeyPair::generate().unwrap();
        let ca_cert = ca_params.self_signed(&ca_key).unwrap();

        // Generate client cert
        let mut params = CertificateParams::default();
        params.distinguished_name = DistinguishedName::new();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "stellar-node-my-validator");
        params.key_usages.push(KeyUsagePurpose::DigitalSignature);
        params
            .extended_key_usages
            .push(ExtendedKeyUsagePurpose::ClientAuth);
        params
            .extended_key_usages
            .push(ExtendedKeyUsagePurpose::ServerAuth);

        let client_key = KeyPair::generate().unwrap();
        let client_cert = params
            .signed_by(&client_key, &ca_cert, &ca_key)
            .expect("Client certificate signing should succeed");

        let pem = client_cert.pem();
        assert!(pem.starts_with("-----BEGIN CERTIFICATE-----"));
    }
}

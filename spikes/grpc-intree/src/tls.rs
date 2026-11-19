//! The TLS configuration: rustls 0.23 on `dcroxide-tlscrypto`, built with
//! `builder_with_provider` as dcroxide requires, set up the way dcrwallet's
//! gRPC listener is (rpcserver.go:272-276, :321-322): the `rpc.cert` /
//! `rpc.key` pair, TLS 1.2 and 1.3, a client certificate required and
//! verified against the `--clientcafile` pool, ALPN `h2` (which grpc-go's
//! credentials add and enforce).
//!
//! The client certificate verifier is rustls's webpki verifier over the
//! pool, not dcroxide-node's private `GoClientCertVerifier`; where the two
//! differ from Go is a finding of the spike (see the ADR).

use std::sync::Arc;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

/// The provider with the key exchange groups in Go's order (X25519, P-256,
/// P-384, P-521) and the RPC listener's verification set.
pub fn provider() -> rustls::crypto::CryptoProvider {
    rustls::crypto::CryptoProvider {
        kx_groups: dcroxide_tlscrypto::ALL_KX_GROUPS.to_vec(),
        signature_verification_algorithms: dcroxide_tlscrypto::RPC_SIG_ALGS,
        ..dcroxide_tlscrypto::default_provider()
    }
}

/// Builds the listener's configuration from PEM bytes: the certificate
/// chain, its key, and the client CA file. Certificates in the CA file that
/// do not parse are skipped, as Go's `AppendCertsFromPEM` skips them; a
/// file from which none is added is an error here, where dcrwallet only
/// warns and then refuses every client.
pub fn server_config(
    cert_pem: &[u8],
    key_pem: &[u8],
    client_ca_pem: &[u8],
) -> Result<rustls::ServerConfig, String> {
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(cert_pem)
        .collect::<Result<_, _>>()
        .map_err(|e| format!("rpc.cert: {e}"))?;
    if certs.is_empty() {
        return Err("rpc.cert: no CERTIFICATE block".into());
    }
    let key = PrivateKeyDer::from_pem_slice(key_pem).map_err(|e| format!("rpc.key: {e}"))?;
    if dcroxide_tlscrypto::sign::key_algorithm(key.secret_der())
        == Some(dcroxide_tlscrypto::sign::KeyAlgorithm::Rsa)
    {
        return Err(format!(
            "rpc.key: {}",
            dcroxide_tlscrypto::sign::RSA_KEY_REFUSED
        ));
    }
    let signing = dcroxide_tlscrypto::sign::load_der(key.secret_der())
        .map_err(|e| format!("rpc.key: {e}"))?;
    let certified = rustls::sign::CertifiedKey::new(certs, signing);
    certified
        .keys_match()
        .map_err(|e| format!("rpc.cert and rpc.key: {e}"))?;

    let mut roots = rustls::RootCertStore::empty();
    for cert in CertificateDer::pem_slice_iter(client_ca_pem).flatten() {
        let _ = roots.add(cert);
    }
    if roots.is_empty() {
        return Err("no certificates added from the client CA file".into());
    }
    let provider = Arc::new(provider());
    let verifier = rustls::server::WebPkiClientVerifier::builder_with_provider(
        Arc::new(roots),
        Arc::clone(&provider),
    )
    .build()
    .map_err(|e| format!("client verifier: {e}"))?;

    let mut config = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
        .map_err(|e| format!("TLS versions: {e}"))?
        .with_client_cert_verifier(verifier)
        .with_cert_resolver(Arc::new(rustls::sign::SingleCertAndKey::from(certified)));
    config.alpn_protocols = vec![b"h2".to_vec()];
    // Go's server chooses the suite in its own order.
    config.ignore_client_order = true;
    // No stateful resumption (Go has none) and no tickets.
    config.session_storage = Arc::new(rustls::server::NoServerSessionStorage {});
    config.send_tls13_tickets = 0;
    Ok(config)
}

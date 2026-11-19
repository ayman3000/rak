//! Every HTTP client rak makes starts here, so they all trust the same
//! certificates: rak's own built-in list (Mozilla's, so a minimal container
//! with no certificates installed still works) plus the system's
//! certificates (and `SSL_CERT_FILE` / `SSL_CERT_DIR`).
//!
//! The system's certificates matter where traffic goes through a proxy that
//! presents its own certificate authority, installed on that machine: a
//! company network, or a benchmark runtime whose egress proxy reads model
//! traffic. With the built-in list alone, every request there fails with
//! "invalid peer certificate: UnknownIssuer".

use std::sync::{Arc, OnceLock};

/// A ureq agent builder with rak's certificates; add timeouts and build.
pub fn builder() -> ureq::AgentBuilder {
    ureq::AgentBuilder::new().tls_connector(Arc::new(WhenNeeded))
}

/// The certificates, read at the first HTTPS connection and not when a
/// client is made: reading the system's takes about a tenth of a second on a
/// Mac, and a chat makes its clients before it shows its prompt.
struct WhenNeeded;

impl ureq::TlsConnector for WhenNeeded {
    fn connect(&self, dns_name: &str, io: Box<dyn ureq::ReadWrite>) -> Result<Box<dyn ureq::ReadWrite>, ureq::Error> {
        tls_config().connect(dns_name, io)
    }
}

/// Reads the certificates on a thread of its own, so the first request does
/// not wait for them either. A chat calls it as it starts.
pub fn warm_up() {
    std::thread::spawn(|| {
        tls_config();
    });
}

fn tls_config() -> Arc<rustls::ClientConfig> {
    static CONFIG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let (roots, _) = root_store(rustls_native_certs::load_native_certs().unwrap_or_default());
            let config = rustls::ClientConfig::builder_with_provider(rustls::crypto::ring::default_provider().into())
                .with_protocol_versions(&[&rustls::version::TLS12, &rustls::version::TLS13])
                .expect("ring supports TLS 1.2 and 1.3")
                .with_root_certificates(roots)
                .with_no_client_auth();
            Arc::new(config)
        })
        .clone()
}

/// Pure — unit-tested. The built-in roots plus `system` ones; returns how
/// many of the system's were usable.
fn root_store(system: Vec<rustls::pki_types::CertificateDer<'static>>) -> (rustls::RootCertStore, usize) {
    let mut roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
    let (added, _ignored) = roots.add_parsable_certificates(system);
    (roots, added)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_roots_stay_when_the_system_has_none_or_bad_ones() {
        let built_in = webpki_roots::TLS_SERVER_ROOTS.len();
        let (roots, added) = root_store(Vec::new());
        assert_eq!((roots.len(), added), (built_in, 0));
        // A file that is not a certificate is skipped, not fatal.
        let (roots, added) = root_store(vec![rustls::pki_types::CertificateDer::from(vec![1, 2, 3])]);
        assert_eq!((roots.len(), added), (built_in, 0));
    }

    #[test]
    fn this_machine_adds_its_certificates() {
        let system = rustls_native_certs::load_native_certs().unwrap_or_default();
        let (roots, added) = root_store(system);
        assert!(roots.len() >= webpki_roots::TLS_SERVER_ROOTS.len());
        assert!(added <= roots.len());
        // The config builds (the crypto provider and versions are valid).
        let _ = tls_config();
        let _ = builder().build();
    }
}

use rcgen::generate_simple_self_signed;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::ServerConfig;
use std::io;

pub fn generate_self_signed_cert(cert_path: &str, key_path: &str) -> io::Result<()> {
    let certified_key =
        generate_simple_self_signed(vec!["localhost".to_string()]).map_err(io::Error::other)?;

    let pem_cert = certified_key.cert.pem();
    let pem_key = certified_key.key_pair.serialize_pem();

    std::fs::write(cert_path, pem_cert)?;
    std::fs::write(key_path, pem_key)?;

    Ok(())
}

pub fn load_tls_config(cert_path: &str, key_path: &str) -> io::Result<ServerConfig> {
    let cert_data = std::fs::read(cert_path)?;
    let key_data = std::fs::read(key_path)?;

    let cert = CertificateDer::from_pem_slice(&cert_data)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, format!("No certificate found: {e}")))?;

    let private_key = PrivateKeyDer::from_pem_slice(&key_data)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, format!("No private key found: {e}")))?;

    let mut config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], private_key)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

    // TDS wraps TLS records in PRELOGIN packets only during the initial
    // handshake. TLS 1.3 session tickets are sent *after* the handshake:
    // rustls would emit these before the transport switches to raw TLS,
    // confusing clients that have already left the TDS PRELOGIN framing.
    // We do not need session resumption for this standalone test server.
    config.send_tls13_tickets = 0;

    Ok(config)
}

use super::*;

use rcgen::generate_simple_self_signed;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn cert_fingerprint_sha256_hex_is_stable() {
    let cert = generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let cert_pem = cert.serialize_pem().unwrap();

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("tls-fingerprint-test-{unique}"));
    std::fs::create_dir_all(&dir).unwrap();
    let cert_path = dir.join("cert.pem");
    std::fs::write(&cert_path, cert_pem).unwrap();

    let fp1 = cert_fingerprint_sha256_hex(cert_path.to_str().unwrap()).unwrap();
    let fp2 = cert_fingerprint_sha256_hex(cert_path.to_str().unwrap()).unwrap();
    assert_eq!(fp1, fp2);
    assert_eq!(fp1.len(), 64);

    std::fs::remove_dir_all(dir).unwrap();
}

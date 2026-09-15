use super::*;

use ed25519_dalek::{Signer, SigningKey};
use http::HeaderValue;

#[test]
fn required_verifier_rejects_missing_signature() {
    let signing = SigningKey::from_bytes(&[7_u8; 32]);
    let key_b64 = STANDARD.encode(signing.verifying_key().to_bytes());
    let verifier = BundleSignatureVerifier::from_config(BundleSignatureConfig {
        required: true,
        public_key_path: None,
    });
    assert!(verifier.is_err());

    let verifier = BundleSignatureVerifier::from_config(BundleSignatureConfig {
        required: true,
        public_key_path: Some(write_temp_key(&key_b64)),
    })
    .unwrap();

    let headers = HeaderMap::new();
    let err = verifier
        .verify_headers_and_body(&headers, b"payload")
        .unwrap_err();
    assert!(matches!(err, BundleSignatureError::MissingHeader));
}

#[test]
fn required_verifier_accepts_valid_signature() {
    let signing = SigningKey::from_bytes(&[9_u8; 32]);
    let key_b64 = STANDARD.encode(signing.verifying_key().to_bytes());
    let verifier = BundleSignatureVerifier::from_config(BundleSignatureConfig {
        required: true,
        public_key_path: Some(write_temp_key(&key_b64)),
    })
    .unwrap();

    let payload = b"bundle-bytes";
    let signature = signing.sign(payload);
    let mut headers = HeaderMap::new();
    headers.insert(
        SIGNATURE_HEADER,
        HeaderValue::from_str(&STANDARD.encode(signature.to_bytes())).unwrap(),
    );

    verifier.verify_headers_and_body(&headers, payload).unwrap();
}

#[test]
fn required_verifier_rejects_invalid_signature() {
    let signing = SigningKey::from_bytes(&[3_u8; 32]);
    let other = SigningKey::from_bytes(&[4_u8; 32]);

    let key_b64 = STANDARD.encode(signing.verifying_key().to_bytes());
    let verifier = BundleSignatureVerifier::from_config(BundleSignatureConfig {
        required: true,
        public_key_path: Some(write_temp_key(&key_b64)),
    })
    .unwrap();

    let payload = b"bundle-bytes";
    let signature = other.sign(payload);

    let mut headers = HeaderMap::new();
    headers.insert(
        SIGNATURE_HEADER,
        HeaderValue::from_str(&STANDARD.encode(signature.to_bytes())).unwrap(),
    );

    let err = verifier
        .verify_headers_and_body(&headers, payload)
        .unwrap_err();
    assert!(matches!(err, BundleSignatureError::InvalidSignature));
}

fn write_temp_key(content: &str) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("bundle-key-test-{unique}"));
    std::fs::create_dir_all(&dir).unwrap();
    let key_path = dir.join("pub.key");
    std::fs::write(&key_path, content).unwrap();
    key_path.to_string_lossy().to_string()
}

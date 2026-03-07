use std::path::Path;

use anyhow::Error;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use ed25519_dalek::pkcs8::DecodePublicKey;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use http::header::HeaderMap;

pub const SIGNATURE_HEADER: &str = "x-bundle-signature-ed25519";

#[derive(Debug, Clone)]
pub struct BundleSignatureConfig {
    pub required: bool,
    pub public_key_path: Option<String>,
}

#[derive(Clone)]
pub struct BundleSignatureVerifier {
    required: bool,
    verifying_key: Option<VerifyingKey>,
}

#[derive(Debug)]
pub enum BundleSignatureError {
    MissingHeader,
    InvalidHeaderEncoding,
    InvalidSignature,
    Misconfigured,
}

impl BundleSignatureVerifier {
    pub fn from_config(config: BundleSignatureConfig) -> Result<Self, Error> {
        let verifying_key = if let Some(path) = config.public_key_path {
            Some(load_verifying_key(&path)?)
        } else {
            None
        };

        if config.required && verifying_key.is_none() {
            return Err(anyhow::anyhow!(
                "bundle signature verification requires a public key path"
            ));
        }

        Ok(Self {
            required: config.required,
            verifying_key,
        })
    }

    pub fn disabled() -> Self {
        Self {
            required: false,
            verifying_key: None,
        }
    }

    pub fn verify_headers_and_body(
        &self,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<(), BundleSignatureError> {
        if !self.required {
            return Ok(());
        }

        let verifying_key = self
            .verifying_key
            .as_ref()
            .ok_or(BundleSignatureError::Misconfigured)?;

        let signature_b64 = headers
            .get(SIGNATURE_HEADER)
            .and_then(|v| v.to_str().ok())
            .ok_or(BundleSignatureError::MissingHeader)?;

        let signature_bytes = STANDARD
            .decode(signature_b64)
            .map_err(|_| BundleSignatureError::InvalidHeaderEncoding)?;

        let signature = Signature::try_from(signature_bytes.as_slice())
            .map_err(|_| BundleSignatureError::InvalidHeaderEncoding)?;

        verifying_key
            .verify(body, &signature)
            .map_err(|_| BundleSignatureError::InvalidSignature)
    }

    pub fn required(&self) -> bool {
        self.required
    }
}

impl BundleSignatureError {
    pub fn as_client_message(&self) -> &'static str {
        match self {
            BundleSignatureError::MissingHeader => "missing x-bundle-signature-ed25519 header",
            BundleSignatureError::InvalidHeaderEncoding => {
                "invalid x-bundle-signature-ed25519 encoding"
            }
            BundleSignatureError::InvalidSignature => "bundle signature verification failed",
            BundleSignatureError::Misconfigured => {
                "bundle signature verification misconfigured on server"
            }
        }
    }
}

fn load_verifying_key(path: &str) -> Result<VerifyingKey, Error> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("failed to read bundle public key '{}': {}", path, e))?;
    parse_verifying_key(content.trim())
        .map_err(|e| anyhow::anyhow!("invalid bundle public key '{}': {}", path, e))
}

fn parse_verifying_key(value: &str) -> Result<VerifyingKey, Error> {
    if value.starts_with("-----BEGIN PUBLIC KEY-----") {
        return VerifyingKey::from_public_key_pem(value)
            .map_err(|e| anyhow::anyhow!("invalid PEM public key: {}", e));
    }

    if let Ok(bytes) = STANDARD.decode(value) {
        if bytes.len() == 32 {
            let arr: [u8; 32] = bytes
                .try_into()
                .map_err(|_| anyhow::anyhow!("public key must be 32 bytes"))?;
            return VerifyingKey::from_bytes(&arr)
                .map_err(|e| anyhow::anyhow!("invalid raw public key: {}", e));
        }
    }

    if let Ok(bytes) = hex_decode(value) {
        if bytes.len() == 32 {
            let arr: [u8; 32] = bytes
                .try_into()
                .map_err(|_| anyhow::anyhow!("public key must be 32 bytes"))?;
            return VerifyingKey::from_bytes(&arr)
                .map_err(|e| anyhow::anyhow!("invalid raw public key: {}", e));
        }
    }

    Err(anyhow::anyhow!(
        "expected PEM, base64(32-byte key), or hex(32-byte key)"
    ))
}

fn hex_decode(input: &str) -> Result<Vec<u8>, Error> {
    if input.len() % 2 != 0 {
        return Err(anyhow::anyhow!("invalid hex length"));
    }

    let mut out = Vec::with_capacity(input.len() / 2);
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let hi = hex_val(bytes[i])?;
        let lo = hex_val(bytes[i + 1])?;
        out.push((hi << 4) | lo);
        i += 2;
    }

    Ok(out)
}

fn hex_val(c: u8) -> Result<u8, Error> {
    match c {
        b'0'..=b'9' => Ok(c - b'0'),
        b'a'..=b'f' => Ok(c - b'a' + 10),
        b'A'..=b'F' => Ok(c - b'A' + 10),
        _ => Err(anyhow::anyhow!("invalid hex character")),
    }
}

pub fn default_public_key_path() -> Option<String> {
    std::env::var("EDGE_RUNTIME_BUNDLE_PUBLIC_KEY_PATH").ok()
}

pub fn config_from_flag(required: bool, public_key_path: Option<String>) -> BundleSignatureConfig {
    BundleSignatureConfig {
        required,
        public_key_path,
    }
}

pub fn ensure_public_key_path_exists(path: &str) -> Result<(), Error> {
    if !Path::new(path).exists() {
        return Err(anyhow::anyhow!(
            "bundle public key path '{}' does not exist",
            path
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
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
}

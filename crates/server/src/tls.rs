use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use anyhow::Error;
use notify::{RecursiveMode, Watcher};
use rustls::ServerConfig;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpStream, UnixStream};
use tokio::sync::RwLock;
use tokio_rustls::server::TlsStream;
use tokio_rustls::TlsAcceptor;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::TlsConfig;

/// Shared TLS acceptor that can be swapped at runtime.
#[derive(Clone)]
pub struct DynamicTlsAcceptor {
    current: Arc<RwLock<TlsAcceptor>>,
}

impl DynamicTlsAcceptor {
    fn new(initial: TlsAcceptor) -> Self {
        Self {
            current: Arc::new(RwLock::new(initial)),
        }
    }

    pub async fn accept(&self, stream: TcpStream) -> Result<TlsStream<TcpStream>, io::Error> {
        let acceptor = self.current.read().await.clone();
        acceptor.accept(stream).await
    }

    async fn replace(&self, next: TlsAcceptor) {
        *self.current.write().await = next;
    }
}

/// Build a TLS acceptor from certificate and key files.
pub fn build_tls_acceptor(config: &TlsConfig) -> Result<TlsAcceptor, Error> {
    let cert_pem = std::fs::read(&config.cert_path)
        .map_err(|e| anyhow::anyhow!("failed to read TLS cert '{}': {}", config.cert_path, e))?;
    let key_pem = std::fs::read(&config.key_path)
        .map_err(|e| anyhow::anyhow!("failed to read TLS key '{}': {}", config.key_path, e))?;

    let certs = rustls_pemfile::certs(&mut io::BufReader::new(cert_pem.as_slice()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| anyhow::anyhow!("failed to parse TLS certificates: {}", e))?;

    let key = rustls_pemfile::private_key(&mut io::BufReader::new(key_pem.as_slice()))
        .map_err(|e| anyhow::anyhow!("failed to parse TLS private key: {}", e))?
        .ok_or_else(|| anyhow::anyhow!("no private key found in TLS key file"))?;

    let mut tls_config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| anyhow::anyhow!("invalid TLS config: {}", e))?;

    tls_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

    Ok(TlsAcceptor::from(Arc::new(tls_config)))
}

/// Build a TLS acceptor with background file watcher to hot-reload cert/key.
pub fn build_dynamic_tls_acceptor(
    config: TlsConfig,
    shutdown: CancellationToken,
    listener_name: &'static str,
) -> Result<DynamicTlsAcceptor, Error> {
    let initial = build_tls_acceptor(&config)?;
    let dynamic = DynamicTlsAcceptor::new(initial);

    let initial_fp = cert_fingerprint_sha256_hex(&config.cert_path)?;
    info!(
        "{} TLS loaded (fingerprint_sha256={})",
        listener_name, initial_fp
    );

    spawn_tls_reload_watcher(config, dynamic.clone(), shutdown, listener_name)?;

    Ok(dynamic)
}

fn spawn_tls_reload_watcher(
    config: TlsConfig,
    dynamic: DynamicTlsAcceptor,
    shutdown: CancellationToken,
    listener_name: &'static str,
) -> Result<(), Error> {
    let cert_path = PathBuf::from(config.cert_path.clone());
    let key_path = PathBuf::from(config.key_path.clone());
    let cert_watch_path = cert_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| cert_path.clone());
    let key_watch_path = key_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| key_path.clone());

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<notify::Result<notify::Event>>();

    let mut watcher = notify::recommended_watcher(move |event| {
        let _ = tx.send(event);
    })
    .map_err(|e| anyhow::anyhow!("failed to create TLS watcher: {}", e))?;

    watcher
        .watch(&cert_watch_path, RecursiveMode::NonRecursive)
        .map_err(|e| {
            anyhow::anyhow!(
                "failed to watch TLS cert path '{}': {}",
                cert_watch_path.display(),
                e
            )
        })?;

    if key_watch_path != cert_watch_path {
        watcher
            .watch(&key_watch_path, RecursiveMode::NonRecursive)
            .map_err(|e| {
                anyhow::anyhow!(
                    "failed to watch TLS key path '{}': {}",
                    key_watch_path.display(),
                    e
                )
            })?;
    }

    info!(
        "{} TLS hot-reload watcher enabled (cert='{}', key='{}')",
        listener_name,
        cert_path.display(),
        key_path.display()
    );

    tokio::spawn(async move {
        let _watcher = watcher;

        loop {
            tokio::select! {
                _ = shutdown.cancelled() => {
                    info!("{} TLS hot-reload watcher stopping", listener_name);
                    break;
                }
                maybe_event = rx.recv() => {
                    let Some(event_result) = maybe_event else {
                        break;
                    };

                    let event = match event_result {
                        Ok(event) => event,
                        Err(err) => {
                            warn!("{} TLS watcher error: {}", listener_name, err);
                            continue;
                        }
                    };

                    if !event_touches_path(&event, &cert_path, &key_path) {
                        continue;
                    }

                    if let Err(err) = reload_tls_with_retry(&config, &dynamic, listener_name).await {
                        error!("{} TLS reload failed: {}", listener_name, err);
                    }
                }
            }
        }
    });

    Ok(())
}

fn event_touches_path(event: &notify::Event, cert_path: &Path, key_path: &Path) -> bool {
    event
        .paths
        .iter()
        .any(|event_path| paths_equal(event_path, cert_path) || paths_equal(event_path, key_path))
}

fn paths_equal(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }

    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => false,
    }
}

async fn reload_tls_with_retry(
    config: &TlsConfig,
    dynamic: &DynamicTlsAcceptor,
    listener_name: &'static str,
) -> Result<(), Error> {
    let mut last_err = None;

    for attempt in 1..=5_u8 {
        match build_tls_acceptor(config) {
            Ok(next) => {
                dynamic.replace(next).await;
                let fp = cert_fingerprint_sha256_hex(&config.cert_path)
                    .unwrap_or_else(|e| format!("unavailable ({})", e));
                info!(
                    "{} TLS certificate reloaded (fingerprint_sha256={})",
                    listener_name, fp
                );
                return Ok(());
            }
            Err(err) => {
                last_err = Some(err);
                if attempt < 5 {
                    tokio::time::sleep(Duration::from_millis(150 * u64::from(attempt))).await;
                }
            }
        }
    }

    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("unknown TLS reload error")))
}

fn cert_fingerprint_sha256_hex(cert_path: &str) -> Result<String, Error> {
    let cert_pem = std::fs::read(cert_path)
        .map_err(|e| anyhow::anyhow!("failed to read TLS cert '{}': {}", cert_path, e))?;
    let certs = rustls_pemfile::certs(&mut io::BufReader::new(cert_pem.as_slice()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| anyhow::anyhow!("failed to parse TLS certificates: {}", e))?;

    let first = certs
        .first()
        .ok_or_else(|| anyhow::anyhow!("no certificates found in TLS cert file"))?;
    let digest = Sha256::digest(first.as_ref());
    Ok(hex_encode_lower(&digest))
}

fn hex_encode_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// Stream that may be TCP (plain or TLS) or Unix socket.
pub enum MaybeHttpsStream {
    /// Plain TCP stream.
    TcpPlain(TcpStream),
    /// TLS-encrypted TCP stream.
    TcpTls(TlsStream<TcpStream>),
    /// Unix domain socket stream.
    Unix(UnixStream),
}

impl AsyncRead for MaybeHttpsStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            MaybeHttpsStream::TcpPlain(s) => Pin::new(s).poll_read(cx, buf),
            MaybeHttpsStream::TcpTls(s) => Pin::new(s).poll_read(cx, buf),
            MaybeHttpsStream::Unix(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for MaybeHttpsStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            MaybeHttpsStream::TcpPlain(s) => Pin::new(s).poll_write(cx, buf),
            MaybeHttpsStream::TcpTls(s) => Pin::new(s).poll_write(cx, buf),
            MaybeHttpsStream::Unix(s) => Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            MaybeHttpsStream::TcpPlain(s) => Pin::new(s).poll_flush(cx),
            MaybeHttpsStream::TcpTls(s) => Pin::new(s).poll_flush(cx),
            MaybeHttpsStream::Unix(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            MaybeHttpsStream::TcpPlain(s) => Pin::new(s).poll_shutdown(cx),
            MaybeHttpsStream::TcpTls(s) => Pin::new(s).poll_shutdown(cx),
            MaybeHttpsStream::Unix(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

#[cfg(test)]
mod tests {
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
}

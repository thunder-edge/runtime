use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use anyhow::Error;
use rustls::ServerConfig;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::server::TlsStream;
use tokio_rustls::TlsAcceptor;

use crate::TlsConfig;

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

/// Stream that may or may not be wrapped in TLS.
pub enum MaybeHttpsStream {
    Plain(TcpStream),
    Tls(TlsStream<TcpStream>),
}

impl AsyncRead for MaybeHttpsStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            MaybeHttpsStream::Plain(s) => Pin::new(s).poll_read(cx, buf),
            MaybeHttpsStream::Tls(s) => Pin::new(s).poll_read(cx, buf),
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
            MaybeHttpsStream::Plain(s) => Pin::new(s).poll_write(cx, buf),
            MaybeHttpsStream::Tls(s) => Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            MaybeHttpsStream::Plain(s) => Pin::new(s).poll_flush(cx),
            MaybeHttpsStream::Tls(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            MaybeHttpsStream::Plain(s) => Pin::new(s).poll_shutdown(cx),
            MaybeHttpsStream::Tls(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

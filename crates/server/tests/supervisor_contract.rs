use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use edge_server::bundle_signature::BundleSignatureConfig;
use edge_server::{
    run_dual_server_with_runtime_state, AdminListenerConfig, BodyLimitsConfig, DualServerConfig,
    IngressListenerConfig, IngressListenerType, RuntimeState, TcpListenerSource,
};
use functions::registry::FunctionRegistry;
use runtime_core::isolate::IsolateConfig;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

async fn reserve_addresses() -> (SocketAddr, SocketAddr) {
    let admin_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to reserve admin address");
    let ingress_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to reserve ingress address");
    let admin_address = admin_listener
        .local_addr()
        .expect("failed to read admin address");
    let ingress_address = ingress_listener
        .local_addr()
        .expect("failed to read ingress address");
    drop(ingress_listener);
    drop(admin_listener);
    (admin_address, ingress_address)
}

async fn wait_for_listener(address: SocketAddr) {
    for _ in 0..100 {
        if TcpStream::connect(address).await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("listener did not start: {address}");
}

async fn request(address: SocketAddr, request: &str) -> String {
    let mut stream = TcpStream::connect(address)
        .await
        .expect("failed to connect to test server");
    stream
        .write_all(request.as_bytes())
        .await
        .expect("failed to write request");
    let mut response = Vec::new();
    tokio::time::timeout(Duration::from_secs(2), stream.read_to_end(&mut response))
        .await
        .expect("timed out waiting for response")
        .expect("failed to read response");
    String::from_utf8(response).expect("server response was not UTF-8")
}

#[tokio::test]
async fn phase_a_b_endpoints_share_lifecycle_state() {
    let (admin_address, ingress_address) = reserve_addresses().await;
    let state = RuntimeState::with_revision("integration-revision");
    let shutdown = CancellationToken::new();
    let registry = Arc::new(FunctionRegistry::new(
        shutdown.clone(),
        IsolateConfig::default(),
    ));

    let config = DualServerConfig {
        admin: AdminListenerConfig {
            listener_type: TcpListenerSource::Bind(admin_address),
            api_key: None,
            tls: None,
            body_limits: BodyLimitsConfig::default(),
            bundle_signature: BundleSignatureConfig {
                required: false,
                public_key_path: None,
            },
        },
        ingress: IngressListenerConfig {
            listener_type: IngressListenerType::Tcp(TcpListenerSource::Bind(ingress_address)),
            tls: None,
            rate_limit_rps: None,
            body_limits: BodyLimitsConfig::default(),
        },
        graceful_exit_deadline_secs: 1,
        max_connections: 32,
    };

    let server = tokio::spawn({
        let shutdown = shutdown.clone();
        let state = state.clone();
        let registry = registry.clone();
        async move { run_dual_server_with_runtime_state(config, registry, shutdown, state).await }
    });

    wait_for_listener(admin_address).await;
    wait_for_listener(ingress_address).await;

    let not_ready = request(
        admin_address,
        "GET /health?ready=1 HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(not_ready.starts_with("HTTP/1.1 503"));

    state.mark_initial_reconcile_complete();
    let ready = request(
        admin_address,
        "GET /health?ready=1 HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(ready.starts_with("HTTP/1.1 200"));

    let state_response = request(
        admin_address,
        "GET /_internal/state HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(state_response.starts_with("HTTP/1.1 200"));
    assert!(state_response.contains("\"runtime_revision\":\"integration-revision\""));
    assert!(state_response.contains("\"routing_epoch\":0"));
    assert!(state_response.contains("\"pid\":"));
    assert!(state_response.contains("\"observed_saturation\":"));

    let first_drain = request(
        admin_address,
        "POST /_internal/drain HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(first_drain.starts_with("HTTP/1.1 202"));

    let rejected = request(
        ingress_address,
        "GET /missing HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(rejected.starts_with("HTTP/1.1 503"));
    assert!(rejected.contains("ERR_RUNTIME_DRAINING"));

    let second_drain = request(
        admin_address,
        "POST /_internal/drain HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(second_drain.starts_with("HTTP/1.1 202"));

    // The admin plane remains available after the idempotent drain request.
    let state_after_drain = request(
        admin_address,
        "GET /_internal/state HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(state_after_drain.starts_with("HTTP/1.1 200"));
    assert!(state_after_drain.contains("\"draining\":true"));

    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(3), server)
        .await
        .expect("server did not stop")
        .expect("server task failed")
        .expect("server returned an error");
}

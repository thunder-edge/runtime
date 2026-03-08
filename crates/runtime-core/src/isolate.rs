use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;

use anyhow::Error;
use deno_core::ModuleSpecifier;
use http::response::Parts;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::ssrf::SsrfConfig;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutgoingProxyConfig {
    #[serde(default)]
    pub http_proxy: Option<String>,
    #[serde(default)]
    pub https_proxy: Option<String>,
    #[serde(default)]
    pub tcp_proxy: Option<String>,
    #[serde(default)]
    pub http_no_proxy: Vec<String>,
    #[serde(default)]
    pub https_no_proxy: Vec<String>,
    #[serde(default)]
    pub tcp_no_proxy: Vec<String>,
}

/// Configuration for creating a new function isolate.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IsolateConfig {
    /// Maximum heap size in bytes (0 = unlimited).
    #[serde(default = "default_max_heap")]
    pub max_heap_size_bytes: usize,

    /// CPU time limit per request in milliseconds (0 = unlimited).
    #[serde(default = "default_cpu_time")]
    pub cpu_time_limit_ms: u64,

    /// Wall clock timeout per request in milliseconds (0 = unlimited).
    #[serde(default = "default_wall_clock")]
    pub wall_clock_timeout_ms: u64,

    /// Optional V8 inspector protocol port. When set, this isolate exposes CDP on localhost.
    #[serde(default)]
    pub inspect_port: Option<u16>,

    /// If true, waits for inspector session and breaks on next statement.
    #[serde(default)]
    pub inspect_brk: bool,

    /// If true, allows inspector server binding on all interfaces.
    /// Default keeps inspector restricted to localhost for safety.
    #[serde(default)]
    pub inspect_allow_remote: bool,

    /// If true, inline source maps from eszip modules into loaded JS.
    #[serde(default = "default_enable_source_maps")]
    pub enable_source_maps: bool,

    /// SSRF protection configuration.
    #[serde(default)]
    pub ssrf_config: SsrfConfig,

    /// If true, user `console.*` logs are printed to runtime stdout/stderr.
    /// If false, logs are captured by the isolate log collector only.
    #[serde(default = "default_print_isolate_logs")]
    pub print_isolate_logs: bool,

    /// VFS total writable quota in bytes (default: 10 MiB).
    #[serde(default = "default_vfs_total_quota_bytes")]
    pub vfs_total_quota_bytes: usize,

    /// VFS max file size in bytes (default: 5 MiB).
    #[serde(default = "default_vfs_max_file_bytes")]
    pub vfs_max_file_bytes: usize,

    /// DNS-over-HTTPS resolver endpoint used by node:dns compat layer.
    #[serde(default = "default_dns_doh_endpoint")]
    pub dns_doh_endpoint: String,

    /// Maximum DNS answers returned per query.
    #[serde(default = "default_dns_max_answers")]
    pub dns_max_answers: usize,

    /// Timeout for DNS resolver requests in milliseconds.
    #[serde(default = "default_dns_timeout_ms")]
    pub dns_timeout_ms: u64,

    /// Default max output length for node:zlib one-shot operations.
    #[serde(default = "default_zlib_max_output_length")]
    pub zlib_max_output_length: usize,

    /// Default max input length for node:zlib one-shot operations.
    #[serde(default = "default_zlib_max_input_length")]
    pub zlib_max_input_length: usize,

    /// Default operation timeout in milliseconds for node:zlib one-shot operations.
    #[serde(default = "default_zlib_operation_timeout_ms")]
    pub zlib_operation_timeout_ms: u64,

    /// Maximum outbound network requests per execution (0 = unlimited).
    #[serde(default = "default_egress_max_requests_per_execution")]
    pub egress_max_requests_per_execution: usize,

    /// Enables context-aware scheduling metadata in request dispatch paths.
    #[serde(default)]
    pub context_pool_enabled: bool,

    /// Maximum logical contexts tracked per isolate for scheduler decisions.
    #[serde(default = "default_max_contexts_per_isolate")]
    pub max_contexts_per_isolate: usize,

    /// Maximum active requests allowed per logical context.
    #[serde(default = "default_max_active_requests_per_context")]
    pub max_active_requests_per_context: usize,
}

fn default_max_heap() -> usize {
    128 * 1024 * 1024 // 128 MiB
}

fn default_cpu_time() -> u64 {
    50_000
}

fn default_wall_clock() -> u64 {
    60_000
}

fn default_enable_source_maps() -> bool {
    true
}

fn default_print_isolate_logs() -> bool {
    true
}

fn default_vfs_total_quota_bytes() -> usize {
    10 * 1024 * 1024
}

fn default_vfs_max_file_bytes() -> usize {
    5 * 1024 * 1024
}

fn default_dns_doh_endpoint() -> String {
    "https://1.1.1.1/dns-query".to_string()
}

fn default_dns_max_answers() -> usize {
    16
}

fn default_dns_timeout_ms() -> u64 {
    2000
}

fn default_zlib_max_output_length() -> usize {
    16 * 1024 * 1024
}

fn default_zlib_max_input_length() -> usize {
    8 * 1024 * 1024
}

fn default_zlib_operation_timeout_ms() -> u64 {
    250
}

fn default_egress_max_requests_per_execution() -> usize {
    0
}

fn default_max_contexts_per_isolate() -> usize {
    8
}

fn default_max_active_requests_per_context() -> usize {
    1
}

impl Default for IsolateConfig {
    fn default() -> Self {
        Self {
            max_heap_size_bytes: default_max_heap(),
            cpu_time_limit_ms: default_cpu_time(),
            wall_clock_timeout_ms: default_wall_clock(),
            inspect_port: None,
            inspect_brk: false,
            inspect_allow_remote: false,
            enable_source_maps: default_enable_source_maps(),
            ssrf_config: SsrfConfig::default(),
            print_isolate_logs: default_print_isolate_logs(),
            vfs_total_quota_bytes: default_vfs_total_quota_bytes(),
            vfs_max_file_bytes: default_vfs_max_file_bytes(),
            dns_doh_endpoint: default_dns_doh_endpoint(),
            dns_max_answers: default_dns_max_answers(),
            dns_timeout_ms: default_dns_timeout_ms(),
            zlib_max_output_length: default_zlib_max_output_length(),
            zlib_max_input_length: default_zlib_max_input_length(),
            zlib_operation_timeout_ms: default_zlib_operation_timeout_ms(),
            egress_max_requests_per_execution: default_egress_max_requests_per_execution(),
            context_pool_enabled: false,
            max_contexts_per_isolate: default_max_contexts_per_isolate(),
            max_active_requests_per_context: default_max_active_requests_per_context(),
        }
    }
}

/// A request message sent to an isolate for processing.
pub struct IsolateRequest {
    /// Logical function name target for routing/scheduling observability.
    pub function_name: Option<String>,
    /// Optional context target within the isolate.
    pub context_id: Option<String>,
    /// The HTTP request to process.
    pub request: http::Request<bytes::Bytes>,
    /// Channel to send the response back on.
    pub response_tx: oneshot::Sender<Result<IsolateResponse, Error>>,
}

/// Streaming body channel returned by an isolate.
pub type ResponseChunkReceiver = mpsc::UnboundedReceiver<Result<bytes::Bytes, Error>>;

/// Response body variants produced by an isolate.
pub enum IsolateResponseBody {
    Full(bytes::Bytes),
    Stream(ResponseChunkReceiver),
}

/// HTTP response returned by an isolate.
pub struct IsolateResponse {
    pub parts: Parts,
    pub body: IsolateResponseBody,
}

impl IsolateResponse {
    pub fn from_full_response(response: http::Response<bytes::Bytes>) -> Self {
        let (parts, body) = response.into_parts();
        Self {
            parts,
            body: IsolateResponseBody::Full(body),
        }
    }
}

/// Handle to communicate with a running isolate.
#[derive(Clone)]
pub struct IsolateHandle {
    /// Send requests to the isolate's event loop.
    pub request_tx: Arc<Mutex<Option<mpsc::UnboundedSender<IsolateRequest>>>>,
    /// Signal the isolate to shut down.
    pub shutdown: CancellationToken,
    /// Unique ID for this isolate instance.
    pub id: Uuid,
    /// Flag indicating if the isolate is still alive.
    /// Set to false when isolate thread exits (panic, error, or normal shutdown).
    pub alive: Arc<AtomicBool>,
}

impl IsolateHandle {
    /// Check if the isolate is still alive and capable of handling requests.
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    /// Send a request and await the response.
    /// Returns an error if the isolate is not alive or the channel is closed.
    pub async fn send_request(
        &self,
        request: http::Request<bytes::Bytes>,
    ) -> Result<IsolateResponse, Error> {
        self.send_routed_request(request, None, None).await
    }

    /// Send a request with logical routing metadata.
    pub async fn send_routed_request(
        &self,
        request: http::Request<bytes::Bytes>,
        function_name: Option<String>,
        context_id: Option<String>,
    ) -> Result<IsolateResponse, Error> {
        // Check if isolate is alive before attempting to send
        if !self.is_alive() {
            return Err(anyhow::anyhow!(
                "isolate is not alive (crashed or shutdown)"
            ));
        }

        let (response_tx, response_rx) = oneshot::channel();

        let sender = self
            .request_tx
            .lock()
            .map_err(|_| anyhow::anyhow!("isolate request channel lock poisoned"))?
            .clone()
            .ok_or_else(|| anyhow::anyhow!("isolate request channel closed"))?;

        sender
            .send(IsolateRequest {
                function_name,
                context_id,
                request,
                response_tx,
            })
            .map_err(|_| anyhow::anyhow!("isolate request channel closed"))?;

        response_rx.await?
    }

    /// Mark the isolate as dead. Called when the isolate thread exits.
    pub fn mark_dead(&self) {
        self.alive.store(false, Ordering::SeqCst);
    }

    /// Mark the isolate as alive (used after successful auto-restart).
    pub fn mark_alive(&self) {
        self.alive.store(true, Ordering::SeqCst);
    }

    /// Replace the underlying request sender (used during auto-restart).
    pub fn replace_request_tx(&self, sender: mpsc::UnboundedSender<IsolateRequest>) {
        if let Ok(mut guard) = self.request_tx.lock() {
            *guard = Some(sender);
        }
    }

    /// Close request sender so pending/new requests fail fast.
    pub fn close_request_tx(&self) {
        if let Ok(mut guard) = self.request_tx.lock() {
            *guard = None;
        }
    }

    /// Returns true when the request channel is closed or unavailable.
    pub fn is_request_channel_closed(&self) -> bool {
        let guard = match self.request_tx.lock() {
            Ok(g) => g,
            Err(_) => return true,
        };

        match guard.as_ref() {
            Some(sender) => sender.is_closed(),
            None => true,
        }
    }
}

/// Module specifier for the function's entrypoint.
pub fn default_entrypoint() -> ModuleSpecifier {
    ModuleSpecifier::parse("file:///src/index.ts").unwrap()
}

/// Determine the root specifier from an eszip bundle.
///
/// Convention: use the first specifier found, or fall back to `file:///src/index.ts`.
pub fn determine_root_specifier(eszip: &eszip::EszipV2) -> Result<ModuleSpecifier, Error> {
    let specifiers = eszip.specifiers();
    if let Some(first) = specifiers.first() {
        ModuleSpecifier::parse(first)
            .map_err(|e| anyhow::anyhow!("invalid root specifier '{}': {}", first, e))
    } else {
        Ok(default_entrypoint())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolate_config_defaults() {
        let config = IsolateConfig::default();
        assert_eq!(config.max_heap_size_bytes, 128 * 1024 * 1024);
        assert_eq!(config.cpu_time_limit_ms, 50_000);
        assert_eq!(config.wall_clock_timeout_ms, 60_000);
        assert_eq!(config.inspect_port, None);
        assert!(!config.inspect_brk);
        assert!(config.enable_source_maps);
        assert!(config.ssrf_config.enabled);
        assert!(config.print_isolate_logs);
        assert_eq!(config.vfs_total_quota_bytes, 10 * 1024 * 1024);
        assert_eq!(config.vfs_max_file_bytes, 5 * 1024 * 1024);
        assert_eq!(config.dns_doh_endpoint, "https://1.1.1.1/dns-query");
        assert_eq!(config.dns_max_answers, 16);
        assert_eq!(config.dns_timeout_ms, 2000);
        assert_eq!(config.egress_max_requests_per_execution, 0);
        assert!(!config.context_pool_enabled);
        assert_eq!(config.max_contexts_per_isolate, 8);
        assert_eq!(config.max_active_requests_per_context, 1);
    }

    #[test]
    fn isolate_config_serde_defaults() {
        let config: IsolateConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(config.max_heap_size_bytes, 128 * 1024 * 1024);
        assert_eq!(config.cpu_time_limit_ms, 50_000);
        assert_eq!(config.wall_clock_timeout_ms, 60_000);
        assert_eq!(config.inspect_port, None);
        assert!(!config.inspect_brk);
        assert!(config.enable_source_maps);
        assert!(config.ssrf_config.enabled);
        assert!(config.print_isolate_logs);
        assert_eq!(config.vfs_total_quota_bytes, 10 * 1024 * 1024);
        assert_eq!(config.vfs_max_file_bytes, 5 * 1024 * 1024);
        assert_eq!(config.dns_doh_endpoint, "https://1.1.1.1/dns-query");
        assert_eq!(config.dns_max_answers, 16);
        assert_eq!(config.dns_timeout_ms, 2000);
        assert_eq!(config.egress_max_requests_per_execution, 0);
        assert!(!config.context_pool_enabled);
        assert_eq!(config.max_contexts_per_isolate, 8);
        assert_eq!(config.max_active_requests_per_context, 1);
    }

    #[test]
    fn isolate_config_serde_custom() {
        let json = r#"{"max_heap_size_bytes":999,"cpu_time_limit_ms":100,"wall_clock_timeout_ms":200,"inspect_port":9333,"inspect_brk":true,"enable_source_maps":false}"#;
        let config: IsolateConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.max_heap_size_bytes, 999);
        assert_eq!(config.cpu_time_limit_ms, 100);
        assert_eq!(config.wall_clock_timeout_ms, 200);
        assert_eq!(config.inspect_port, Some(9333));
        assert!(config.inspect_brk);
        assert!(!config.enable_source_maps);
    }

    #[test]
    fn isolate_config_serializes() {
        let config = IsolateConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("\"max_heap_size_bytes\""));
        assert!(json.contains("\"cpu_time_limit_ms\""));
        assert!(json.contains("\"wall_clock_timeout_ms\""));
        assert!(json.contains("\"inspect_port\""));
        assert!(json.contains("\"inspect_brk\""));
        assert!(json.contains("\"enable_source_maps\""));
        assert!(json.contains("\"ssrf_config\""));
        assert!(json.contains("\"print_isolate_logs\""));
        assert!(json.contains("\"vfs_total_quota_bytes\""));
        assert!(json.contains("\"vfs_max_file_bytes\""));
        assert!(json.contains("\"dns_doh_endpoint\""));
        assert!(json.contains("\"dns_max_answers\""));
        assert!(json.contains("\"dns_timeout_ms\""));
        assert!(json.contains("\"egress_max_requests_per_execution\""));
        assert!(json.contains("\"context_pool_enabled\""));
        assert!(json.contains("\"max_contexts_per_isolate\""));
        assert!(json.contains("\"max_active_requests_per_context\""));
    }

    #[test]
    fn default_entrypoint_is_index_ts() {
        let spec = default_entrypoint();
        assert_eq!(spec.as_str(), "file:///src/index.ts");
    }
}

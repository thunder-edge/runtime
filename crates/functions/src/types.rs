use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use bytes::Bytes;
use chrono::{DateTime, Utc};
use runtime_core::isolate::{IsolateConfig, IsolateHandle};
use serde::{Deserialize, Serialize};

/// Bundle format for function deployment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BundleFormat {
    /// Traditional eszip bundle (modules loaded at startup).
    Eszip,
    /// V8 snapshot bundle (pre-initialized isolate state).
    Snapshot,
}

impl std::fmt::Display for BundleFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BundleFormat::Eszip => write!(f, "eszip"),
            BundleFormat::Snapshot => write!(f, "snapshot"),
        }
    }
}

impl Default for BundleFormat {
    fn default() -> Self {
        BundleFormat::Eszip
    }
}

/// Packaged bundle containing either snapshot or eszip (with optional fallback).
#[derive(Debug, Serialize, Deserialize)]
pub struct BundlePackage {
    /// Format of the primary bundle (snapshot or eszip).
    pub format: BundleFormat,
    /// V8 version that the snapshot was created with (for compatibility checking).
    pub v8_version: String,
    /// Primary bundle data (snapshot or eszip bytes).
    pub bundle: Vec<u8>,
    /// Fallback eszip bytes (used if snapshot fails to load).
    pub fallback_eszip: Option<Vec<u8>>,
}

impl BundlePackage {
    /// Create a new snapshot bundle with eszip fallback.
    pub fn snapshot_with_fallback(snapshot: Vec<u8>, fallback_eszip: Vec<u8>) -> Self {
        Self {
            format: BundleFormat::Snapshot,
            v8_version: get_v8_version().to_string(),
            bundle: snapshot,
            fallback_eszip: Some(fallback_eszip),
        }
    }

    /// Create a new eszip-only bundle.
    pub fn eszip_only(eszip: Vec<u8>) -> Self {
        Self {
            format: BundleFormat::Eszip,
            v8_version: get_v8_version().to_string(),
            bundle: eszip,
            fallback_eszip: None,
        }
    }

    /// Check if V8 version matches (for snapshot compatibility).
    pub fn is_v8_compatible(&self) -> bool {
        self.v8_version == get_v8_version()
    }
}

/// Get current V8 version from deno_core.
fn get_v8_version() -> &'static str {
    deno_core::v8::VERSION_STRING
}

/// Status of a deployed function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FunctionStatus {
    /// Function is being loaded (eszip parsed, isolate booting).
    Loading,
    /// Function is running and ready to handle requests.
    Running,
    /// Function encountered an error during boot or at runtime.
    Error,
    /// Function is shutting down (being removed or reloaded).
    ShuttingDown,
}

/// Per-function request/response metrics.
#[derive(Debug)]
pub struct FunctionMetrics {
    pub total_requests: AtomicU64,
    pub active_requests: AtomicU64,
    pub total_errors: AtomicU64,
    pub total_cpu_time_ms: AtomicU64,
    pub cold_start_count: AtomicU64,              // Total de cold starts (inicializações)
    pub total_cold_start_time_ms: AtomicU64,      // Tempo acumulado de cold start (ms)
    pub total_warm_start_time_ms: AtomicU64,      // Tempo acumulado de requisições após boot (ms)
}

impl Default for FunctionMetrics {
    fn default() -> Self {
        Self {
            total_requests: AtomicU64::new(0),
            active_requests: AtomicU64::new(0),
            total_errors: AtomicU64::new(0),
            total_cpu_time_ms: AtomicU64::new(0),
            cold_start_count: AtomicU64::new(0),
            total_cold_start_time_ms: AtomicU64::new(0),
            total_warm_start_time_ms: AtomicU64::new(0),
        }
    }
}

impl FunctionMetrics {
    pub fn snapshot(&self) -> FunctionMetricsSnapshot {
        let total_requests = self.total_requests.load(Ordering::Relaxed);
        let total_warm_start_time_ms = self.total_warm_start_time_ms.load(Ordering::Relaxed);
        let cold_start_count = self.cold_start_count.load(Ordering::Relaxed);
        let total_cold_start_time_ms = self.total_cold_start_time_ms.load(Ordering::Relaxed);

        let avg_warm_request_ms = if total_requests > 0 {
            total_warm_start_time_ms / total_requests
        } else {
            0
        };

        let avg_cold_start_ms = if cold_start_count > 0 {
            total_cold_start_time_ms / cold_start_count
        } else {
            0
        };

        FunctionMetricsSnapshot {
            total_requests,
            active_requests: self.active_requests.load(Ordering::Relaxed),
            total_errors: self.total_errors.load(Ordering::Relaxed),
            total_cpu_time_ms: self.total_cpu_time_ms.load(Ordering::Relaxed),
            cold_starts: cold_start_count,
            avg_cold_start_ms,
            total_cold_start_time_ms,
            total_warm_start_time_ms,
            avg_warm_request_ms,
        }
    }
}

/// Serializable snapshot of function metrics.
#[derive(Debug, Clone, Serialize)]
pub struct FunctionMetricsSnapshot {
    pub total_requests: u64,
    pub active_requests: u64,
    pub total_errors: u64,
    pub total_cpu_time_ms: u64,
    pub cold_starts: u64,                       // Total de cold starts
    pub avg_cold_start_ms: u64,                 // Média de cold start (ms)
    pub total_cold_start_time_ms: u64,          // Tempo total de cold start (ms)
    pub total_warm_start_time_ms: u64,          // Tempo total de requisições após boot (ms)
    pub avg_warm_request_ms: u64,               // Média de requisição warm start (ms)
}

/// A single deployed function entry.
pub struct FunctionEntry {
    /// Unique name (used as path prefix for routing).
    pub name: String,
    /// The raw eszip bundle bytes (kept for hot-reload).
    pub eszip_bytes: Bytes,
    /// The original bundle format used.
    pub bundle_format: BundleFormat,
    /// Handle to the running isolate (None if loading/error).
    pub isolate_handle: Option<IsolateHandle>,
    /// Stop signal for the inspector listener thread (if inspector is active).
    pub inspector_stop: Option<Arc<AtomicBool>>,
    /// Current status.
    pub status: FunctionStatus,
    /// Runtime configuration.
    pub config: IsolateConfig,
    /// Metrics.
    pub metrics: Arc<FunctionMetrics>,
    /// When the function was deployed.
    pub created_at: DateTime<Utc>,
    /// When it was last updated.
    pub updated_at: DateTime<Utc>,
    /// Last error message (if status == Error).
    pub last_error: Option<String>,
}

impl FunctionEntry {
    /// Create a serializable info response from this entry.
    pub fn to_info(&self) -> FunctionInfo {
        FunctionInfo {
            name: self.name.clone(),
            status: self.status,
            metrics: self.metrics.snapshot(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            last_error: self.last_error.clone(),
        }
    }
}

/// API response for function info.
#[derive(Debug, Serialize)]
pub struct FunctionInfo {
    pub name: String,
    pub status: FunctionStatus,
    pub metrics: FunctionMetricsSnapshot,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_error: Option<String>,
}

/// API payload for deploying a function.
#[derive(Debug, Deserialize)]
pub struct DeployRequest {
    pub name: String,
    #[serde(default)]
    pub config: IsolateConfig,
    #[serde(default)]
    pub bundle_format: BundleFormat,
    /// The eszip bytes (set from multipart body, not from JSON).
    #[serde(skip)]
    pub eszip_bytes: Bytes,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn function_status_serde_round_trip() {
        let statuses = vec![
            FunctionStatus::Loading,
            FunctionStatus::Running,
            FunctionStatus::Error,
            FunctionStatus::ShuttingDown,
        ];
        for status in statuses {
            let json = serde_json::to_string(&status).unwrap();
            let deserialized: FunctionStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(status, deserialized);
        }
    }

    #[test]
    fn function_status_snake_case() {
        assert_eq!(serde_json::to_string(&FunctionStatus::Loading).unwrap(), "\"loading\"");
        assert_eq!(serde_json::to_string(&FunctionStatus::Running).unwrap(), "\"running\"");
        assert_eq!(serde_json::to_string(&FunctionStatus::Error).unwrap(), "\"error\"");
        assert_eq!(serde_json::to_string(&FunctionStatus::ShuttingDown).unwrap(), "\"shutting_down\"");
    }

    #[test]
    fn function_metrics_default_zeros() {
        let m = FunctionMetrics::default();
        assert_eq!(m.total_requests.load(Ordering::Relaxed), 0);
        assert_eq!(m.active_requests.load(Ordering::Relaxed), 0);
        assert_eq!(m.total_errors.load(Ordering::Relaxed), 0);
        assert_eq!(m.total_cpu_time_ms.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn function_metrics_atomic_counters() {
        let m = FunctionMetrics::default();
        m.total_requests.fetch_add(5, Ordering::Relaxed);
        m.active_requests.fetch_add(2, Ordering::Relaxed);
        m.total_errors.fetch_add(1, Ordering::Relaxed);
        m.total_cpu_time_ms.fetch_add(100, Ordering::Relaxed);
        let snap = m.snapshot();
        assert_eq!(snap.total_requests, 5);
        assert_eq!(snap.active_requests, 2);
        assert_eq!(snap.total_errors, 1);
        assert_eq!(snap.total_cpu_time_ms, 100);
    }

    #[test]
    fn function_metrics_snapshot_serializes() {
        let m = FunctionMetrics::default();
        m.total_requests.fetch_add(10, Ordering::Relaxed);
        let snap = m.snapshot();
        let json = serde_json::to_string(&snap).unwrap();
        assert!(json.contains("\"total_requests\":10"));
    }

    #[test]
    fn function_entry_to_info() {
        let metrics = Arc::new(FunctionMetrics::default());
        metrics.total_requests.fetch_add(3, Ordering::Relaxed);
        let now = Utc::now();
        let entry = FunctionEntry {
            name: "test-fn".to_string(),
            eszip_bytes: Bytes::new(),
            bundle_format: BundleFormat::Eszip,
            isolate_handle: None,
            status: FunctionStatus::Running,
            config: IsolateConfig::default(),
            metrics,
            created_at: now,
            updated_at: now,
            last_error: None,
        };
        let info = entry.to_info();
        assert_eq!(info.name, "test-fn");
        assert_eq!(info.status, FunctionStatus::Running);
        assert_eq!(info.metrics.total_requests, 3);
        assert!(info.last_error.is_none());
    }

    #[test]
    fn deploy_request_deserialization() {
        let json = r#"{"name":"my-func"}"#;
        let req: DeployRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.name, "my-func");
        assert!(req.eszip_bytes.is_empty());
    }
}

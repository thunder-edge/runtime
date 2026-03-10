use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use bytes::Bytes;
use chrono::{DateTime, Utc};
use runtime_core::isolate::{IsolateConfig, IsolateHandle};
use runtime_core::manifest::{ManifestRouteKind, ResolvedFunctionManifest};
use serde::{Deserialize, Serialize};

/// Bundle format for function deployment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BundleFormat {
    /// Traditional eszip bundle (modules loaded at startup).
    Eszip,
    /// Snapshot-flavor bundle (bytecode cache envelope + ESZIP fallback).
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
    /// Primary bundle data (snapshot-flavor payload or eszip bytes).
    pub bundle: Vec<u8>,
    /// Fallback eszip bytes (used if snapshot fails to load).
    #[serde(default)]
    pub fallback_eszip: Option<Vec<u8>>,
    /// Optional manifest JSON embedded at bundle time.
    #[serde(default)]
    pub embedded_manifest_json: Option<String>,
    /// Optional routed-app route metadata generated at build time.
    #[serde(default)]
    pub embedded_route_metadata: Option<BundleRouteMetadata>,
}

/// Build-time route metadata embedded in routed-app bundle artifacts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleRouteMetadata {
    pub generated_at_unix_ms: i64,
    pub routes: Vec<BundleRouteRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleRouteRecord {
    pub kind: ManifestRouteKind,
    pub path: String,
    pub methods: Vec<String>,
    pub entrypoint: Option<String>,
    pub asset_dir: Option<String>,
    pub precedence_rank: u32,
}

impl BundlePackage {
    /// Create a new snapshot bundle with eszip fallback.
    pub fn snapshot_with_fallback(snapshot: Vec<u8>, fallback_eszip: Vec<u8>) -> Self {
        Self {
            format: BundleFormat::Snapshot,
            v8_version: get_v8_version().to_string(),
            bundle: snapshot,
            fallback_eszip: Some(fallback_eszip),
            embedded_manifest_json: None,
            embedded_route_metadata: None,
        }
    }

    /// Create a new eszip-only bundle.
    pub fn eszip_only(eszip: Vec<u8>) -> Self {
        Self {
            format: BundleFormat::Eszip,
            v8_version: get_v8_version().to_string(),
            bundle: eszip,
            fallback_eszip: None,
            embedded_manifest_json: None,
            embedded_route_metadata: None,
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
    pub cold_start_count: AtomicU64, // Total de cold starts (inicializações)
    pub total_cold_start_time_ms: AtomicU64, // Tempo acumulado de cold start (ms)
    pub total_cold_start_time_us: AtomicU64, // Tempo acumulado de cold start (us)
    pub total_warm_start_time_ms: AtomicU64, // Tempo acumulado de requisições após boot (ms)
    pub total_warm_start_time_us: AtomicU64, // Tempo acumulado de requisições após boot (us)
    pub current_heap_used_bytes: AtomicU64,
    pub peak_heap_used_bytes: AtomicU64,
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
            total_cold_start_time_us: AtomicU64::new(0),
            total_warm_start_time_ms: AtomicU64::new(0),
            total_warm_start_time_us: AtomicU64::new(0),
            current_heap_used_bytes: AtomicU64::new(0),
            peak_heap_used_bytes: AtomicU64::new(0),
        }
    }
}

impl FunctionMetrics {
    pub fn snapshot(&self) -> FunctionMetricsSnapshot {
        let total_requests = self.total_requests.load(Ordering::Relaxed);
        let total_warm_start_time_ms = self.total_warm_start_time_ms.load(Ordering::Relaxed);
        let total_warm_start_time_us = self.total_warm_start_time_us.load(Ordering::Relaxed);
        let cold_start_count = self.cold_start_count.load(Ordering::Relaxed);
        let total_cold_start_time_ms = self.total_cold_start_time_ms.load(Ordering::Relaxed);
        let total_cold_start_time_us = self.total_cold_start_time_us.load(Ordering::Relaxed);
        let current_heap_used_bytes = self.current_heap_used_bytes.load(Ordering::Relaxed);
        let peak_heap_used_bytes = self.peak_heap_used_bytes.load(Ordering::Relaxed);

        let avg_warm_request_ms = if total_requests > 0 {
            total_warm_start_time_ms / total_requests
        } else {
            0
        };

        let avg_warm_request_us = if total_requests > 0 {
            total_warm_start_time_us / total_requests
        } else {
            0
        };

        let avg_warm_request_ms_precise = if total_requests > 0 {
            total_warm_start_time_us as f64 / total_requests as f64 / 1000.0
        } else {
            0.0
        };

        let avg_cold_start_ms = if cold_start_count > 0 {
            total_cold_start_time_ms / cold_start_count
        } else {
            0
        };

        let avg_cold_start_us = if cold_start_count > 0 {
            total_cold_start_time_us / cold_start_count
        } else {
            0
        };

        let avg_cold_start_ms_precise = if cold_start_count > 0 {
            total_cold_start_time_us as f64 / cold_start_count as f64 / 1000.0
        } else {
            0.0
        };

        FunctionMetricsSnapshot {
            total_requests,
            active_requests: self.active_requests.load(Ordering::Relaxed),
            total_errors: self.total_errors.load(Ordering::Relaxed),
            total_cpu_time_ms: self.total_cpu_time_ms.load(Ordering::Relaxed),
            cold_starts: cold_start_count,
            avg_cold_start_ms,
            total_cold_start_time_ms,
            total_cold_start_time_us,
            avg_cold_start_us,
            avg_cold_start_ms_precise,
            total_warm_start_time_ms,
            total_warm_start_time_us,
            avg_warm_request_ms,
            avg_warm_request_us,
            avg_warm_request_ms_precise,
            current_heap_used_bytes,
            peak_heap_used_bytes,
            current_heap_used_mb: current_heap_used_bytes as f64 / (1024.0 * 1024.0),
            peak_heap_used_mb: peak_heap_used_bytes as f64 / (1024.0 * 1024.0),
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
    pub cold_starts: u64,              // Total de cold starts
    pub avg_cold_start_ms: u64,        // Média de cold start (ms)
    pub total_cold_start_time_ms: u64, // Tempo total de cold start (ms)
    pub total_cold_start_time_us: u64, // Tempo total de cold start (us)
    pub avg_cold_start_us: u64,        // Média de cold start (us)
    pub avg_cold_start_ms_precise: f64, // Média de cold start (ms, precisão sub-ms)
    pub total_warm_start_time_ms: u64, // Tempo total de requisições após boot (ms)
    pub total_warm_start_time_us: u64, // Tempo total de requisições após boot (us)
    pub avg_warm_request_ms: u64,      // Média de requisição warm start (ms)
    pub avg_warm_request_us: u64,      // Média de requisição warm start (us)
    pub avg_warm_request_ms_precise: f64, // Média de requisição warm start (ms, precisão sub-ms)
    pub current_heap_used_bytes: u64,
    pub peak_heap_used_bytes: u64,
    pub current_heap_used_mb: f64,
    pub peak_heap_used_mb: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PoolLimits {
    pub min: usize,
    pub max: usize,
}

impl Default for PoolLimits {
    fn default() -> Self {
        Self { min: 1, max: 1 }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ContextPoolLimits {
    pub min: usize,
    pub max: usize,
}

impl Default for ContextPoolLimits {
    fn default() -> Self {
        Self { min: 1, max: 8 }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FunctionPoolSnapshot {
    pub min: usize,
    pub max: usize,
    pub current: usize,
}

/// A single deployed function entry.
pub struct FunctionEntry {
    /// Unique name (used as path prefix for routing).
    pub name: String,
    /// Original deploy package bytes (used for replica boot and updates).
    pub bundle_package_bytes: Bytes,
    /// The raw eszip bundle bytes (kept for hot-reload).
    pub eszip_bytes: Bytes,
    /// The original bundle format used.
    pub bundle_format: BundleFormat,
    /// V8 version embedded in the deploy package metadata.
    pub package_v8_version: String,
    /// Handle to the running isolate (None if loading/error).
    pub isolate_handle: Option<IsolateHandle>,
    /// Additional isolate handles for the same function (pool replicas).
    pub extra_isolate_handles: Vec<IsolateHandle>,
    /// Pool limits for this function.
    pub pool_limits: PoolLimits,
    /// Round-robin cursor across available handles.
    pub next_handle_index: u64,
    /// Stop signal for the inspector listener thread (if inspector is active).
    pub inspector_stop: Option<Arc<AtomicBool>>,
    /// Current status.
    pub status: FunctionStatus,
    /// Runtime configuration.
    pub config: IsolateConfig,
    /// Optional manifest policy resolved at deploy time.
    pub manifest: Option<ResolvedFunctionManifest>,
    /// Optional route metadata embedded by the bundle pipeline.
    pub route_metadata: Option<BundleRouteMetadata>,
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
        let runtime_v8_version = get_v8_version().to_string();
        let snapshot_compatible_with_runtime = self.package_v8_version == runtime_v8_version;
        let requires_snapshot_regeneration =
            self.bundle_format == BundleFormat::Snapshot && !snapshot_compatible_with_runtime;

        FunctionInfo {
            name: self.name.clone(),
            status: self.status,
            metrics: self.metrics.snapshot(),
            bundle_format: self.bundle_format,
            package_v8_version: self.package_v8_version.clone(),
            runtime_v8_version,
            snapshot_compatible_with_runtime,
            requires_snapshot_regeneration,
            stored_eszip_size_bytes: self.eszip_bytes.len() as u64,
            can_regenerate_snapshot_from_stored_eszip: !self.eszip_bytes.is_empty(),
            pool: FunctionPoolSnapshot {
                min: self.pool_limits.min,
                max: self.pool_limits.max,
                current: self.current_pool_size(),
            },
            created_at: self.created_at,
            updated_at: self.updated_at,
            last_error: self.last_error.clone(),
        }
    }

    pub fn current_pool_size(&self) -> usize {
        let base = if self.isolate_handle.is_some() { 1 } else { 0 };
        base + self.extra_isolate_handles.len()
    }
}

/// API response for function info.
#[derive(Debug, Serialize)]
pub struct FunctionInfo {
    pub name: String,
    pub status: FunctionStatus,
    pub metrics: FunctionMetricsSnapshot,
    pub bundle_format: BundleFormat,
    pub package_v8_version: String,
    pub runtime_v8_version: String,
    pub snapshot_compatible_with_runtime: bool,
    pub requires_snapshot_regeneration: bool,
    pub stored_eszip_size_bytes: u64,
    pub can_regenerate_snapshot_from_stored_eszip: bool,
    pub pool: FunctionPoolSnapshot,
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
        assert_eq!(
            serde_json::to_string(&FunctionStatus::Loading).unwrap(),
            "\"loading\""
        );
        assert_eq!(
            serde_json::to_string(&FunctionStatus::Running).unwrap(),
            "\"running\""
        );
        assert_eq!(
            serde_json::to_string(&FunctionStatus::Error).unwrap(),
            "\"error\""
        );
        assert_eq!(
            serde_json::to_string(&FunctionStatus::ShuttingDown).unwrap(),
            "\"shutting_down\""
        );
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
            bundle_package_bytes: Bytes::new(),
            eszip_bytes: Bytes::new(),
            bundle_format: BundleFormat::Eszip,
            package_v8_version: get_v8_version().to_string(),
            isolate_handle: None,
            extra_isolate_handles: Vec::new(),
            pool_limits: PoolLimits::default(),
            next_handle_index: 0,
            inspector_stop: None,
            status: FunctionStatus::Running,
            config: IsolateConfig::default(),
            manifest: None,
            route_metadata: None,
            metrics,
            created_at: now,
            updated_at: now,
            last_error: None,
        };
        let info = entry.to_info();
        assert_eq!(info.name, "test-fn");
        assert_eq!(info.status, FunctionStatus::Running);
        assert_eq!(info.metrics.total_requests, 3);
        assert_eq!(info.bundle_format, BundleFormat::Eszip);
        assert_eq!(info.package_v8_version, get_v8_version());
        assert_eq!(info.runtime_v8_version, get_v8_version());
        assert!(info.snapshot_compatible_with_runtime);
        assert!(!info.requires_snapshot_regeneration);
        assert_eq!(info.stored_eszip_size_bytes, 0);
        assert!(!info.can_regenerate_snapshot_from_stored_eszip);
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

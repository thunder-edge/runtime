use anyhow::Error;
use bytes::Bytes;
use chrono::Utc;
use dashmap::DashMap;
use sysinfo::System;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use runtime_core::isolate::IsolateConfig;
use runtime_core::manifest::ResolvedFunctionManifest;

use crate::lifecycle;
use crate::types::*;

#[derive(Debug, Clone)]
pub struct PoolRuntimeConfig {
    pub enabled: bool,
    pub global_max_isolates: usize,
    pub min_free_memory_mib: u64,
}

impl Default for PoolRuntimeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            global_max_isolates: 1024,
            min_free_memory_mib: 256,
        }
    }
}

/// Thread-safe registry of all deployed functions.
pub struct FunctionRegistry {
    functions: DashMap<String, FunctionEntry>,
    global_shutdown: CancellationToken,
    default_config: IsolateConfig,
    pool_config: PoolRuntimeConfig,
    default_pool_limits: PoolLimits,
}

impl FunctionRegistry {
    fn apply_manifest_resources(config: &mut IsolateConfig, manifest: &ResolvedFunctionManifest) {
        if let Some(max_heap_mi_b) = manifest.resources.max_heap_mi_b {
            let max_bytes = (max_heap_mi_b as usize).saturating_mul(1024 * 1024);
            config.max_heap_size_bytes = max_bytes;
        }

        if let Some(cpu_time_ms) = manifest.resources.cpu_time_ms {
            config.cpu_time_limit_ms = cpu_time_ms;
        }

        if let Some(wall_clock_timeout_ms) = manifest.resources.wall_clock_timeout_ms {
            config.wall_clock_timeout_ms = wall_clock_timeout_ms;
        }
    }

    fn all_request_channels_closed(&self) -> bool {
        self.functions.iter().all(|entry| {
            let primary_closed = entry
                .isolate_handle
                .as_ref()
                .map(|h| h.is_request_channel_closed())
                .unwrap_or(true);
            let extra_closed = entry
                .extra_isolate_handles
                .iter()
                .all(|h| h.is_request_channel_closed());
            primary_closed && extra_closed
        })
    }

    fn reconcile_entry_status(entry: &mut FunctionEntry) {
        entry
            .extra_isolate_handles
            .retain(|handle| handle.is_alive());

        let mut alive_count = entry
            .extra_isolate_handles
            .iter()
            .filter(|handle| handle.is_alive())
            .count();

        if let Some(handle) = &entry.isolate_handle {
            if handle.is_alive() {
                alive_count += 1;
            } else {
                entry.isolate_handle = None;
            }
        }

        let is_dead = alive_count == 0;

        if is_dead && entry.status == FunctionStatus::Running {
            entry.status = FunctionStatus::Error;
            entry.updated_at = Utc::now();
            if entry.last_error.is_none() {
                entry.last_error =
                    Some("isolate terminated unexpectedly (panic or resource limit)".to_string());
            }
        } else if !is_dead && entry.status == FunctionStatus::Error {
            entry.status = FunctionStatus::Running;
            entry.updated_at = Utc::now();
        }
    }

    pub fn new(global_shutdown: CancellationToken, default_config: IsolateConfig) -> Self {
        Self::new_with_pool(
            global_shutdown,
            default_config,
            PoolRuntimeConfig::default(),
            PoolLimits::default(),
        )
    }

    pub fn new_with_pool(
        global_shutdown: CancellationToken,
        default_config: IsolateConfig,
        pool_config: PoolRuntimeConfig,
        default_pool_limits: PoolLimits,
    ) -> Self {
        Self {
            functions: DashMap::new(),
            global_shutdown,
            default_config,
            pool_config,
            default_pool_limits,
        }
    }

    fn current_total_isolates(&self) -> usize {
        self.functions
            .iter()
            .map(|entry| entry.current_pool_size())
            .sum()
    }

    fn can_scale_with_memory(&self, function_name: &str) -> bool {
        let mut sys = System::new_all();
        sys.refresh_memory();
        let available_mib = sys.available_memory() / (1024 * 1024);
        if available_mib < self.pool_config.min_free_memory_mib {
            warn!(
                "pool scale blocked for '{}' due to low memory (available={}MiB, min_required={}MiB)",
                function_name, available_mib, self.pool_config.min_free_memory_mib
            );
            warn!(
                "TODO: trigger external alert hook for low-memory pool scale block"
            );
            return false;
        }
        true
    }

    async fn create_replica_handle(
        &self,
        function_name: &str,
        eszip_bytes: Bytes,
        config: IsolateConfig,
        manifest: Option<ResolvedFunctionManifest>,
    ) -> Result<Option<runtime_core::isolate::IsolateHandle>, Error> {
        if !self.pool_config.enabled {
            return Ok(None);
        }

        if self.current_total_isolates() >= self.pool_config.global_max_isolates {
            warn!(
                "pool scale blocked for '{}' due to global isolate limit ({})",
                function_name, self.pool_config.global_max_isolates
            );
            return Ok(None);
        }

        if !self.can_scale_with_memory(function_name) {
            return Ok(None);
        }

        let replica_entry = lifecycle::create_function(
            function_name.to_string(),
            eszip_bytes.to_vec(),
            config,
            manifest,
            self.global_shutdown.child_token(),
        )
        .await?;

        Ok(replica_entry.isolate_handle)
    }

    /// Deploy a new function. Returns error if name already exists.
    pub async fn deploy(
        &self,
        name: String,
        eszip_bytes: Bytes,
        config: Option<IsolateConfig>,
        manifest: Option<ResolvedFunctionManifest>,
    ) -> Result<FunctionInfo, Error> {
        if self.functions.contains_key(&name) {
            return Err(anyhow::anyhow!(
                "function '{}' already exists, use PUT to update",
                name
            ));
        }

        let mut config = config.unwrap_or_else(|| self.default_config.clone());
        if let Some(policy) = &manifest {
            Self::apply_manifest_resources(&mut config, policy);
        }

        info!("deploying function '{}'", name);

        let entry = lifecycle::create_function(
            name.clone(),
            eszip_bytes.to_vec(),
            config,
            manifest,
            self.global_shutdown.child_token(),
        )
        .await?;

        let mut entry = entry;
        entry.pool_limits = if self.pool_config.enabled {
            self.default_pool_limits
        } else {
            PoolLimits::default()
        };

        if self.pool_config.enabled && entry.pool_limits.max > 1 {
            while entry.current_pool_size() < entry.pool_limits.min
                && entry.current_pool_size() < entry.pool_limits.max
            {
                match self
                    .create_replica_handle(
                        &name,
                        entry.eszip_bytes.clone(),
                        entry.config.clone(),
                        entry.manifest.clone(),
                    )
                    .await
                {
                    Ok(Some(handle)) => entry.extra_isolate_handles.push(handle),
                    Ok(None) => break,
                    Err(err) => {
                        warn!(
                            "failed to pre-warm replica for '{}': {}",
                            name, err
                        );
                        break;
                    }
                }
            }
        }

        let info = entry.to_info();
        self.functions.insert(name, entry);
        Ok(info)
    }

    /// Get a handle to route a request to.
    /// Returns None if function doesn't exist, isn't running, or the isolate is dead.
    pub fn get_handle(&self, name: &str) -> Option<runtime_core::isolate::IsolateHandle> {
        self.functions.get_mut(name).and_then(|mut entry| {
            Self::reconcile_entry_status(&mut entry);
            if entry.status != FunctionStatus::Running {
                return None;
            }

            let mut handles: Vec<runtime_core::isolate::IsolateHandle> = Vec::new();
            if let Some(handle) = &entry.isolate_handle {
                if handle.is_alive() {
                    handles.push(handle.clone());
                }
            }
            for handle in &entry.extra_isolate_handles {
                if handle.is_alive() {
                    handles.push(handle.clone());
                }
            }

            if handles.is_empty() {
                return None;
            }

            let idx = (entry.next_handle_index as usize) % handles.len();
            entry.next_handle_index = entry.next_handle_index.wrapping_add(1);
            Some(handles[idx].clone())
        })
    }

    /// Get the config for a function.
    pub fn get_config(&self, name: &str) -> Option<IsolateConfig> {
        self.functions.get(name).map(|entry| entry.config.clone())
    }

    /// List all functions.
    pub fn list(&self) -> Vec<FunctionInfo> {
        let names: Vec<String> = self
            .functions
            .iter()
            .map(|entry| entry.key().clone())
            .collect();

        names
            .into_iter()
            .filter_map(|name| self.get_info(&name))
            .collect()
    }

    /// Get info about a specific function.
    pub fn get_info(&self, name: &str) -> Option<FunctionInfo> {
        self.functions.get_mut(name).map(|mut entry| {
            Self::reconcile_entry_status(&mut entry);
            entry.to_info()
        })
    }

    /// Update a function with a new eszip bundle.
    pub async fn update(
        &self,
        name: &str,
        eszip_bytes: Bytes,
        config: Option<IsolateConfig>,
        manifest: Option<ResolvedFunctionManifest>,
    ) -> Result<FunctionInfo, Error> {
        let old_config = self.functions.get(name).map(|entry| entry.config.clone());
        let old_manifest = self.functions.get(name).and_then(|entry| entry.manifest.clone());

        // Destroy the old entry
        if let Some((_, old_entry)) = self.functions.remove(name) {
            info!("shutting down old isolate for function '{}'", name);
            lifecycle::destroy_function(&old_entry).await;
        }

        let config = config
            .or(old_config)
            .unwrap_or_else(|| self.default_config.clone());
        let manifest = manifest.or(old_manifest);
        let mut config = config;
        if let Some(policy) = &manifest {
            Self::apply_manifest_resources(&mut config, policy);
        }

        info!("deploying updated function '{}'", name);

        let entry = lifecycle::create_function(
            name.to_string(),
            eszip_bytes.to_vec(),
            config,
            manifest,
            self.global_shutdown.child_token(),
        )
        .await?;

        let info = entry.to_info();
        self.functions.insert(name.to_string(), entry);
        Ok(info)
    }

    /// Delete a function entirely.
    pub async fn delete(&self, name: &str) -> Result<(), Error> {
        if let Some((_, entry)) = self.functions.remove(name) {
            info!("deleting function '{}'", name);
            for extra in &entry.extra_isolate_handles {
                extra.shutdown.cancel();
                extra.close_request_tx();
            }
            lifecycle::destroy_function(&entry).await;
            Ok(())
        } else {
            Err(anyhow::anyhow!("function '{}' not found", name))
        }
    }

    /// Hot-reload a function (feature-gated).
    #[cfg(feature = "hot-reload")]
    pub async fn reload(&self, name: &str) -> Result<FunctionInfo, Error> {
        let eszip_bytes = self
            .functions
            .get(name)
            .map(|entry| entry.eszip_bytes.clone())
            .ok_or_else(|| anyhow::anyhow!("function '{}' not found", name))?;

        let config = self
            .functions
            .get(name)
            .map(|entry| entry.config.clone())
            .unwrap_or_default();

        let manifest = self
            .functions
            .get(name)
            .and_then(|entry| entry.manifest.clone());

        // Destroy old, recreate from same bytes
        if let Some((_, old_entry)) = self.functions.remove(name) {
            for extra in &old_entry.extra_isolate_handles {
                extra.shutdown.cancel();
                extra.close_request_tx();
            }
            lifecycle::destroy_function(&old_entry).await;
        }

        info!("hot-reloading function '{}'", name);

        let entry = lifecycle::create_function(
            name.to_string(),
            eszip_bytes.to_vec(),
            config,
            manifest,
            self.global_shutdown.child_token(),
        )
        .await?;

        let info = entry.to_info();
        self.functions.insert(name.to_string(), entry);
        Ok(info)
    }

    /// Shut down all functions gracefully.
    pub async fn shutdown_all(&self) {
        self.shutdown_all_with_deadline(std::time::Duration::from_secs(2))
            .await;
    }

    /// Shut down all functions with explicit deadline.
    ///
    /// Steps:
    /// 1) mark entries as shutting down and cancel each isolate token
    /// 2) wait until request channels are closed
    /// 3) on deadline, force clear with warning
    pub async fn shutdown_all_with_deadline(&self, deadline: std::time::Duration) {
        let total = self.functions.len();
        info!(
            "shutting down all functions ({} total, deadline={}ms)",
            total,
            deadline.as_millis()
        );

        self.global_shutdown.cancel();

        for mut entry in self.functions.iter_mut() {
            entry.status = FunctionStatus::ShuttingDown;
            entry.updated_at = Utc::now();
            if let Some(handle) = &entry.isolate_handle {
                handle.shutdown.cancel();
            }
            for handle in &entry.extra_isolate_handles {
                handle.shutdown.cancel();
                handle.close_request_tx();
            }
        }

        let started = std::time::Instant::now();
        while started.elapsed() < deadline {
            if self.all_request_channels_closed() {
                self.functions.clear();
                info!("all function channels closed before shutdown deadline");
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }

        let still_open = self
            .functions
            .iter()
            .filter(|entry| {
                entry
                    .isolate_handle
                    .as_ref()
                    .map(|h| !h.is_request_channel_closed())
                    .unwrap_or(false)
                    || entry
                        .extra_isolate_handles
                        .iter()
                        .any(|h| !h.is_request_channel_closed())
            })
            .count();

        if still_open > 0 {
            warn!(
                "shutdown deadline reached with {} function(s) still open; forcing clear",
                still_open
            );
        }

        self.functions.clear();
    }

    /// Number of deployed functions.
    pub fn count(&self) -> usize {
        self.functions.len()
    }

    pub fn get_pool_limits(&self, name: &str) -> Option<PoolLimits> {
        self.functions.get(name).map(|entry| entry.pool_limits)
    }

    pub async fn set_pool_limits(
        &self,
        name: &str,
        min: usize,
        max: usize,
    ) -> Result<FunctionInfo, Error> {
        if min > max {
            return Err(anyhow::anyhow!("invalid pool limits: min must be <= max"));
        }

        let Some((key, mut entry)) = self.functions.remove(name) else {
            return Err(anyhow::anyhow!("function '{}' not found", name));
        };

        entry.pool_limits = if self.pool_config.enabled {
            PoolLimits { min, max }
        } else {
            PoolLimits::default()
        };

        // Shrink extra replicas above max (primary handle is always retained if alive).
        while entry.current_pool_size() > entry.pool_limits.max {
            if let Some(extra) = entry.extra_isolate_handles.pop() {
                extra.shutdown.cancel();
                extra.close_request_tx();
            } else {
                break;
            }
        }

        // Pre-warm up to min when pooling is enabled.
        if self.pool_config.enabled && entry.pool_limits.max > 1 {
            while entry.current_pool_size() < entry.pool_limits.min
                && entry.current_pool_size() < entry.pool_limits.max
            {
                match self
                    .create_replica_handle(
                        name,
                        entry.eszip_bytes.clone(),
                        entry.config.clone(),
                        entry.manifest.clone(),
                    )
                    .await
                {
                    Ok(Some(handle)) => entry.extra_isolate_handles.push(handle),
                    Ok(None) => break,
                    Err(err) => {
                        warn!("failed to scale pool for '{}': {}", name, err);
                        break;
                    }
                }
            }
        }

        let info = entry.to_info();
        self.functions.insert(key, entry);
        Ok(info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    use chrono::Utc;

    use crate::types::{BundleFormat, FunctionEntry, FunctionMetrics, FunctionStatus};

    fn make_registry() -> FunctionRegistry {
        let shutdown = CancellationToken::new();
        FunctionRegistry::new(shutdown, IsolateConfig::default())
    }

    fn make_registry_with_pool(enabled: bool) -> FunctionRegistry {
        let shutdown = CancellationToken::new();
        FunctionRegistry::new_with_pool(
            shutdown,
            IsolateConfig::default(),
            PoolRuntimeConfig {
                enabled,
                global_max_isolates: 16,
                min_free_memory_mib: 0,
            },
            PoolLimits::default(),
        )
    }

    #[test]
    fn empty_registry_count_zero() {
        let reg = make_registry();
        assert_eq!(reg.count(), 0);
    }

    #[test]
    fn empty_registry_list_empty() {
        let reg = make_registry();
        assert!(reg.list().is_empty());
    }

    #[test]
    fn get_handle_none_for_missing() {
        let reg = make_registry();
        assert!(reg.get_handle("nonexistent").is_none());
    }

    #[test]
    fn get_info_none_for_missing() {
        let reg = make_registry();
        assert!(reg.get_info("nonexistent").is_none());
    }

    #[test]
    fn delete_missing_returns_error() {
        let reg = make_registry();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = rt.block_on(reg.delete("nonexistent"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn dead_isolate_is_marked_as_error_in_registry() {
        let reg = make_registry();

        let (request_tx, _request_rx) = tokio::sync::mpsc::unbounded_channel();
        let alive = Arc::new(AtomicBool::new(false));
        let handle = runtime_core::isolate::IsolateHandle {
            request_tx: Arc::new(std::sync::Mutex::new(Some(request_tx))),
            shutdown: CancellationToken::new(),
            id: uuid::Uuid::new_v4(),
            alive,
        };

        let entry = FunctionEntry {
            name: "dead-fn".to_string(),
            eszip_bytes: Bytes::new(),
            bundle_format: BundleFormat::Eszip,
            isolate_handle: Some(handle),
            extra_isolate_handles: Vec::new(),
            pool_limits: PoolLimits::default(),
            next_handle_index: 0,
            inspector_stop: None,
            status: FunctionStatus::Running,
            config: IsolateConfig::default(),
            manifest: None,
            metrics: Arc::new(FunctionMetrics::default()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_error: None,
        };

        reg.functions.insert("dead-fn".to_string(), entry);

        assert!(reg.get_handle("dead-fn").is_none());

        let info = reg.get_info("dead-fn").expect("missing function info");
        assert_eq!(info.status, FunctionStatus::Error);
        assert!(info.last_error.is_some());
    }

    #[test]
    fn shutdown_all_with_deadline_closes_registry_entries() {
        let reg = make_registry();

        let (request_tx, _request_rx) = tokio::sync::mpsc::unbounded_channel();
        let alive = Arc::new(AtomicBool::new(true));
        let handle = runtime_core::isolate::IsolateHandle {
            request_tx: Arc::new(std::sync::Mutex::new(Some(request_tx))),
            shutdown: CancellationToken::new(),
            id: uuid::Uuid::new_v4(),
            alive,
        };

        let entry = FunctionEntry {
            name: "shutdown-fn".to_string(),
            eszip_bytes: Bytes::new(),
            bundle_format: BundleFormat::Eszip,
            isolate_handle: Some(handle),
            extra_isolate_handles: Vec::new(),
            pool_limits: PoolLimits::default(),
            next_handle_index: 0,
            inspector_stop: None,
            status: FunctionStatus::Running,
            config: IsolateConfig::default(),
            manifest: None,
            metrics: Arc::new(FunctionMetrics::default()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_error: None,
        };
        reg.functions.insert("shutdown-fn".to_string(), entry);

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(reg.shutdown_all_with_deadline(std::time::Duration::from_millis(20)));

        assert_eq!(reg.count(), 0);
    }

    #[test]
    fn get_handle_round_robin_across_replicas() {
        let reg = make_registry_with_pool(true);

        let (primary_tx, _primary_rx) = tokio::sync::mpsc::unbounded_channel();
        let primary = runtime_core::isolate::IsolateHandle {
            request_tx: Arc::new(std::sync::Mutex::new(Some(primary_tx))),
            shutdown: CancellationToken::new(),
            id: uuid::Uuid::new_v4(),
            alive: Arc::new(AtomicBool::new(true)),
        };

        let (replica_tx, _replica_rx) = tokio::sync::mpsc::unbounded_channel();
        let replica = runtime_core::isolate::IsolateHandle {
            request_tx: Arc::new(std::sync::Mutex::new(Some(replica_tx))),
            shutdown: CancellationToken::new(),
            id: uuid::Uuid::new_v4(),
            alive: Arc::new(AtomicBool::new(true)),
        };

        let entry = FunctionEntry {
            name: "rr-fn".to_string(),
            eszip_bytes: Bytes::new(),
            bundle_format: BundleFormat::Eszip,
            isolate_handle: Some(primary.clone()),
            extra_isolate_handles: vec![replica.clone()],
            pool_limits: PoolLimits { min: 1, max: 2 },
            next_handle_index: 0,
            inspector_stop: None,
            status: FunctionStatus::Running,
            config: IsolateConfig::default(),
            manifest: None,
            metrics: Arc::new(FunctionMetrics::default()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_error: None,
        };

        reg.functions.insert("rr-fn".to_string(), entry);

        let h1 = reg.get_handle("rr-fn").expect("missing first handle");
        let h2 = reg.get_handle("rr-fn").expect("missing second handle");
        let h3 = reg.get_handle("rr-fn").expect("missing third handle");

        assert_eq!(h1.id, primary.id);
        assert_eq!(h2.id, replica.id);
        assert_eq!(h3.id, primary.id);
    }

    #[test]
    fn set_pool_limits_updates_entry_when_pool_enabled() {
        let reg = make_registry_with_pool(true);

        let (request_tx, _request_rx) = tokio::sync::mpsc::unbounded_channel();
        let handle = runtime_core::isolate::IsolateHandle {
            request_tx: Arc::new(std::sync::Mutex::new(Some(request_tx))),
            shutdown: CancellationToken::new(),
            id: uuid::Uuid::new_v4(),
            alive: Arc::new(AtomicBool::new(true)),
        };

        let entry = FunctionEntry {
            name: "pool-fn".to_string(),
            eszip_bytes: Bytes::new(),
            bundle_format: BundleFormat::Eszip,
            isolate_handle: Some(handle),
            extra_isolate_handles: Vec::new(),
            pool_limits: PoolLimits::default(),
            next_handle_index: 0,
            inspector_stop: None,
            status: FunctionStatus::Running,
            config: IsolateConfig::default(),
            manifest: None,
            metrics: Arc::new(FunctionMetrics::default()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_error: None,
        };
        reg.functions.insert("pool-fn".to_string(), entry);

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = rt.block_on(reg.set_pool_limits("pool-fn", 1, 3));

        assert!(result.is_ok(), "set_pool_limits should succeed");
        let limits = reg.get_pool_limits("pool-fn").expect("pool limits missing");
        assert_eq!(limits.min, 1);
        assert_eq!(limits.max, 3);
    }
}

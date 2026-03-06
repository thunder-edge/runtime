use anyhow::Error;
use bytes::Bytes;
use chrono::Utc;
use dashmap::DashMap;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use runtime_core::isolate::IsolateConfig;

use crate::lifecycle;
use crate::types::*;

/// Thread-safe registry of all deployed functions.
pub struct FunctionRegistry {
    functions: DashMap<String, FunctionEntry>,
    global_shutdown: CancellationToken,
    default_config: IsolateConfig,
}

impl FunctionRegistry {
    fn all_request_channels_closed(&self) -> bool {
        self.functions.iter().all(|entry| {
            entry
                .isolate_handle
                .as_ref()
                .map(|h| h.is_request_channel_closed())
                .unwrap_or(true)
        })
    }

    fn reconcile_entry_status(entry: &mut FunctionEntry) {
        let is_dead = entry
            .isolate_handle
            .as_ref()
            .map(|handle| !handle.is_alive())
            .unwrap_or(true);

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
        Self {
            functions: DashMap::new(),
            global_shutdown,
            default_config,
        }
    }

    /// Deploy a new function. Returns error if name already exists.
    pub async fn deploy(
        &self,
        name: String,
        eszip_bytes: Bytes,
        config: Option<IsolateConfig>,
    ) -> Result<FunctionInfo, Error> {
        if self.functions.contains_key(&name) {
            return Err(anyhow::anyhow!(
                "function '{}' already exists, use PUT to update",
                name
            ));
        }

        let config = config.unwrap_or_else(|| self.default_config.clone());

        info!("deploying function '{}'", name);

        let entry = lifecycle::create_function(
            name.clone(),
            eszip_bytes.to_vec(),
            config,
            self.global_shutdown.child_token(),
        )
        .await?;

        let info = entry.to_info();
        self.functions.insert(name, entry);
        Ok(info)
    }

    /// Get a handle to route a request to.
    /// Returns None if function doesn't exist, isn't running, or the isolate is dead.
    pub fn get_handle(&self, name: &str) -> Option<runtime_core::isolate::IsolateHandle> {
        self.functions.get_mut(name).and_then(|mut entry| {
            Self::reconcile_entry_status(&mut entry);
            if entry.status == FunctionStatus::Running {
                // Also check if isolate is still alive (hasn't panicked or exited)
                if let Some(ref handle) = entry.isolate_handle {
                    if handle.is_alive() {
                        return Some(handle.clone());
                    }
                }
            }
            None
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
    ) -> Result<FunctionInfo, Error> {
        // Destroy the old entry
        if let Some((_, old_entry)) = self.functions.remove(name) {
            info!("shutting down old isolate for function '{}'", name);
            lifecycle::destroy_function(&old_entry).await;
        }

        let config = config.unwrap_or_else(|| self.default_config.clone());

        info!("deploying updated function '{}'", name);

        let entry = lifecycle::create_function(
            name.to_string(),
            eszip_bytes.to_vec(),
            config,
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

        // Destroy old, recreate from same bytes
        if let Some((_, old_entry)) = self.functions.remove(name) {
            lifecycle::destroy_function(&old_entry).await;
        }

        info!("hot-reloading function '{}'", name);

        let entry = lifecycle::create_function(
            name.to_string(),
            eszip_bytes.to_vec(),
            config,
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
            inspector_stop: None,
            status: FunctionStatus::Running,
            config: IsolateConfig::default(),
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
            inspector_stop: None,
            status: FunctionStatus::Running,
            config: IsolateConfig::default(),
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
}

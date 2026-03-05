use anyhow::Error;
use bytes::Bytes;
use dashmap::DashMap;
use tokio_util::sync::CancellationToken;
use tracing::info;

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
            return Err(anyhow::anyhow!("function '{}' already exists, use PUT to update", name));
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
    pub fn get_handle(
        &self,
        name: &str,
    ) -> Option<runtime_core::isolate::IsolateHandle> {
        self.functions.get(name).and_then(|entry| {
            if entry.status == FunctionStatus::Running {
                entry.isolate_handle.clone()
            } else {
                None
            }
        })
    }

    /// Get the config for a function.
    pub fn get_config(&self, name: &str) -> Option<IsolateConfig> {
        self.functions.get(name).map(|entry| entry.config.clone())
    }

    /// List all functions.
    pub fn list(&self) -> Vec<FunctionInfo> {
        self.functions
            .iter()
            .map(|entry| entry.value().to_info())
            .collect()
    }

    /// Get info about a specific function.
    pub fn get_info(&self, name: &str) -> Option<FunctionInfo> {
        self.functions.get(name).map(|entry| entry.value().to_info())
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
        info!("shutting down all functions ({} total)", self.functions.len());
        self.global_shutdown.cancel();
        // Give isolates time to drain
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
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
}

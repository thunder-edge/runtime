use anyhow::Error;
use bytes::Bytes;
use chrono::Utc;
use dashmap::DashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use sysinfo::System;
use tokio::sync::{Mutex, Notify};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use uuid::Uuid;

use runtime_core::isolate::IsolateConfig;
use runtime_core::isolate::IsolateHandle;
use runtime_core::isolate::OutgoingProxyConfig;
use runtime_core::manifest::ResolvedFunctionManifest;

use crate::lifecycle;
use crate::types::*;

const CONTEXT_SCALE_DOWN_COOLDOWN: Duration = Duration::from_secs(5);
const ISOLATE_SCALE_DOWN_COOLDOWN: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy)]
struct EffectiveScalingLimits {
    isolates: PoolLimits,
    contexts: ContextPoolLimits,
}

#[derive(Debug, Clone)]
pub struct PoolRuntimeConfig {
    pub enabled: bool,
    pub global_max_isolates: usize,
    pub min_free_memory_mib: u64,
    pub capacity_wait_timeout_ms: u64,
    pub capacity_wait_max_waiters: usize,
    pub outgoing_proxy: OutgoingProxyConfig,
}

impl Default for PoolRuntimeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            global_max_isolates: 1024,
            min_free_memory_mib: 256,
            capacity_wait_timeout_ms: 300,
            capacity_wait_max_waiters: 20_000,
            outgoing_proxy: OutgoingProxyConfig::default(),
        }
    }
}

#[derive(Clone)]
pub struct RouteTarget {
    pub function_name: String,
    pub context_id: String,
    pub isolate_id: Uuid,
    pub handle: IsolateHandle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteTargetError {
    FunctionUnavailable,
    CapacityExhausted,
}

#[derive(Debug, Clone, Default)]
pub struct RoutingMetricsSnapshot {
    pub total_contexts: u64,
    pub total_isolates: u64,
    pub global_pool_total_isolates: u64,
    pub global_pool_max_isolates: u64,
    pub total_active_requests: u64,
    pub saturated_rejections: u64,
    pub saturated_rejections_context_capacity: u64,
    pub saturated_rejections_scale_blocked: u64,
    pub saturated_rejections_scale_failed: u64,
    pub burst_scale_batch_last: u64,
    pub burst_scale_events_total: u64,
    pub saturated_contexts: u64,
    pub saturated_isolates: u64,
}

#[derive(Debug, Clone)]
struct ContextRouteEntry {
    context_id: String,
    isolate_id: Uuid,
    active_requests: u64,
    draining: bool,
    idle_since: Option<Instant>,
}

#[derive(Debug, Clone, Default)]
struct FunctionRouteState {
    entries: Vec<ContextRouteEntry>,
    next_context_seq: u64,
}

/// Thread-safe registry of all deployed functions.
pub struct FunctionRegistry {
    functions: DashMap<String, FunctionEntry>,
    global_shutdown: CancellationToken,
    default_config: IsolateConfig,
    pool_config: PoolRuntimeConfig,
    default_pool_limits: PoolLimits,
    default_context_pool_limits: ContextPoolLimits,
    context_pool_limits: DashMap<String, ContextPoolLimits>,
    usage_clock: AtomicU64,
    handle_last_used: DashMap<Uuid, u64>,
    handle_last_seen_at: DashMap<Uuid, Instant>,
    route_state: DashMap<String, FunctionRouteState>,
    saturated_rejections: AtomicU64,
    saturated_rejections_context_capacity: AtomicU64,
    saturated_rejections_scale_blocked: AtomicU64,
    saturated_rejections_scale_failed: AtomicU64,
    burst_scale_batch_last: AtomicU64,
    burst_scale_events_total: AtomicU64,
    capacity_waiters: AtomicU64,
    capacity_notify: Notify,
    scale_lock: Mutex<()>,
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

        if let Some(vfs_total_quota_bytes) = manifest.resources.vfs_total_quota_bytes {
            config.vfs_total_quota_bytes = vfs_total_quota_bytes as usize;
        }

        if let Some(vfs_max_file_bytes) = manifest.resources.vfs_max_file_bytes {
            config.vfs_max_file_bytes = vfs_max_file_bytes as usize;
        }

        if let Some(egress_max_requests_per_execution) =
            manifest.resources.egress_max_requests_per_execution
        {
            config.egress_max_requests_per_execution = egress_max_requests_per_execution as usize;
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

    fn reconcile_entry_status(entry: &mut FunctionEntry) -> Vec<Uuid> {
        let mut removed_handles = Vec::new();

        let mut alive_extras = Vec::with_capacity(entry.extra_isolate_handles.len());
        for handle in entry.extra_isolate_handles.drain(..) {
            if handle.is_alive() {
                alive_extras.push(handle);
            } else {
                removed_handles.push(handle.id);
            }
        }
        entry.extra_isolate_handles = alive_extras;

        let mut alive_count = entry
            .extra_isolate_handles
            .iter()
            .filter(|handle| handle.is_alive())
            .count();

        if let Some(handle) = &entry.isolate_handle {
            if handle.is_alive() {
                alive_count += 1;
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

        removed_handles
    }

    fn next_usage_tick(&self) -> u64 {
        self.usage_clock
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1)
    }

    fn mark_handle_used(&self, handle_id: Uuid) {
        let now = Instant::now();
        self.handle_last_used
            .insert(handle_id, self.next_usage_tick());
        self.handle_last_seen_at.insert(handle_id, now);
    }

    fn mark_entry_handles_used(&self, entry: &FunctionEntry) {
        if let Some(handle) = &entry.isolate_handle {
            self.mark_handle_used(handle.id);
        }
        for handle in &entry.extra_isolate_handles {
            self.mark_handle_used(handle.id);
        }
    }

    fn remove_handle_usage(&self, handle_id: Uuid) {
        self.handle_last_used.remove(&handle_id);
        self.handle_last_seen_at.remove(&handle_id);
    }

    fn remove_entry_handle_usage(&self, entry: &FunctionEntry) {
        if let Some(handle) = &entry.isolate_handle {
            self.remove_handle_usage(handle.id);
        }
        for handle in &entry.extra_isolate_handles {
            self.remove_handle_usage(handle.id);
        }
    }

    /// Evict one replica (never primary isolate) to free pool capacity.
    ///
    /// Policy:
    /// 1) Prefer functions other than the requester.
    /// 2) Prefer functions with lower active requests (cold first).
    /// 3) Within same activity bucket, evict least recently used handle.
    fn evict_lru_replica_for_capacity(&self, requester_function: &str) -> bool {
        let isolate_has_in_flight_requests = |function_name: &str, isolate_id: Uuid| -> bool {
            self.route_state
                .get(function_name)
                .map(|state| {
                    state
                        .entries
                        .iter()
                        .any(|entry| entry.isolate_id == isolate_id && entry.active_requests > 0)
                })
                .unwrap_or(false)
        };

        let mut function_active_requests: std::collections::HashMap<String, u64> =
            std::collections::HashMap::new();
        for state in self.route_state.iter() {
            let total_active = state
                .entries
                .iter()
                .map(|entry| entry.active_requests)
                .sum::<u64>();
            function_active_requests.insert(state.key().clone(), total_active);
        }

        let mut candidate_function = None::<String>;
        let mut candidate_handle = None::<Uuid>;
        let mut candidate_active_requests = u64::MAX;
        let mut candidate_tick = u64::MAX;
        let mut candidate_is_requester = true;

        for entry in self.functions.iter() {
            if entry.current_pool_size() <= entry.pool_limits.min {
                continue;
            }

            let function_name = entry.key();
            let is_requester = function_name == requester_function;
            let active_requests = *function_active_requests.get(function_name).unwrap_or(&0);

            for handle in &entry.extra_isolate_handles {
                if !handle.is_alive() {
                    continue;
                }

                if isolate_has_in_flight_requests(function_name, handle.id) {
                    continue;
                }

                let tick = self
                    .handle_last_used
                    .get(&handle.id)
                    .map(|value| *value)
                    .unwrap_or(0);

                let should_replace =
                    (candidate_function.is_none())
                        || (candidate_is_requester && !is_requester)
                        || (candidate_is_requester == is_requester
                            && (active_requests < candidate_active_requests
                                || (active_requests == candidate_active_requests
                                    && tick < candidate_tick)));

                if should_replace {
                    candidate_function = Some(function_name.clone());
                    candidate_handle = Some(handle.id);
                    candidate_active_requests = active_requests;
                    candidate_tick = tick;
                    candidate_is_requester = is_requester;
                }
            }
        }

        let (Some(function_name), Some(handle_id)) = (candidate_function, candidate_handle) else {
            return false;
        };

        let Some(mut entry) = self.functions.get_mut(&function_name) else {
            return false;
        };

        let Some(pos) = entry
            .extra_isolate_handles
            .iter()
            .position(|h| h.id == handle_id)
        else {
            return false;
        };

        let handle = entry.extra_isolate_handles.remove(pos);
        self.remove_handle_usage(handle.id);
        handle.shutdown.cancel();
        handle.close_request_tx();
        info!(
            function_name = %function_name,
            request_id = "system",
            evicted_active_requests = candidate_active_requests,
            requester_function = %requester_function,
            "evicted LRU replica '{}' from function '{}' to free pool capacity (cold-first policy)",
            handle.id, function_name
        );
        true
    }

    pub fn new(global_shutdown: CancellationToken, default_config: IsolateConfig) -> Self {
        Self::new_with_pool(
            global_shutdown,
            default_config,
            PoolRuntimeConfig::default(),
            PoolLimits::default(),
            ContextPoolLimits::default(),
        )
    }

    pub fn new_with_pool(
        global_shutdown: CancellationToken,
        default_config: IsolateConfig,
        pool_config: PoolRuntimeConfig,
        default_pool_limits: PoolLimits,
        default_context_pool_limits: ContextPoolLimits,
    ) -> Self {
        Self {
            functions: DashMap::new(),
            global_shutdown,
            default_config,
            pool_config,
            default_pool_limits,
            default_context_pool_limits,
            context_pool_limits: DashMap::new(),
            usage_clock: AtomicU64::new(0),
            handle_last_used: DashMap::new(),
            handle_last_seen_at: DashMap::new(),
            route_state: DashMap::new(),
            saturated_rejections: AtomicU64::new(0),
            saturated_rejections_context_capacity: AtomicU64::new(0),
            saturated_rejections_scale_blocked: AtomicU64::new(0),
            saturated_rejections_scale_failed: AtomicU64::new(0),
            burst_scale_batch_last: AtomicU64::new(0),
            burst_scale_events_total: AtomicU64::new(0),
            capacity_waiters: AtomicU64::new(0),
            capacity_notify: Notify::new(),
            scale_lock: Mutex::new(()),
        }
    }

    fn normalize_pool_limits(&self, limits: PoolLimits) -> PoolLimits {
        let min = limits.min.max(1);
        let max = limits.max.max(min);
        PoolLimits { min, max }
    }

    fn normalize_context_limits(&self, limits: ContextPoolLimits) -> ContextPoolLimits {
        let min = limits.min.max(1);
        let max = limits.max.max(min);
        ContextPoolLimits { min, max }
    }

    fn effective_scaling_limits(&self, name: &str, configured_pool: PoolLimits) -> EffectiveScalingLimits {
        let isolates = self.normalize_pool_limits(configured_pool);
        let contexts = self
            .context_pool_limits
            .get(name)
            .map(|value| self.normalize_context_limits(*value))
            .unwrap_or_else(|| self.normalize_context_limits(self.default_context_pool_limits));

        EffectiveScalingLimits { isolates, contexts }
    }

    fn current_total_isolates(&self) -> usize {
        self.functions
            .iter()
            .map(|entry| {
                let base = entry
                    .isolate_handle
                    .as_ref()
                    .map(|h| usize::from(h.is_alive()))
                    .unwrap_or(0);
                let extras = entry
                    .extra_isolate_handles
                    .iter()
                    .filter(|h| h.is_alive())
                    .count();
                base + extras
            })
            .sum()
    }

    fn can_scale_with_memory(&self, function_name: &str) -> bool {
        let mut sys = System::new_all();
        sys.refresh_memory();
        // Some environments can transiently report 0 for both available/free memory.
        // Treat that as unknown signal to avoid false-positive scale blocking.
        let available_bytes = sys.available_memory().max(sys.free_memory());
        if available_bytes == 0 {
            warn!(
                function_name = %function_name,
                request_id = "system",
                "pool memory guardrail skipped for '{}' because available memory is unknown (reported as 0)",
                function_name
            );
            return true;
        }

        let available_mib = available_bytes / (1024 * 1024);
        if available_mib < self.pool_config.min_free_memory_mib {
            warn!(
                function_name = %function_name,
                request_id = "system",
                "pool scale blocked for '{}' due to low memory (available={}MiB, min_required={}MiB)",
                function_name, available_mib, self.pool_config.min_free_memory_mib
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
        // Serialize scale-up decisions to avoid overshooting global isolate cap
        // under concurrent bursts.
        let _scale_guard = self.scale_lock.lock().await;

        if !self.pool_config.enabled {
            return Ok(None);
        }

        if self.current_total_isolates() >= self.pool_config.global_max_isolates {
            if !self.evict_lru_replica_for_capacity(function_name) {
                warn!(
                    function_name = %function_name,
                    request_id = "system",
                    "pool scale blocked for '{}' due to global isolate limit ({}) and no evictable replica",
                    function_name, self.pool_config.global_max_isolates
                );
                return Ok(None);
            }

            if self.current_total_isolates() >= self.pool_config.global_max_isolates {
                warn!(
                    function_name = %function_name,
                    request_id = "system",
                    "pool scale blocked for '{}' after LRU eviction attempt (global limit: {})",
                    function_name, self.pool_config.global_max_isolates
                );
                return Ok(None);
            }
        }

        if !self.can_scale_with_memory(function_name) {
            return Ok(None);
        }

        let replica_entry = lifecycle::create_function(
            function_name.to_string(),
            eszip_bytes.to_vec(),
            config,
            self.pool_config.outgoing_proxy.clone(),
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

        info!(function_name = %name, request_id = "system", "deploying function '{}'", name);

        let entry = lifecycle::create_function(
            name.clone(),
            eszip_bytes.to_vec(),
            config,
            self.pool_config.outgoing_proxy.clone(),
            manifest,
            self.global_shutdown.child_token(),
        )
        .await?;

        let mut entry = entry;
        entry.pool_limits = if self.pool_config.enabled {
            self.normalize_pool_limits(self.default_pool_limits)
        } else {
            PoolLimits::default()
        };

        self.context_pool_limits.insert(
            name.clone(),
            self.normalize_context_limits(self.default_context_pool_limits),
        );

        if self.pool_config.enabled && entry.pool_limits.max > 1 {
            while entry.current_pool_size() < entry.pool_limits.min
                && entry.current_pool_size() < entry.pool_limits.max
            {
                match self
                    .create_replica_handle(
                        &name,
                        entry.bundle_package_bytes.clone(),
                        entry.config.clone(),
                        entry.manifest.clone(),
                    )
                    .await
                {
                    Ok(Some(handle)) => entry.extra_isolate_handles.push(handle),
                    Ok(None) => break,
                    Err(err) => {
                        warn!(function_name = %name, request_id = "system", "failed to pre-warm replica for '{}': {}", name, err);
                        break;
                    }
                }
            }
        }

        let info = entry.to_info();
        self.mark_entry_handles_used(&entry);
        self.route_state.remove(&name);
        self.functions.insert(name, entry);
        Ok(info)
    }

    /// Get a handle to route a request to.
    /// Returns None if function doesn't exist, isn't running, or the isolate is dead.
    pub fn get_handle(&self, name: &str) -> Option<runtime_core::isolate::IsolateHandle> {
        self.functions.get_mut(name).and_then(|mut entry| {
            let removed_handles = Self::reconcile_entry_status(&mut entry);
            for handle_id in removed_handles {
                self.remove_handle_usage(handle_id);
            }
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
            let selected = handles[idx].clone();
            self.mark_handle_used(selected.id);
            Some(selected)
        })
    }

    fn compute_route_target(&self, name: &str) -> Result<RouteTarget, RouteTargetError> {
        const BURST_ISOLATE_PREFERENCE_WAITERS: u64 = 64;

        let (handles, config, configured_pool_limits) = {
            let Some(mut entry) = self.functions.get_mut(name) else {
                return Err(RouteTargetError::FunctionUnavailable);
            };
            let removed_handles = Self::reconcile_entry_status(&mut entry);
            for handle_id in removed_handles {
                self.remove_handle_usage(handle_id);
            }
            if entry.status != FunctionStatus::Running {
                return Err(RouteTargetError::FunctionUnavailable);
            }

            let mut alive_handles: Vec<IsolateHandle> = Vec::new();
            if let Some(handle) = &entry.isolate_handle {
                if handle.is_alive() {
                    alive_handles.push(handle.clone());
                }
            }
            for handle in &entry.extra_isolate_handles {
                if handle.is_alive() {
                    alive_handles.push(handle.clone());
                }
            }

            (alive_handles, entry.config.clone(), entry.pool_limits)
        };

        if handles.is_empty() {
            return Err(RouteTargetError::FunctionUnavailable);
        }

        let scaling_limits = self.effective_scaling_limits(name, configured_pool_limits);

        let handle_by_id: std::collections::HashMap<Uuid, IsolateHandle> =
            handles.iter().cloned().map(|h| (h.id, h)).collect();

        let mut state = self
            .route_state
            .entry(name.to_string())
            .or_insert_with(FunctionRouteState::default);

        // Remove entries that reference dead or missing isolates.
        state
            .entries
            .retain(|entry| handle_by_id.contains_key(&entry.isolate_id) && !entry.draining);

        // Always keep at least one logical context per function, and pre-warm up to min contexts.
        while state.entries.len() < scaling_limits.contexts.min {
            let mut contexts_per_isolate: std::collections::HashMap<Uuid, usize> =
                std::collections::HashMap::new();
            for entry in &state.entries {
                *contexts_per_isolate.entry(entry.isolate_id).or_insert(0) += 1;
            }

            let candidate = handles
                .iter()
                .filter(|handle| {
                    let current = *contexts_per_isolate.get(&handle.id).unwrap_or(&0);
                    current < config.max_contexts_per_isolate.max(1)
                })
                .min_by_key(|handle| {
                    let current = *contexts_per_isolate.get(&handle.id).unwrap_or(&0);
                    (current, handle.id)
                })
                .cloned();

            let Some(handle) = candidate else {
                break;
            };

            let context_id = format!("ctx-{}-{}", name, state.next_context_seq);
            state.next_context_seq = state.next_context_seq.saturating_add(1);
            state.entries.push(ContextRouteEntry {
                context_id,
                isolate_id: handle.id,
                active_requests: 0,
                draining: false,
                idle_since: Some(Instant::now()),
            });
        }

        if state.entries.is_empty() {
            if let Some(handle) = handles.first() {
                let context_id = format!("ctx-{}-{}", name, state.next_context_seq);
                state.next_context_seq = state.next_context_seq.saturating_add(1);
                state.entries.push(ContextRouteEntry {
                    context_id,
                    isolate_id: handle.id,
                    active_requests: 0,
                    draining: false,
                    idle_since: Some(Instant::now()),
                });
            }
        }

        let max_active = if config.max_active_requests_per_context == 0 {
            u64::MAX
        } else {
            config.max_active_requests_per_context as u64
        };

        // Under heavy queue pressure, prefer isolate scale-out earlier instead of
        // consuming all remaining context slots in existing isolates.
        let prefer_isolate_scale_under_burst = self.capacity_waiters.load(Ordering::Relaxed)
            >= BURST_ISOLATE_PREFERENCE_WAITERS;

        // If context pool is enabled and all contexts are saturated, add a new context first.
        if config.context_pool_enabled
            && !prefer_isolate_scale_under_burst
            && state
                .entries
                .iter()
                .all(|entry| entry.active_requests >= max_active)
            && state.entries.len() < scaling_limits.contexts.max
        {
            let mut contexts_per_isolate: std::collections::HashMap<Uuid, usize> =
                std::collections::HashMap::new();
            for entry in &state.entries {
                *contexts_per_isolate.entry(entry.isolate_id).or_insert(0) += 1;
            }

            let candidate = handles
                .iter()
                .filter(|handle| {
                    let current = *contexts_per_isolate.get(&handle.id).unwrap_or(&0);
                    current < config.max_contexts_per_isolate.max(1)
                })
                .min_by_key(|handle| {
                    let current = *contexts_per_isolate.get(&handle.id).unwrap_or(&0);
                    (current, handle.id)
                })
                .cloned();

            if let Some(handle) = candidate {
                let context_id = format!("ctx-{}-{}", name, state.next_context_seq);
                state.next_context_seq = state.next_context_seq.saturating_add(1);
                state.entries.push(ContextRouteEntry {
                    context_id,
                    isolate_id: handle.id,
                    active_requests: 0,
                    draining: false,
                    idle_since: Some(Instant::now()),
                });
            }
        }

        let chosen_index = state
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| !entry.draining)
            .filter(|(_, entry)| entry.active_requests < max_active)
            .min_by_key(|(_, entry)| (entry.active_requests, entry.context_id.clone()))
            .map(|(idx, _)| idx);

        let chosen_index = if config.context_pool_enabled {
            match chosen_index {
                Some(idx) => Some(idx),
                None => {
                    self.saturated_rejections.fetch_add(1, Ordering::Relaxed);
                    self
                        .saturated_rejections_context_capacity
                        .fetch_add(1, Ordering::Relaxed);
                    None
                }
            }
        } else {
            chosen_index.or_else(|| {
                state
                    .entries
                    .iter()
                    .enumerate()
                    .filter(|(_, entry)| !entry.draining)
                    .min_by_key(|(_, entry)| (entry.active_requests, entry.context_id.clone()))
                    .map(|(idx, _)| idx)
            })
        };

        let Some(chosen_index) = chosen_index else {
            return Err(RouteTargetError::CapacityExhausted);
        };

        let chosen = &mut state.entries[chosen_index];
        if chosen.active_requests == 0 {
            chosen.idle_since = None;
        }
        chosen.active_requests = chosen.active_requests.saturating_add(1);

        let isolate_id = chosen.isolate_id;
        let context_id = chosen.context_id.clone();
        drop(state);

        let Some(handle) = handle_by_id.get(&isolate_id).cloned() else {
            return Err(RouteTargetError::FunctionUnavailable);
        };
        self.mark_handle_used(handle.id);

        Ok(RouteTarget {
            function_name: name.to_string(),
            context_id,
            isolate_id,
            handle,
        })
    }

    /// Resolve a context-aware route target using context-first scheduling.
    pub fn get_route_target(&self, name: &str) -> Option<RouteTarget> {
        self.compute_route_target(name).ok()
    }

    /// Resolve a route target and distinguish between unavailable functions and capacity saturation.
    pub async fn get_route_target_with_status(
        &self,
        name: &str,
    ) -> Result<RouteTarget, RouteTargetError> {
        let queue_timeout = Duration::from_millis(self.pool_config.capacity_wait_timeout_ms);
        if queue_timeout.is_zero() {
            return self.get_route_target_with_status_once(name).await;
        }

        let deadline = Instant::now() + queue_timeout;
        loop {
            match self.get_route_target_with_status_once(name).await {
                Ok(target) => return Ok(target),
                Err(RouteTargetError::FunctionUnavailable) => {
                    return Err(RouteTargetError::FunctionUnavailable)
                }
                Err(RouteTargetError::CapacityExhausted) => {
                    let now = Instant::now();
                    if now >= deadline {
                        return Err(RouteTargetError::CapacityExhausted);
                    }

                    if self.capacity_waiters.load(Ordering::Relaxed)
                        >= self.pool_config.capacity_wait_max_waiters as u64
                    {
                        return Err(RouteTargetError::CapacityExhausted);
                    }

                    self.capacity_waiters.fetch_add(1, Ordering::Relaxed);
                    let remaining = deadline.saturating_duration_since(now);
                    let wait_for = remaining.min(Duration::from_millis(10));
                    let _ = tokio::time::timeout(wait_for, self.capacity_notify.notified()).await;
                    self.capacity_waiters.fetch_sub(1, Ordering::Relaxed);
                }
            }
        }
    }

    async fn get_route_target_with_status_once(
        &self,
        name: &str,
    ) -> Result<RouteTarget, RouteTargetError> {
        const BURST_WAITERS_MEDIUM: u64 = 16;
        const BURST_WAITERS_HIGH: u64 = 64;
        const BURST_WAITERS_EXTREME: u64 = 256;

        match self.compute_route_target(name) {
            Ok(target) => Ok(target),
            Err(RouteTargetError::FunctionUnavailable) => {
                Err(RouteTargetError::FunctionUnavailable)
            }
            Err(RouteTargetError::CapacityExhausted) => {
                let waiter_count = self.capacity_waiters.load(Ordering::Relaxed);
                let burst_scale_batch = if waiter_count >= BURST_WAITERS_EXTREME {
                    16_usize
                } else if waiter_count >= BURST_WAITERS_HIGH {
                    8_usize
                } else if waiter_count >= BURST_WAITERS_MEDIUM {
                    4_usize
                } else {
                    1_usize
                };
                self.burst_scale_batch_last
                    .store(burst_scale_batch as u64, Ordering::Relaxed);
                if burst_scale_batch > 1 {
                    self.burst_scale_events_total.fetch_add(1, Ordering::Relaxed);
                }

                let scale_plan = {
                    let Some(mut entry) = self.functions.get_mut(name) else {
                        return Err(RouteTargetError::FunctionUnavailable);
                    };
                    let removed_handles = Self::reconcile_entry_status(&mut entry);
                    for handle_id in removed_handles {
                        self.remove_handle_usage(handle_id);
                    }
                    let scaling_limits = self.effective_scaling_limits(name, entry.pool_limits);

                    if entry.status != FunctionStatus::Running
                        || !self.pool_config.enabled
                        || !entry.config.context_pool_enabled
                        || entry.current_pool_size() >= scaling_limits.isolates.max
                    {
                        self
                            .saturated_rejections_scale_blocked
                            .fetch_add(1, Ordering::Relaxed);
                        None
                    } else {
                        Some((
                            entry.bundle_package_bytes.clone(),
                            entry.config.clone(),
                            entry.manifest.clone(),
                            scaling_limits.isolates.max,
                        ))
                    }
                };

                if let Some((bundle_package_bytes, config, manifest, per_function_max)) = scale_plan {
                    let mut created = 0_usize;
                    for _ in 0..burst_scale_batch {
                        let current_pool_size = self
                            .functions
                            .get(name)
                            .map(|entry| entry.current_pool_size())
                            .unwrap_or(0);
                        if current_pool_size >= per_function_max {
                            break;
                        }

                        match self
                            .create_replica_handle(
                                name,
                                bundle_package_bytes.clone(),
                                config.clone(),
                                manifest.clone(),
                            )
                            .await
                        {
                            Ok(Some(handle)) => {
                                if let Some(mut entry) = self.functions.get_mut(name) {
                                    entry.extra_isolate_handles.push(handle.clone());
                                }
                                self.mark_handle_used(handle.id);
                                created = created.saturating_add(1);
                            }
                            Ok(None) => {
                                break;
                            }
                            Err(err) => {
                                self
                                    .saturated_rejections_scale_failed
                                    .fetch_add(1, Ordering::Relaxed);
                                warn!(
                                    function_name = %name,
                                    request_id = "system",
                                    "failed to scale isolate for route target: {}",
                                    err
                                );
                                break;
                            }
                        }
                    }

                    if created == 0 {
                        self
                            .saturated_rejections_scale_blocked
                            .fetch_add(1, Ordering::Relaxed);
                        Err(RouteTargetError::CapacityExhausted)
                    } else {
                        self.compute_route_target(name)
                    }
                } else {
                    Err(RouteTargetError::CapacityExhausted)
                }
            }
        }
    }

    /// Decrease active request counter for a previously acquired route target.
    pub fn release_route_target(&self, target: &RouteTarget) {
        let now = Instant::now();
        let Some(mut state) = self.route_state.get_mut(&target.function_name) else {
            return;
        };

        if let Some(entry) = state.entries.iter_mut().find(|entry| {
            entry.context_id == target.context_id && entry.isolate_id == target.isolate_id
        }) {
            entry.active_requests = entry.active_requests.saturating_sub(1);
            if entry.active_requests == 0 {
                entry.idle_since = Some(now);
            }
        }

        let context_limits = self
            .context_pool_limits
            .get(&target.function_name)
            .map(|value| self.normalize_context_limits(*value))
            .unwrap_or_else(|| self.normalize_context_limits(self.default_context_pool_limits));

        // Burst shrink: retire idle contexts above min.
        while state.entries.len() > context_limits.min {
            let removable = state
                .entries
                .iter()
                .enumerate()
                .filter(|(_, entry)| {
                    !entry.draining
                        && entry.active_requests == 0
                        && entry
                            .idle_since
                            .map(|since| now.saturating_duration_since(since) >= CONTEXT_SCALE_DOWN_COOLDOWN)
                            .unwrap_or(false)
                })
                .map(|(idx, entry)| (idx, entry.context_id.clone()))
                .max_by(|a, b| a.1.cmp(&b.1));

            let Some((idx, _)) = removable else {
                break;
            };

            state.entries.remove(idx);
        }

        let active_isolates: std::collections::HashSet<Uuid> =
            state.entries.iter().map(|entry| entry.isolate_id).collect();
        drop(state);

        // Scale down idle extra isolates above min isolate pool size.
        let mut removed_any = false;
        if let Some(mut entry) = self.functions.get_mut(&target.function_name) {
            let scaling_limits = self.effective_scaling_limits(&target.function_name, entry.pool_limits);
            while entry.current_pool_size() > scaling_limits.isolates.min {
                let removable_idx = entry
                    .extra_isolate_handles
                    .iter()
                    .enumerate()
                    .find(|(_, handle)| {
                        if active_isolates.contains(&handle.id) {
                            return false;
                        }
                        self.handle_last_seen_at
                            .get(&handle.id)
                            .map(|seen_at| {
                                now.saturating_duration_since(*seen_at) >= ISOLATE_SCALE_DOWN_COOLDOWN
                            })
                            .unwrap_or(false)
                    })
                    .map(|(idx, _)| idx);

                let Some(idx) = removable_idx else {
                    break;
                };

                let handle = entry.extra_isolate_handles.remove(idx);
                self.remove_handle_usage(handle.id);
                handle.shutdown.cancel();
                handle.close_request_tx();
                removed_any = true;
            }
        }

        if removed_any {
            self.route_state.remove(&target.function_name);
        }

        self.capacity_notify.notify_one();
    }

    pub fn routing_metrics_snapshot(&self) -> RoutingMetricsSnapshot {
        let mut total_contexts = 0_u64;
        let mut total_isolates = 0_u64;
        let mut total_active_requests = 0_u64;
        let mut saturated_contexts = 0_u64;
        let mut saturated_isolates = 0_u64;

        for function_state in self.route_state.iter() {
            let Some(function_entry) = self.functions.get(function_state.key()) else {
                continue;
            };

            let max_active = if function_entry.config.max_active_requests_per_context == 0 {
                u64::MAX
            } else {
                function_entry.config.max_active_requests_per_context as u64
            };
            let max_contexts_per_isolate = function_entry.config.max_contexts_per_isolate.max(1);

            let mut isolate_rollup: std::collections::HashMap<Uuid, (usize, usize)> =
                std::collections::HashMap::new();

            for entry in &function_state.entries {
                total_contexts = total_contexts.saturating_add(1);
                total_active_requests = total_active_requests.saturating_add(entry.active_requests);

                let is_context_saturated = entry.active_requests >= max_active;
                if is_context_saturated {
                    saturated_contexts = saturated_contexts.saturating_add(1);
                }

                let rollup = isolate_rollup.entry(entry.isolate_id).or_insert((0, 0));
                rollup.0 = rollup.0.saturating_add(1);
                if is_context_saturated {
                    rollup.1 = rollup.1.saturating_add(1);
                }
            }

            total_isolates = total_isolates.saturating_add(isolate_rollup.len() as u64);

            if function_entry.config.context_pool_enabled {
                for (_isolate_id, (contexts, saturated)) in isolate_rollup {
                    let isolate_at_context_limit = contexts >= max_contexts_per_isolate;
                    let isolate_fully_saturated = contexts > 0 && contexts == saturated;
                    if isolate_at_context_limit && isolate_fully_saturated {
                        saturated_isolates = saturated_isolates.saturating_add(1);
                    }
                }
            }
        }

        RoutingMetricsSnapshot {
            total_contexts,
            total_isolates,
            global_pool_total_isolates: self.current_total_isolates() as u64,
            global_pool_max_isolates: self.pool_config.global_max_isolates as u64,
            total_active_requests,
            saturated_rejections: self.saturated_rejections.load(Ordering::Relaxed),
            saturated_rejections_context_capacity: self
                .saturated_rejections_context_capacity
                .load(Ordering::Relaxed),
            saturated_rejections_scale_blocked: self
                .saturated_rejections_scale_blocked
                .load(Ordering::Relaxed),
            saturated_rejections_scale_failed: self
                .saturated_rejections_scale_failed
                .load(Ordering::Relaxed),
            burst_scale_batch_last: self.burst_scale_batch_last.load(Ordering::Relaxed),
            burst_scale_events_total: self.burst_scale_events_total.load(Ordering::Relaxed),
            saturated_contexts,
            saturated_isolates,
        }
    }

    /// Get the config for a function.
    pub fn get_config(&self, name: &str) -> Option<IsolateConfig> {
        self.functions.get(name).map(|entry| entry.config.clone())
    }

    /// Get the resolved manifest policy currently attached to a function.
    pub fn get_resolved_manifest(&self, name: &str) -> Option<ResolvedFunctionManifest> {
        self.functions
            .get(name)
            .and_then(|entry| entry.manifest.clone())
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
            let removed_handles = Self::reconcile_entry_status(&mut entry);
            for handle_id in removed_handles {
                self.remove_handle_usage(handle_id);
            }
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
        let old_manifest = self
            .functions
            .get(name)
            .and_then(|entry| entry.manifest.clone());
        let old_context_limits = self
            .context_pool_limits
            .get(name)
            .map(|value| *value)
            .unwrap_or(self.default_context_pool_limits);

        // Destroy the old entry
        if let Some((_, old_entry)) = self.functions.remove(name) {
            info!(function_name = %name, request_id = "system", "shutting down old isolate for function '{}'", name);
            self.remove_entry_handle_usage(&old_entry);
            self.route_state.remove(name);
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

        info!(function_name = %name, request_id = "system", "deploying updated function '{}'", name);

        let entry = lifecycle::create_function(
            name.to_string(),
            eszip_bytes.to_vec(),
            config,
            self.pool_config.outgoing_proxy.clone(),
            manifest,
            self.global_shutdown.child_token(),
        )
        .await?;

        let info = entry.to_info();
        self.mark_entry_handles_used(&entry);
        self.route_state.remove(name);
        self.context_pool_limits.remove(name);
        self.context_pool_limits.insert(
            name.to_string(),
            self.normalize_context_limits(old_context_limits),
        );
        self.functions.insert(name.to_string(), entry);
        Ok(info)
    }

    /// Delete a function entirely.
    pub async fn delete(&self, name: &str) -> Result<(), Error> {
        if let Some((_, entry)) = self.functions.remove(name) {
            info!(function_name = %name, request_id = "system", "deleting function '{}'", name);
            self.remove_entry_handle_usage(&entry);
            self.route_state.remove(name);
            self.context_pool_limits.remove(name);
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
        let context_limits = self
            .context_pool_limits
            .get(name)
            .map(|value| *value)
            .unwrap_or(self.default_context_pool_limits);

        // Destroy old, recreate from same bytes
        if let Some((_, old_entry)) = self.functions.remove(name) {
            self.remove_entry_handle_usage(&old_entry);
            self.route_state.remove(name);
            self.context_pool_limits.remove(name);
            for extra in &old_entry.extra_isolate_handles {
                extra.shutdown.cancel();
                extra.close_request_tx();
            }
            lifecycle::destroy_function(&old_entry).await;
        }

        info!(function_name = %name, request_id = "system", "hot-reloading function '{}'", name);

        let entry = lifecycle::create_function(
            name.to_string(),
            eszip_bytes.to_vec(),
            config,
            self.pool_config.outgoing_proxy.clone(),
            manifest,
            self.global_shutdown.child_token(),
        )
        .await?;

        let info = entry.to_info();
        self.mark_entry_handles_used(&entry);
        self.context_pool_limits
            .insert(name.to_string(), self.normalize_context_limits(context_limits));
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
            function_name = "runtime",
            request_id = "system",
            "shutting down all functions ({} total, deadline={}ms)",
            total,
            deadline.as_millis()
        );

        self.global_shutdown.cancel();

        for mut entry in self.functions.iter_mut() {
            entry.status = FunctionStatus::ShuttingDown;
            entry.updated_at = Utc::now();
            if let Some(handle) = &entry.isolate_handle {
                self.remove_handle_usage(handle.id);
                handle.shutdown.cancel();
            }
            for handle in &entry.extra_isolate_handles {
                self.remove_handle_usage(handle.id);
                handle.shutdown.cancel();
                handle.close_request_tx();
            }
        }

        let started = std::time::Instant::now();
        while started.elapsed() < deadline {
            if self.all_request_channels_closed() {
                self.functions.clear();
                self.handle_last_used.clear();
                self.handle_last_seen_at.clear();
                self.route_state.clear();
                self.context_pool_limits.clear();
                info!(
                    function_name = "runtime",
                    request_id = "system",
                    "all function channels closed before shutdown deadline"
                );
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
                function_name = "runtime",
                request_id = "system",
                "shutdown deadline reached with {} function(s) still open; forcing clear",
                still_open
            );
        }

        self.functions.clear();
        self.handle_last_used.clear();
        self.handle_last_seen_at.clear();
        self.route_state.clear();
        self.context_pool_limits.clear();
    }

    /// Number of deployed functions.
    pub fn count(&self) -> usize {
        self.functions.len()
    }

    pub fn get_pool_limits(&self, name: &str) -> Option<PoolLimits> {
        self.functions.get(name).map(|entry| entry.pool_limits)
    }

    pub fn get_context_pool_limits(&self, name: &str) -> Option<ContextPoolLimits> {
        if !self.functions.contains_key(name) {
            return None;
        }
        Some(
            self.context_pool_limits
                .get(name)
                .map(|value| self.normalize_context_limits(*value))
                .unwrap_or_else(|| self.normalize_context_limits(self.default_context_pool_limits)),
        )
    }

    pub async fn set_pool_limits(
        &self,
        name: &str,
        min: usize,
        max: usize,
    ) -> Result<FunctionInfo, Error> {
        let current_context = self
            .get_context_pool_limits(name)
            .ok_or_else(|| anyhow::anyhow!("function '{}' not found", name))?;
        self.set_scaling_limits(name, min, max, current_context.min, current_context.max)
            .await
    }

    pub async fn set_scaling_limits(
        &self,
        name: &str,
        isolate_min: usize,
        isolate_max: usize,
        context_min: usize,
        context_max: usize,
    ) -> Result<FunctionInfo, Error> {
        if isolate_min > isolate_max {
            return Err(anyhow::anyhow!("invalid isolate pool limits: min must be <= max"));
        }

        if context_min > context_max {
            return Err(anyhow::anyhow!("invalid context pool limits: min must be <= max"));
        }

        let Some((key, mut entry)) = self.functions.remove(name) else {
            return Err(anyhow::anyhow!("function '{}' not found", name));
        };

        let next_isolate_limits = self.normalize_pool_limits(PoolLimits {
            min: isolate_min,
            max: isolate_max,
        });
        let next_context_limits = self.normalize_context_limits(ContextPoolLimits {
            min: context_min,
            max: context_max,
        });

        entry.pool_limits = if self.pool_config.enabled {
            next_isolate_limits
        } else {
            PoolLimits::default()
        };

        self.context_pool_limits
            .insert(name.to_string(), next_context_limits);

        // Shrink extra replicas above max (primary handle is always retained if alive).
        while entry.current_pool_size() > entry.pool_limits.max {
            if let Some(extra) = entry.extra_isolate_handles.pop() {
                self.remove_handle_usage(extra.id);
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
                        entry.bundle_package_bytes.clone(),
                        entry.config.clone(),
                        entry.manifest.clone(),
                    )
                    .await
                {
                    Ok(Some(handle)) => {
                        self.mark_handle_used(handle.id);
                        entry.extra_isolate_handles.push(handle)
                    }
                    Ok(None) => break,
                    Err(err) => {
                        warn!(function_name = %name, request_id = "system", "failed to scale pool for '{}': {}", name, err);
                        break;
                    }
                }
            }
        }

        let info = entry.to_info();
        self.mark_entry_handles_used(&entry);
        if let Some(mut state) = self.route_state.get_mut(name) {
            while state.entries.len() > next_context_limits.max {
                if let Some(idx) = state
                    .entries
                    .iter()
                    .enumerate()
                    .filter(|(_, entry)| entry.active_requests == 0)
                    .map(|(idx, _)| idx)
                    .next_back()
                {
                    state.entries.remove(idx);
                } else {
                    break;
                }
            }
        }
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
                capacity_wait_timeout_ms: 75,
                capacity_wait_max_waiters: 20_000,
                outgoing_proxy: OutgoingProxyConfig::default(),
            },
            PoolLimits::default(),
            ContextPoolLimits::default(),
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
            bundle_package_bytes: Bytes::new(),
            eszip_bytes: Bytes::new(),
            bundle_format: BundleFormat::Eszip,
            package_v8_version: deno_core::v8::VERSION_STRING.to_string(),
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
    fn dead_primary_handle_can_transition_back_to_running() {
        let reg = make_registry();

        let (request_tx, _request_rx) = tokio::sync::mpsc::unbounded_channel();
        let alive = Arc::new(AtomicBool::new(false));
        let handle = runtime_core::isolate::IsolateHandle {
            request_tx: Arc::new(std::sync::Mutex::new(Some(request_tx))),
            shutdown: CancellationToken::new(),
            id: uuid::Uuid::new_v4(),
            alive: alive.clone(),
        };

        let entry = FunctionEntry {
            name: "recover-fn".to_string(),
            bundle_package_bytes: Bytes::new(),
            eszip_bytes: Bytes::new(),
            bundle_format: BundleFormat::Eszip,
            package_v8_version: deno_core::v8::VERSION_STRING.to_string(),
            isolate_handle: Some(handle.clone()),
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

        reg.functions.insert("recover-fn".to_string(), entry);

        assert!(reg.get_handle("recover-fn").is_none());
        assert_eq!(
            reg.get_info("recover-fn").expect("missing info").status,
            FunctionStatus::Error
        );

        alive.store(true, Ordering::SeqCst);

        let recovered_handle = reg
            .get_handle("recover-fn")
            .expect("expected handle after recovery");
        assert_eq!(recovered_handle.id, handle.id);
        assert_eq!(
            reg.get_info("recover-fn").expect("missing info").status,
            FunctionStatus::Running
        );
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
            bundle_package_bytes: Bytes::new(),
            eszip_bytes: Bytes::new(),
            bundle_format: BundleFormat::Eszip,
            package_v8_version: deno_core::v8::VERSION_STRING.to_string(),
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
            bundle_package_bytes: Bytes::new(),
            eszip_bytes: Bytes::new(),
            bundle_format: BundleFormat::Eszip,
            package_v8_version: deno_core::v8::VERSION_STRING.to_string(),
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
    fn get_route_target_none_for_missing_function() {
        let reg = make_registry();
        assert!(reg.get_route_target("missing").is_none());
    }

    #[test]
    fn context_first_scheduler_creates_new_context_before_new_isolate() {
        let reg = make_registry();

        let (request_tx, _request_rx) = tokio::sync::mpsc::unbounded_channel();
        let primary = runtime_core::isolate::IsolateHandle {
            request_tx: Arc::new(std::sync::Mutex::new(Some(request_tx))),
            shutdown: CancellationToken::new(),
            id: uuid::Uuid::new_v4(),
            alive: Arc::new(AtomicBool::new(true)),
        };

        let entry = FunctionEntry {
            name: "ctx-fn".to_string(),
            bundle_package_bytes: Bytes::new(),
            eszip_bytes: Bytes::new(),
            bundle_format: BundleFormat::Eszip,
            package_v8_version: deno_core::v8::VERSION_STRING.to_string(),
            isolate_handle: Some(primary.clone()),
            extra_isolate_handles: Vec::new(),
            pool_limits: PoolLimits::default(),
            next_handle_index: 0,
            inspector_stop: None,
            status: FunctionStatus::Running,
            config: IsolateConfig {
                context_pool_enabled: true,
                max_contexts_per_isolate: 2,
                max_active_requests_per_context: 1,
                ..IsolateConfig::default()
            },
            manifest: None,
            metrics: Arc::new(FunctionMetrics::default()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_error: None,
        };

        reg.functions.insert("ctx-fn".to_string(), entry);

        let route_a = reg
            .get_route_target("ctx-fn")
            .expect("first route target should exist");
        let route_b = reg
            .get_route_target("ctx-fn")
            .expect("second route target should exist");

        assert_eq!(route_a.isolate_id, primary.id);
        assert_eq!(route_b.isolate_id, primary.id);
        assert_ne!(route_a.context_id, route_b.context_id);

        reg.release_route_target(&route_a);
        reg.release_route_target(&route_b);

        let route_c = reg
            .get_route_target("ctx-fn")
            .expect("route target after release should exist");
        assert_eq!(route_c.isolate_id, primary.id);
    }

    #[test]
    fn route_target_with_status_returns_unavailable_for_missing_function() {
        let reg = make_registry();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let result = rt.block_on(reg.get_route_target_with_status("missing"));
        assert!(matches!(result, Err(RouteTargetError::FunctionUnavailable)));
    }

    #[test]
    fn route_target_with_status_returns_capacity_exhausted_when_context_is_saturated() {
        let reg = make_registry();

        let (request_tx, _request_rx) = tokio::sync::mpsc::unbounded_channel();
        let primary = runtime_core::isolate::IsolateHandle {
            request_tx: Arc::new(std::sync::Mutex::new(Some(request_tx))),
            shutdown: CancellationToken::new(),
            id: uuid::Uuid::new_v4(),
            alive: Arc::new(AtomicBool::new(true)),
        };

        let entry = FunctionEntry {
            name: "ctx-saturated".to_string(),
            bundle_package_bytes: Bytes::new(),
            eszip_bytes: Bytes::new(),
            bundle_format: BundleFormat::Eszip,
            package_v8_version: deno_core::v8::VERSION_STRING.to_string(),
            isolate_handle: Some(primary.clone()),
            extra_isolate_handles: Vec::new(),
            pool_limits: PoolLimits::default(),
            next_handle_index: 0,
            inspector_stop: None,
            status: FunctionStatus::Running,
            config: IsolateConfig {
                context_pool_enabled: true,
                max_contexts_per_isolate: 1,
                max_active_requests_per_context: 1,
                ..IsolateConfig::default()
            },
            manifest: None,
            metrics: Arc::new(FunctionMetrics::default()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_error: None,
        };

        reg.functions.insert("ctx-saturated".to_string(), entry);

        let route = reg
            .get_route_target("ctx-saturated")
            .expect("first route target should exist");

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let saturated = rt.block_on(reg.get_route_target_with_status("ctx-saturated"));
        assert!(matches!(
            saturated,
            Err(RouteTargetError::CapacityExhausted)
        ));

        reg.release_route_target(&route);
    }

    #[test]
    fn routing_metrics_snapshot_reports_context_and_isolate_saturation() {
        let reg = make_registry();

        let (request_tx, _request_rx) = tokio::sync::mpsc::unbounded_channel();
        let primary = runtime_core::isolate::IsolateHandle {
            request_tx: Arc::new(std::sync::Mutex::new(Some(request_tx))),
            shutdown: CancellationToken::new(),
            id: uuid::Uuid::new_v4(),
            alive: Arc::new(AtomicBool::new(true)),
        };

        let entry = FunctionEntry {
            name: "ctx-metrics".to_string(),
            bundle_package_bytes: Bytes::new(),
            eszip_bytes: Bytes::new(),
            bundle_format: BundleFormat::Eszip,
            package_v8_version: deno_core::v8::VERSION_STRING.to_string(),
            isolate_handle: Some(primary.clone()),
            extra_isolate_handles: Vec::new(),
            pool_limits: PoolLimits::default(),
            next_handle_index: 0,
            inspector_stop: None,
            status: FunctionStatus::Running,
            config: IsolateConfig {
                context_pool_enabled: true,
                max_contexts_per_isolate: 1,
                max_active_requests_per_context: 1,
                ..IsolateConfig::default()
            },
            manifest: None,
            metrics: Arc::new(FunctionMetrics::default()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_error: None,
        };

        reg.functions.insert("ctx-metrics".to_string(), entry);

        let route = reg
            .get_route_target("ctx-metrics")
            .expect("first route target should exist");

        let snapshot = reg.routing_metrics_snapshot();
        assert_eq!(snapshot.total_contexts, 1);
        assert_eq!(snapshot.total_isolates, 1);
        assert_eq!(snapshot.total_active_requests, 1);
        assert_eq!(snapshot.saturated_contexts, 1);
        assert_eq!(snapshot.saturated_isolates, 1);

        reg.release_route_target(&route);

        let after_release = reg.routing_metrics_snapshot();
        assert_eq!(after_release.saturated_contexts, 0);
        assert_eq!(after_release.saturated_isolates, 0);
    }

    #[test]
    fn get_route_target_skips_dead_isolate_entries() {
        let reg = make_registry();

        let (dead_tx, _dead_rx) = tokio::sync::mpsc::unbounded_channel();
        let dead = runtime_core::isolate::IsolateHandle {
            request_tx: Arc::new(std::sync::Mutex::new(Some(dead_tx))),
            shutdown: CancellationToken::new(),
            id: uuid::Uuid::new_v4(),
            alive: Arc::new(AtomicBool::new(false)),
        };

        let (alive_tx, _alive_rx) = tokio::sync::mpsc::unbounded_channel();
        let alive = runtime_core::isolate::IsolateHandle {
            request_tx: Arc::new(std::sync::Mutex::new(Some(alive_tx))),
            shutdown: CancellationToken::new(),
            id: uuid::Uuid::new_v4(),
            alive: Arc::new(AtomicBool::new(true)),
        };

        let entry = FunctionEntry {
            name: "ctx-dead".to_string(),
            bundle_package_bytes: Bytes::new(),
            eszip_bytes: Bytes::new(),
            bundle_format: BundleFormat::Eszip,
            package_v8_version: deno_core::v8::VERSION_STRING.to_string(),
            isolate_handle: Some(dead),
            extra_isolate_handles: vec![alive.clone()],
            pool_limits: PoolLimits::default(),
            next_handle_index: 0,
            inspector_stop: None,
            status: FunctionStatus::Running,
            config: IsolateConfig {
                context_pool_enabled: true,
                max_contexts_per_isolate: 2,
                max_active_requests_per_context: 1,
                ..IsolateConfig::default()
            },
            manifest: None,
            metrics: Arc::new(FunctionMetrics::default()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_error: None,
        };

        reg.functions.insert("ctx-dead".to_string(), entry);

        let route = reg
            .get_route_target("ctx-dead")
            .expect("route target should exist on alive isolate");
        assert_eq!(route.isolate_id, alive.id);
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
            bundle_package_bytes: Bytes::new(),
            eszip_bytes: Bytes::new(),
            bundle_format: BundleFormat::Eszip,
            package_v8_version: deno_core::v8::VERSION_STRING.to_string(),
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

    #[test]
    fn evict_lru_replica_prefers_oldest_extra_handle() {
        let reg = make_registry_with_pool(true);

        let (primary_a_tx, _primary_a_rx) = tokio::sync::mpsc::unbounded_channel();
        let primary_a = runtime_core::isolate::IsolateHandle {
            request_tx: Arc::new(std::sync::Mutex::new(Some(primary_a_tx))),
            shutdown: CancellationToken::new(),
            id: uuid::Uuid::new_v4(),
            alive: Arc::new(AtomicBool::new(true)),
        };
        let (extra_a_tx, _extra_a_rx) = tokio::sync::mpsc::unbounded_channel();
        let extra_a = runtime_core::isolate::IsolateHandle {
            request_tx: Arc::new(std::sync::Mutex::new(Some(extra_a_tx))),
            shutdown: CancellationToken::new(),
            id: uuid::Uuid::new_v4(),
            alive: Arc::new(AtomicBool::new(true)),
        };

        let entry_a = FunctionEntry {
            name: "fn-a".to_string(),
            bundle_package_bytes: Bytes::new(),
            eszip_bytes: Bytes::new(),
            bundle_format: BundleFormat::Eszip,
            package_v8_version: deno_core::v8::VERSION_STRING.to_string(),
            isolate_handle: Some(primary_a),
            extra_isolate_handles: vec![extra_a.clone()],
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

        let (primary_b_tx, _primary_b_rx) = tokio::sync::mpsc::unbounded_channel();
        let primary_b = runtime_core::isolate::IsolateHandle {
            request_tx: Arc::new(std::sync::Mutex::new(Some(primary_b_tx))),
            shutdown: CancellationToken::new(),
            id: uuid::Uuid::new_v4(),
            alive: Arc::new(AtomicBool::new(true)),
        };
        let (extra_b_tx, _extra_b_rx) = tokio::sync::mpsc::unbounded_channel();
        let extra_b = runtime_core::isolate::IsolateHandle {
            request_tx: Arc::new(std::sync::Mutex::new(Some(extra_b_tx))),
            shutdown: CancellationToken::new(),
            id: uuid::Uuid::new_v4(),
            alive: Arc::new(AtomicBool::new(true)),
        };

        let entry_b = FunctionEntry {
            name: "fn-b".to_string(),
            bundle_package_bytes: Bytes::new(),
            eszip_bytes: Bytes::new(),
            bundle_format: BundleFormat::Eszip,
            package_v8_version: deno_core::v8::VERSION_STRING.to_string(),
            isolate_handle: Some(primary_b),
            extra_isolate_handles: vec![extra_b.clone()],
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

        reg.functions.insert("fn-a".to_string(), entry_a);
        reg.functions.insert("fn-b".to_string(), entry_b);

        // Lower tick means older usage and should be evicted first.
        reg.handle_last_used.insert(extra_a.id, 10);
        reg.handle_last_used.insert(extra_b.id, 20);

        assert!(reg.evict_lru_replica_for_capacity("fn-caller"));

        {
            let fn_a = reg.functions.get("fn-a").expect("missing fn-a");
            assert!(fn_a.extra_isolate_handles.is_empty());
        }
        {
            let fn_b = reg.functions.get("fn-b").expect("missing fn-b");
            assert_eq!(fn_b.extra_isolate_handles.len(), 1);
        }
    }

    #[test]
    fn evict_lru_replica_respects_min_pool_size() {
        let reg = make_registry_with_pool(true);

        let (primary_tx, _primary_rx) = tokio::sync::mpsc::unbounded_channel();
        let primary = runtime_core::isolate::IsolateHandle {
            request_tx: Arc::new(std::sync::Mutex::new(Some(primary_tx))),
            shutdown: CancellationToken::new(),
            id: uuid::Uuid::new_v4(),
            alive: Arc::new(AtomicBool::new(true)),
        };
        let (extra_tx, _extra_rx) = tokio::sync::mpsc::unbounded_channel();
        let extra = runtime_core::isolate::IsolateHandle {
            request_tx: Arc::new(std::sync::Mutex::new(Some(extra_tx))),
            shutdown: CancellationToken::new(),
            id: uuid::Uuid::new_v4(),
            alive: Arc::new(AtomicBool::new(true)),
        };

        let entry = FunctionEntry {
            name: "fn-min".to_string(),
            bundle_package_bytes: Bytes::new(),
            eszip_bytes: Bytes::new(),
            bundle_format: BundleFormat::Eszip,
            package_v8_version: deno_core::v8::VERSION_STRING.to_string(),
            isolate_handle: Some(primary),
            extra_isolate_handles: vec![extra],
            pool_limits: PoolLimits { min: 2, max: 2 },
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
        reg.functions.insert("fn-min".to_string(), entry);

        assert!(!reg.evict_lru_replica_for_capacity("fn-min"));
        let current = reg.functions.get("fn-min").expect("missing fn-min");
        assert_eq!(current.extra_isolate_handles.len(), 1);
    }
}

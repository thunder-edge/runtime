use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use serde::Serialize;
use tokio::sync::Notify;

#[derive(Debug, Clone)]
pub struct ConnectionManagerConfig {
    pub fd_reserved_absolute: usize,
    pub fd_reserved_ratio: f64,
    pub queue_max_waiters: usize,
    pub default_wait_timeout: Duration,
    pub per_tenant_max_active: usize,
    pub lease_hard_ttl: Duration,
    pub min_token_refill_per_sec: f64,
}

impl Default for ConnectionManagerConfig {
    fn default() -> Self {
        Self {
            fd_reserved_absolute: 512,
            fd_reserved_ratio: 0.20,
            queue_max_waiters: 50_000,
            default_wait_timeout: Duration::from_millis(75),
            per_tenant_max_active: 2_048,
            lease_hard_ttl: Duration::from_secs(300),
            min_token_refill_per_sec: 256.0,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionManagerSnapshot {
    pub soft_limit: u64,
    pub open_fd_count: u64,
    pub reserved_fd: u64,
    pub outbound_fd_budget: u64,
    pub adaptive_active_limit: u64,
    pub active_leases: u64,
    pub queued_waiters: u64,
    pub total_acquired: u64,
    pub total_released: u64,
    pub total_rejected: u64,
    pub total_timeouts: u64,
    pub total_reaped: u64,
    pub known_tenants: u64,
    pub top_tenants_by_active: Vec<TenantActiveSnapshot>,
    pub token_bucket: TokenBucketSnapshot,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TenantActiveSnapshot {
    pub tenant: String,
    pub active: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenBucketSnapshot {
    pub tokens: f64,
    pub capacity: f64,
    pub refill_per_sec: f64,
}

#[derive(Debug)]
pub enum AcquireError {
    Backpressure,
    Timeout,
    Internal(String),
}

#[derive(Debug)]
struct LeaseMeta {
    tenant: String,
    execution_id: String,
    created_at: Instant,
}

#[derive(Debug)]
struct TokenBucketState {
    tokens: f64,
    capacity: f64,
    refill_per_sec: f64,
    last_refill: Instant,
}

impl TokenBucketState {
    fn new() -> Self {
        Self {
            tokens: 1.0,
            capacity: 1.0,
            refill_per_sec: 1.0,
            last_refill: Instant::now(),
        }
    }

    fn refill(&mut self, now: Instant) {
        let elapsed = now.saturating_duration_since(self.last_refill).as_secs_f64();
        if elapsed > 0.0 {
            self.tokens = (self.tokens + elapsed * self.refill_per_sec).min(self.capacity);
            self.last_refill = now;
        }
    }

    fn set_profile(&mut self, capacity: f64, refill_per_sec: f64) {
        self.capacity = capacity.max(1.0);
        self.refill_per_sec = refill_per_sec.max(1.0);
        self.tokens = self.tokens.min(self.capacity);
    }

    fn try_take(&mut self, now: Instant, amount: f64) -> bool {
        self.refill(now);
        if self.tokens >= amount {
            self.tokens -= amount;
            true
        } else {
            false
        }
    }
}

pub struct ConnectionManager {
    config: ConnectionManagerConfig,
    notify: Notify,
    next_lease_id: AtomicU64,
    active_leases: AtomicUsize,
    queued_waiters: AtomicUsize,
    total_acquired: AtomicU64,
    total_released: AtomicU64,
    total_rejected: AtomicU64,
    total_timeouts: AtomicU64,
    total_reaped: AtomicU64,
    leases: Mutex<HashMap<u64, LeaseMeta>>,
    tenant_active: DashMap<String, usize>,
    token_bucket: Mutex<TokenBucketState>,
}

impl ConnectionManager {
    pub fn new(config: ConnectionManagerConfig) -> Arc<Self> {
        let manager = Arc::new(Self {
            config,
            notify: Notify::new(),
            next_lease_id: AtomicU64::new(1),
            active_leases: AtomicUsize::new(0),
            queued_waiters: AtomicUsize::new(0),
            total_acquired: AtomicU64::new(0),
            total_released: AtomicU64::new(0),
            total_rejected: AtomicU64::new(0),
            total_timeouts: AtomicU64::new(0),
            total_reaped: AtomicU64::new(0),
            leases: Mutex::new(HashMap::new()),
            tenant_active: DashMap::new(),
            token_bucket: Mutex::new(TokenBucketState::new()),
        });

        Self::start_reaper(&manager);
        manager
    }

    fn start_reaper(manager: &Arc<Self>) {
        let weak = Arc::downgrade(manager);
        let lease_ttl = manager.config.lease_hard_ttl;
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(30));
                let Some(manager) = weak.upgrade() else {
                    break;
                };
                manager.reap_stale_leases(lease_ttl);
            }
        });
    }

    pub async fn acquire_lease(
        &self,
        tenant: String,
        execution_id: String,
        timeout_override: Option<Duration>,
    ) -> Result<u64, AcquireError> {
        let timeout = timeout_override.unwrap_or(self.config.default_wait_timeout);

        if self.queued_waiters.load(Ordering::Relaxed) >= self.config.queue_max_waiters {
            self.total_rejected.fetch_add(1, Ordering::Relaxed);
            return Err(AcquireError::Backpressure);
        }

        self.queued_waiters.fetch_add(1, Ordering::Relaxed);
        let deadline = Instant::now() + timeout;

        let result = loop {
            match self.try_acquire_now(&tenant, &execution_id) {
                Ok(lease_id) => break Ok(lease_id),
                Err(AcquireError::Backpressure) => {
                    let now = Instant::now();
                    if now >= deadline {
                        self.total_timeouts.fetch_add(1, Ordering::Relaxed);
                        break Err(AcquireError::Timeout);
                    }
                    let remaining = deadline.saturating_duration_since(now);
                    let wait_for = remaining.min(Duration::from_millis(10));
                    if tokio::time::timeout(wait_for, self.notify.notified()).await.is_err() {
                        continue;
                    }
                }
                Err(other) => break Err(other),
            }
        };

        self.queued_waiters.fetch_sub(1, Ordering::Relaxed);
        result
    }

    fn try_acquire_now(&self, tenant: &str, execution_id: &str) -> Result<u64, AcquireError> {
        let (soft_limit, open_fds) = fd_limits();
        let reserved_fd = self.reserved_fd(soft_limit);

        if soft_limit > 0 && open_fds.saturating_add(reserved_fd).saturating_add(1) > soft_limit {
            self.total_rejected.fetch_add(1, Ordering::Relaxed);
            return Err(AcquireError::Backpressure);
        }

        let adaptive_limit = self.adaptive_active_limit(soft_limit, open_fds, reserved_fd);
        let current_active = self.active_leases.load(Ordering::Relaxed);
        if adaptive_limit > 0 && current_active >= adaptive_limit {
            self.total_rejected.fetch_add(1, Ordering::Relaxed);
            return Err(AcquireError::Backpressure);
        }

        let tenant_active = self.tenant_active.get(tenant).map(|v| *v).unwrap_or(0);
        if tenant_active >= self.config.per_tenant_max_active {
            self.total_rejected.fetch_add(1, Ordering::Relaxed);
            return Err(AcquireError::Backpressure);
        }

        {
            let mut bucket = self
                .token_bucket
                .lock()
                .map_err(|e| AcquireError::Internal(format!("token bucket lock poisoned: {e}")))?;
            let capacity = adaptive_limit.max(1) as f64;
            let refill_per_sec = capacity.max(self.config.min_token_refill_per_sec);
            bucket.set_profile(capacity, refill_per_sec);
            if !bucket.try_take(Instant::now(), 1.0) {
                self.total_rejected.fetch_add(1, Ordering::Relaxed);
                return Err(AcquireError::Backpressure);
            }
        }

        let lease_id = self.next_lease_id.fetch_add(1, Ordering::Relaxed);
        {
            let mut leases = self
                .leases
                .lock()
                .map_err(|e| AcquireError::Internal(format!("lease map lock poisoned: {e}")))?;
            leases.insert(
                lease_id,
                LeaseMeta {
                    tenant: tenant.to_string(),
                    execution_id: execution_id.to_string(),
                    created_at: Instant::now(),
                },
            );
        }

        self.tenant_active
            .entry(tenant.to_string())
            .and_modify(|v| *v = v.saturating_add(1))
            .or_insert(1);
        self.active_leases.fetch_add(1, Ordering::Relaxed);
        self.total_acquired.fetch_add(1, Ordering::Relaxed);

        Ok(lease_id)
    }

    pub fn release_lease(&self, lease_id: u64) -> bool {
        let meta = {
            let mut leases = match self.leases.lock() {
                Ok(leases) => leases,
                Err(_) => return false,
            };
            leases.remove(&lease_id)
        };

        let Some(meta) = meta else {
            return false;
        };

        self.dec_tenant(&meta.tenant);
        self.active_leases.fetch_sub(1, Ordering::Relaxed);
        self.total_released.fetch_add(1, Ordering::Relaxed);
        self.notify.notify_one();
        true
    }

    pub fn release_execution_leases(&self, execution_id: &str) -> usize {
        let lease_ids: Vec<u64> = {
            let leases = match self.leases.lock() {
                Ok(leases) => leases,
                Err(_) => return 0,
            };
            leases
                .iter()
                .filter_map(|(id, meta)| {
                    if meta.execution_id == execution_id {
                        Some(*id)
                    } else {
                        None
                    }
                })
                .collect()
        };

        let mut released = 0usize;
        for lease_id in lease_ids {
            if self.release_lease(lease_id) {
                released = released.saturating_add(1);
            }
        }
        released
    }

    fn reap_stale_leases(&self, ttl: Duration) {
        let now = Instant::now();
        let stale_lease_ids: Vec<u64> = {
            let leases = match self.leases.lock() {
                Ok(leases) => leases,
                Err(_) => return,
            };
            leases
                .iter()
                .filter_map(|(id, meta)| {
                    if now.saturating_duration_since(meta.created_at) > ttl {
                        Some(*id)
                    } else {
                        None
                    }
                })
                .collect()
        };

        let mut reaped = 0u64;
        for lease_id in stale_lease_ids {
            if self.release_lease(lease_id) {
                reaped = reaped.saturating_add(1);
            }
        }

        if reaped > 0 {
            self.total_reaped.fetch_add(reaped, Ordering::Relaxed);
        }
    }

    pub fn snapshot(&self) -> ConnectionManagerSnapshot {
        let (soft_limit, open_fd_count) = fd_limits();
        let reserved_fd = self.reserved_fd(soft_limit);
        let outbound_fd_budget = soft_limit.saturating_sub(reserved_fd);
        let adaptive_active_limit = self.adaptive_active_limit(soft_limit, open_fd_count, reserved_fd);

        let mut top_tenants_by_active: Vec<TenantActiveSnapshot> = self
            .tenant_active
            .iter()
            .map(|entry| TenantActiveSnapshot {
                tenant: entry.key().clone(),
                active: *entry.value() as u64,
            })
            .collect();
        top_tenants_by_active.sort_by(|a, b| b.active.cmp(&a.active));
        top_tenants_by_active.truncate(10);

        let token_bucket = self
            .token_bucket
            .lock()
            .map(|bucket| TokenBucketSnapshot {
                tokens: bucket.tokens,
                capacity: bucket.capacity,
                refill_per_sec: bucket.refill_per_sec,
            })
            .unwrap_or(TokenBucketSnapshot {
                tokens: 0.0,
                capacity: 0.0,
                refill_per_sec: 0.0,
            });

        ConnectionManagerSnapshot {
            soft_limit: soft_limit as u64,
            open_fd_count: open_fd_count as u64,
            reserved_fd: reserved_fd as u64,
            outbound_fd_budget: outbound_fd_budget as u64,
            adaptive_active_limit: adaptive_active_limit as u64,
            active_leases: self.active_leases.load(Ordering::Relaxed) as u64,
            queued_waiters: self.queued_waiters.load(Ordering::Relaxed) as u64,
            total_acquired: self.total_acquired.load(Ordering::Relaxed),
            total_released: self.total_released.load(Ordering::Relaxed),
            total_rejected: self.total_rejected.load(Ordering::Relaxed),
            total_timeouts: self.total_timeouts.load(Ordering::Relaxed),
            total_reaped: self.total_reaped.load(Ordering::Relaxed),
            known_tenants: self.tenant_active.len() as u64,
            top_tenants_by_active,
            token_bucket,
        }
    }

    fn dec_tenant(&self, tenant: &str) {
        if let Some(mut entry) = self.tenant_active.get_mut(tenant) {
            if *entry > 1 {
                *entry -= 1;
            } else {
                let key = tenant.to_string();
                drop(entry);
                self.tenant_active.remove(&key);
            }
        }
    }

    fn reserved_fd(&self, soft_limit: usize) -> usize {
        if soft_limit == 0 {
            return self.config.fd_reserved_absolute;
        }
        let ratio_reserved = ((soft_limit as f64) * self.config.fd_reserved_ratio).round() as usize;
        self.config.fd_reserved_absolute.max(ratio_reserved)
    }

    fn adaptive_active_limit(&self, soft_limit: usize, open_fd_count: usize, reserved_fd: usize) -> usize {
        if soft_limit == 0 {
            return 1_024;
        }

        let outbound_budget = soft_limit.saturating_sub(reserved_fd);
        if outbound_budget == 0 {
            return 1;
        }

        let fd_pressure = if soft_limit > 0 {
            open_fd_count as f64 / soft_limit as f64
        } else {
            0.0
        };

        let pressure_factor = if fd_pressure >= 0.90 {
            0.25
        } else if fd_pressure >= 0.80 {
            0.50
        } else if fd_pressure >= 0.70 {
            0.75
        } else {
            1.0
        };

        ((outbound_budget as f64 * pressure_factor).round() as usize).max(1)
    }
}

fn fd_limits() -> (usize, usize) {
    let soft_limit = fd_soft_limit().unwrap_or(0);
    let open = open_fd_count().unwrap_or(0);
    (soft_limit, open)
}

fn fd_soft_limit() -> Option<usize> {
    let mut lim = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // Safety: getrlimit writes to a valid pointer to `rlimit`.
    let rc = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim) };
    if rc == 0 {
        Some(lim.rlim_cur as usize)
    } else {
        None
    }
}

fn open_fd_count() -> Option<usize> {
    std::fs::read_dir("/proc/self/fd")
        .ok()
        .map(|entries| entries.count())
        .or_else(|| std::fs::read_dir("/dev/fd").ok().map(|entries| entries.count()))
}

static CONNECTION_MANAGER: OnceLock<Arc<ConnectionManager>> = OnceLock::new();

pub fn global_connection_manager() -> &'static Arc<ConnectionManager> {
    CONNECTION_MANAGER.get_or_init(|| ConnectionManager::new(ConnectionManagerConfig::default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn acquire_and_release_updates_snapshot() {
        let manager = ConnectionManager::new(ConnectionManagerConfig {
            queue_max_waiters: 8,
            per_tenant_max_active: 8,
            ..ConnectionManagerConfig::default()
        });

        let lease_id = manager
            .acquire_lease("tenant-a".to_string(), "exec-1".to_string(), None)
            .await
            .expect("lease should be acquired");

        let snapshot = manager.snapshot();
        assert_eq!(snapshot.active_leases, 1);
        assert_eq!(snapshot.total_acquired, 1);

        assert!(manager.release_lease(lease_id));
        let snapshot = manager.snapshot();
        assert_eq!(snapshot.active_leases, 0);
        assert_eq!(snapshot.total_released, 1);
    }

    #[tokio::test]
    async fn per_tenant_limit_is_enforced() {
        let manager = ConnectionManager::new(ConnectionManagerConfig {
            queue_max_waiters: 1,
            per_tenant_max_active: 1,
            default_wait_timeout: Duration::from_millis(1),
            ..ConnectionManagerConfig::default()
        });

        let lease_id = manager
            .acquire_lease("tenant-limited".to_string(), "exec-a".to_string(), None)
            .await
            .expect("first lease should be acquired");

        let second = manager
            .acquire_lease(
                "tenant-limited".to_string(),
                "exec-b".to_string(),
                Some(Duration::from_millis(1)),
            )
            .await;
        assert!(matches!(second, Err(AcquireError::Timeout)));

        assert!(manager.release_lease(lease_id));
    }
}

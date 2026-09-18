//! Shared lifecycle state for the dual-listener runtime.
//!
//! The state is deliberately small and independent from the Supervisor
//! process.  It gives the admin and ingress routers one linearization point
//! for readiness and ingress admission while keeping the admin listener alive
//! during a drain.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::Notify;

const RUNTIME_REVISION_ENV: &str = "EDGE_RUNTIME_REVISION";

#[derive(Debug)]
struct RuntimeStateInner {
    started_at: Instant,
    runtime_ready: AtomicBool,
    initial_reconcile_complete: AtomicBool,
    draining: AtomicBool,
    active_ingress_requests: AtomicU64,
    active_requests_changed: Notify,
    admission_lock: Mutex<()>,
    runtime_revision: String,
    pid: u32,
}

/// Lifecycle and admission state shared by the admin and ingress listeners.
#[derive(Clone, Debug)]
pub struct RuntimeState {
    inner: Arc<RuntimeStateInner>,
}

/// A request admission held until its response body is dropped or fully
/// consumed.
///
/// Keeping this guard in the response body, rather than releasing it when the
/// router future returns, makes graceful drain cover streaming responses and
/// clients that disconnect while a response is in flight.
#[derive(Debug)]
pub struct IngressAdmission {
    state: RuntimeState,
}

/// Point-in-time lifecycle information used by health and state endpoints.
#[derive(Debug, Clone)]
pub struct RuntimeStateSnapshot {
    pub runtime_revision: String,
    pub pid: u32,
    pub uptime: Duration,
    pub runtime_ready: bool,
    pub initial_reconcile_complete: bool,
    pub ready: bool,
    pub draining: bool,
    pub active_requests: u64,
}

impl RuntimeState {
    /// Create a state that starts unready until the runtime and its initial
    /// reconcile have both completed.
    pub fn new() -> Self {
        Self::with_revision(
            std::env::var(RUNTIME_REVISION_ENV)
                .ok()
                .filter(|revision| !revision.trim().is_empty())
                .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string()),
        )
    }

    /// Create lifecycle state with an explicit revision.
    ///
    /// This is useful for callers that already have a revision from an
    /// orchestrator, and keeps tests independent of process environment.
    pub fn with_revision(runtime_revision: impl Into<String>) -> Self {
        Self {
            inner: Arc::new(RuntimeStateInner {
                started_at: Instant::now(),
                runtime_ready: AtomicBool::new(false),
                initial_reconcile_complete: AtomicBool::new(false),
                draining: AtomicBool::new(false),
                active_ingress_requests: AtomicU64::new(0),
                active_requests_changed: Notify::new(),
                admission_lock: Mutex::new(()),
                runtime_revision: runtime_revision.into(),
                pid: std::process::id(),
            }),
        }
    }

    /// Build state for the legacy standalone server path.
    ///
    /// Standalone mode has no external Supervisor/reconcile loop.  Its
    /// successful server bootstrap is therefore the initial no-op reconcile,
    /// preserving the existing always-live standalone behavior.
    pub fn standalone() -> Self {
        let state = Self::new();
        state.mark_runtime_ready();
        state.mark_initial_reconcile_complete();
        state
    }

    /// Mark the runtime listener/bootstrap portion as healthy.
    pub fn mark_runtime_ready(&self) {
        self.inner.runtime_ready.store(true, Ordering::Release);
    }

    /// Mark the first desired-vs-observed reconciliation as successful.
    pub fn mark_initial_reconcile_complete(&self) {
        self.inner
            .initial_reconcile_complete
            .store(true, Ordering::Release);
    }

    /// Reset runtime readiness after a runtime health failure.
    pub fn mark_runtime_not_ready(&self) {
        self.inner.runtime_ready.store(false, Ordering::Release);
    }

    /// Return whether the process is ready to receive traffic.
    pub fn is_ready(&self) -> bool {
        self.inner.runtime_ready.load(Ordering::Acquire)
            && self
                .inner
                .initial_reconcile_complete
                .load(Ordering::Acquire)
            && !self.is_draining()
    }

    pub fn runtime_ready(&self) -> bool {
        self.inner.runtime_ready.load(Ordering::Acquire)
    }

    pub fn initial_reconcile_complete(&self) -> bool {
        self.inner
            .initial_reconcile_complete
            .load(Ordering::Acquire)
    }

    pub fn is_draining(&self) -> bool {
        self.inner.draining.load(Ordering::Acquire)
    }

    pub fn active_requests(&self) -> u64 {
        self.inner.active_ingress_requests.load(Ordering::Acquire)
    }

    /// Begin graceful drain.  The transition is idempotent.
    ///
    /// The atomic admission check in [`try_admit_ingress`] is paired with this
    /// flag, so no request can be admitted after the drain transition has
    /// linearized.
    pub fn begin_drain(&self) -> bool {
        let _admission_lock = self
            .inner
            .admission_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let was_draining = self.inner.draining.swap(true, Ordering::AcqRel);
        self.inner.active_requests_changed.notify_waiters();
        !was_draining
    }

    /// Admit one ingress request unless the runtime is draining.
    pub fn try_admit_ingress(&self) -> Option<IngressAdmission> {
        let _admission_lock = self
            .inner
            .admission_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.is_draining() {
            return None;
        }

        self.inner
            .active_ingress_requests
            .fetch_add(1, Ordering::AcqRel);

        Some(IngressAdmission {
            state: self.clone(),
        })
    }

    fn release_ingress_request(&self) {
        let previous = self
            .inner
            .active_ingress_requests
            .fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0, "ingress admission counter underflow");
        self.inner.active_requests_changed.notify_waiters();
    }

    /// Wait until all admitted ingress requests have completed.
    pub async fn wait_for_no_active_requests(&self) {
        loop {
            if self.active_requests() == 0 {
                return;
            }

            let notified = self.inner.active_requests_changed.notified();
            if self.active_requests() == 0 {
                return;
            }
            notified.await;
        }
    }

    pub fn snapshot(&self) -> RuntimeStateSnapshot {
        let runtime_ready = self.runtime_ready();
        let initial_reconcile_complete = self.initial_reconcile_complete();
        let draining = self.is_draining();

        RuntimeStateSnapshot {
            runtime_revision: self.inner.runtime_revision.clone(),
            pid: self.inner.pid,
            uptime: self.inner.started_at.elapsed(),
            runtime_ready,
            initial_reconcile_complete,
            ready: runtime_ready && initial_reconcile_complete && !draining,
            draining,
            active_requests: self.active_requests(),
        }
    }
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for IngressAdmission {
    fn drop(&mut self) {
        self.state.release_ingress_request();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn readiness_requires_runtime_and_initial_reconcile() {
        let state = RuntimeState::with_revision("rev-test");
        assert!(!state.is_ready());

        state.mark_runtime_ready();
        assert!(!state.is_ready());

        state.mark_initial_reconcile_complete();
        assert!(state.is_ready());
        assert_eq!(state.snapshot().runtime_revision, "rev-test");
    }

    #[tokio::test]
    async fn drain_is_idempotent_and_rejects_new_admissions() {
        let state = RuntimeState::standalone();
        let admission = state
            .try_admit_ingress()
            .expect("request should be admitted");
        assert_eq!(state.active_requests(), 1);

        assert!(state.begin_drain());
        assert!(!state.begin_drain());
        assert!(!state.is_ready());
        assert!(state.try_admit_ingress().is_none());

        drop(admission);
        tokio::time::timeout(Duration::from_secs(1), state.wait_for_no_active_requests())
            .await
            .expect("active admission did not finish");
        assert_eq!(state.active_requests(), 0);
    }
}

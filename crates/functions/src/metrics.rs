use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;

/// Global runtime metrics aggregated from all functions.
#[derive(Debug, Default)]
pub struct GlobalMetrics {
    pub total_requests: AtomicU64,
    pub total_errors: AtomicU64,
}

impl GlobalMetrics {
    pub fn snapshot(&self) -> GlobalMetricsSnapshot {
        GlobalMetricsSnapshot {
            total_requests: self.total_requests.load(Ordering::Relaxed),
            total_errors: self.total_errors.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GlobalMetricsSnapshot {
    pub total_requests: u64,
    pub total_errors: u64,
}

#[cfg(test)]
#[path = "metrics_test.rs"]
mod tests;

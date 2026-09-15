use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Tracks CPU time consumed by an isolate.
///
/// Preferred source is per-thread CPU time (`CLOCK_THREAD_CPUTIME_ID`) when
/// available on the current platform. This excludes idle/sleep time and better
/// represents actual CPU usage of the isolate thread.
///
/// Fallback is wall-clock (`Instant`) for platforms/environments where thread
/// CPU clock is unavailable.
///
/// CPU time vs wall-clock:
/// - CPU time: counts only time while this thread is actively running.
/// - Wall-clock: counts elapsed real time, including sleep/wait/blocked states.
pub struct CpuTimer {
    started_wall: Option<Instant>,
    started_cpu_ns: Option<u64>,
    accumulated_ms: u64,
    limit_ms: u64,
    exceeded: Arc<AtomicBool>,
}

#[cfg(unix)]
fn thread_cpu_time_nanos() -> Option<u64> {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };

    // Safety: `ts` is a valid mutable pointer for `clock_gettime`.
    let rc = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) };
    if rc != 0 {
        return None;
    }

    let secs = u64::try_from(ts.tv_sec).ok()?;
    let nanos = u64::try_from(ts.tv_nsec).ok()?;
    Some(secs.saturating_mul(1_000_000_000).saturating_add(nanos))
}

#[cfg(not(unix))]
fn thread_cpu_time_nanos() -> Option<u64> {
    None
}

fn nanos_to_millis_saturating(ns: u64) -> u64 {
    ns / 1_000_000
}

impl CpuTimer {
    pub fn new(limit_ms: u64) -> Self {
        Self {
            started_wall: None,
            started_cpu_ns: None,
            accumulated_ms: 0,
            limit_ms,
            exceeded: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Start timing a request.
    pub fn start(&mut self) {
        self.started_wall = Some(Instant::now());
        self.started_cpu_ns = thread_cpu_time_nanos();
    }

    /// Stop timing and accumulate elapsed time. Returns elapsed ms for this request.
    pub fn stop(&mut self) -> u64 {
        let started_wall = self.started_wall.take();
        let started_cpu_ns = self.started_cpu_ns.take();

        let elapsed =
            if let (Some(cpu_start), Some(cpu_now)) = (started_cpu_ns, thread_cpu_time_nanos()) {
                nanos_to_millis_saturating(cpu_now.saturating_sub(cpu_start))
            } else if let Some(started) = started_wall {
                started.elapsed().as_millis() as u64
            } else {
                0
            };

        self.accumulated_ms = self.accumulated_ms.saturating_add(elapsed);
        if self.limit_ms > 0 && self.accumulated_ms >= self.limit_ms {
            self.exceeded.store(true, Ordering::Relaxed);
        }
        elapsed
    }

    /// Check if the CPU time limit has been exceeded.
    pub fn is_exceeded(&self) -> bool {
        self.exceeded.load(Ordering::Relaxed)
    }

    /// Get the shared exceeded flag (for passing to V8 interrupt).
    pub fn exceeded_flag(&self) -> Arc<AtomicBool> {
        self.exceeded.clone()
    }

    pub fn accumulated_ms(&self) -> u64 {
        self.accumulated_ms
    }

    pub fn limit_ms(&self) -> u64 {
        self.limit_ms
    }

    /// Reset the timer for a new request.
    /// Clears accumulated time and exceeded flag.
    pub fn reset(&mut self) {
        self.started_wall = None;
        self.started_cpu_ns = None;
        self.accumulated_ms = 0;
        self.exceeded.store(false, Ordering::Relaxed);
    }

    /// Indicates whether thread CPU clock is available on this platform/runtime.
    pub fn supports_thread_cpu_time() -> bool {
        thread_cpu_time_nanos().is_some()
    }
}

/// Wall-clock timeout guard for a single request.
pub struct WallClockGuard {
    deadline: Instant,
}

impl WallClockGuard {
    pub fn new(timeout_ms: u64) -> Self {
        Self {
            deadline: Instant::now() + Duration::from_millis(timeout_ms),
        }
    }

    pub fn remaining(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }

    pub fn is_expired(&self) -> bool {
        Instant::now() >= self.deadline
    }

    pub fn as_sleep(&self) -> tokio::time::Sleep {
        tokio::time::sleep(self.remaining())
    }
}

#[cfg(test)]
#[path = "cpu_timer_test.rs"]
mod tests;

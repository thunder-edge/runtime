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
mod tests {
    use super::*;
    use std::thread;

    fn busy_work(duration: Duration) {
        let start = Instant::now();
        let mut x: u64 = 0;
        while start.elapsed() < duration {
            // Keep CPU busy with deterministic integer work.
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            std::hint::black_box(x);
        }
    }

    fn thread_cpu_elapsed_ms_for(duration: Duration, busy: bool) -> Option<u64> {
        let start = thread_cpu_time_nanos()?;
        if busy {
            busy_work(duration);
        } else {
            thread::sleep(duration);
        }
        let end = thread_cpu_time_nanos()?;
        Some(nanos_to_millis_saturating(end.saturating_sub(start)))
    }

    #[test]
    fn cpu_timer_new_not_exceeded() {
        let timer = CpuTimer::new(5000);
        assert!(!timer.is_exceeded());
        assert_eq!(timer.accumulated_ms(), 0);
        assert_eq!(timer.limit_ms(), 5000);
    }

    #[test]
    fn cpu_timer_start_stop_accumulates() {
        let mut timer = CpuTimer::new(10_000);
        timer.start();
        busy_work(Duration::from_millis(50));
        let elapsed = timer.stop();
        assert!(elapsed >= 30, "elapsed should be >= 30ms, got {elapsed}");
        assert!(timer.accumulated_ms() >= 30);
        assert!(!timer.is_exceeded());
    }

    #[test]
    fn cpu_timer_stop_without_start_returns_zero() {
        let mut timer = CpuTimer::new(1000);
        let elapsed = timer.stop();
        assert_eq!(elapsed, 0);
    }

    #[test]
    fn cpu_timer_exceeds_limit() {
        let mut timer = CpuTimer::new(10);
        timer.start();
        busy_work(Duration::from_millis(30));
        timer.stop();
        assert!(timer.is_exceeded());
    }

    #[test]
    fn cpu_timer_exceeded_flag_shared() {
        let mut timer = CpuTimer::new(10);
        let flag = timer.exceeded_flag();
        assert!(!flag.load(Ordering::Relaxed));
        timer.start();
        busy_work(Duration::from_millis(30));
        timer.stop();
        assert!(flag.load(Ordering::Relaxed));
    }

    #[test]
    fn cpu_timer_multiple_start_stop() {
        let mut timer = CpuTimer::new(10_000);
        timer.start();
        busy_work(Duration::from_millis(20));
        timer.stop();
        let first = timer.accumulated_ms();

        timer.start();
        busy_work(Duration::from_millis(20));
        timer.stop();
        assert!(timer.accumulated_ms() > first);
    }

    #[test]
    fn wall_clock_not_expired_initially() {
        let guard = WallClockGuard::new(5000);
        assert!(!guard.is_expired());
        assert!(guard.remaining() > Duration::from_millis(4000));
    }

    #[test]
    fn wall_clock_expires() {
        let guard = WallClockGuard::new(10);
        thread::sleep(Duration::from_millis(30));
        assert!(guard.is_expired());
        assert_eq!(guard.remaining(), Duration::ZERO);
    }

    #[test]
    fn wall_clock_remaining_decreases() {
        let guard = WallClockGuard::new(1000);
        let r1 = guard.remaining();
        thread::sleep(Duration::from_millis(50));
        let r2 = guard.remaining();
        assert!(r2 < r1);
    }

    #[test]
    fn cpu_timer_reset_clears_accumulated() {
        let mut timer = CpuTimer::new(10_000);
        timer.start();
        busy_work(Duration::from_millis(30));
        timer.stop();
        assert!(timer.accumulated_ms() >= 20);

        timer.reset();
        assert_eq!(timer.accumulated_ms(), 0);
        assert!(!timer.is_exceeded());
    }

    #[test]
    fn cpu_timer_reset_clears_exceeded_flag() {
        let mut timer = CpuTimer::new(10); // Very low limit
        let flag = timer.exceeded_flag();

        timer.start();
        busy_work(Duration::from_millis(30));
        timer.stop();
        assert!(timer.is_exceeded());
        assert!(flag.load(Ordering::Relaxed));

        timer.reset();
        assert!(!timer.is_exceeded());
        assert!(!flag.load(Ordering::Relaxed));
    }

    #[test]
    fn cpu_timer_usable_after_reset() {
        let mut timer = CpuTimer::new(10); // Very low limit

        // First run - exceed limit
        timer.start();
        busy_work(Duration::from_millis(30));
        timer.stop();
        assert!(timer.is_exceeded());

        // Reset and use again
        timer.reset();
        timer.start();
        busy_work(Duration::from_millis(5));
        timer.stop();
        // Should not be exceeded with only 5ms
        assert!(!timer.is_exceeded());
    }

    #[test]
    fn cpu_timer_supports_thread_cpu_time_or_falls_back() {
        // This test documents the intended behavior: use thread CPU clock when
        // available, otherwise run in wall-clock compatibility mode.
        let mut timer = CpuTimer::new(1_000);
        timer.start();
        busy_work(Duration::from_millis(10));
        let elapsed = timer.stop();
        assert!(elapsed > 0);
    }

    #[test]
    #[ignore = "benchmark-style comparison; run manually"]
    fn benchmark_wall_clock_vs_thread_cpu_time() {
        let sleep_for = Duration::from_millis(100);
        let busy_for = Duration::from_millis(100);

        let wall_sleep_start = Instant::now();
        thread::sleep(sleep_for);
        let wall_sleep_ms = wall_sleep_start.elapsed().as_millis() as u64;

        let wall_busy_start = Instant::now();
        busy_work(busy_for);
        let wall_busy_ms = wall_busy_start.elapsed().as_millis() as u64;

        if let (Some(cpu_sleep_ms), Some(cpu_busy_ms)) = (
            thread_cpu_elapsed_ms_for(sleep_for, false),
            thread_cpu_elapsed_ms_for(busy_for, true),
        ) {
            eprintln!(
                "benchmark cpu_timer: wall_sleep={}ms cpu_sleep={}ms wall_busy={}ms cpu_busy={}ms",
                wall_sleep_ms, cpu_sleep_ms, wall_busy_ms, cpu_busy_ms
            );

            // For sleep-heavy sections, thread CPU time should be much smaller.
            assert!(cpu_sleep_ms <= wall_sleep_ms / 3 + 2);
            // For busy sections, both clocks should be of same order of magnitude.
            assert!(cpu_busy_ms > 0);
            assert!(wall_busy_ms > 0);
        } else {
            eprintln!(
                "benchmark cpu_timer: thread CPU clock unavailable, wall_sleep={}ms wall_busy={}ms",
                wall_sleep_ms, wall_busy_ms
            );
            assert!(wall_sleep_ms > 0 && wall_busy_ms > 0);
        }
    }
}

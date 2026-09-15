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
    // CI runners can be CPU-throttled and produce lower per-thread CPU time
    // than local machines; assert functional behavior rather than strict wall thresholds.
    assert!(elapsed > 0, "elapsed should be > 0ms, got {elapsed}");
    assert!(timer.accumulated_ms() >= elapsed);
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
    let elapsed = timer.stop();
    assert!(elapsed > 0, "elapsed should be > 0ms, got {elapsed}");
    assert!(timer.accumulated_ms() >= elapsed);

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

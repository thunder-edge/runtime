use super::*;

#[test]
fn mem_check_new_stores_limit() {
    let mc = MemCheck::new(1024);
    assert_eq!(mc.limit_bytes(), 1024);
}

#[test]
fn mem_check_zero_limit() {
    let mc = MemCheck::new(0);
    assert_eq!(mc.limit_bytes(), 0);
}

#[test]
fn global_memory_tracker_starts_zero() {
    let tracker = GlobalMemoryTracker::new();
    assert_eq!(tracker.total(), 0);
}

#[test]
fn global_memory_tracker_add_sub() {
    let tracker = GlobalMemoryTracker::new();
    tracker.add(100);
    tracker.add(200);
    assert_eq!(tracker.total(), 300);
    tracker.sub(50);
    assert_eq!(tracker.total(), 250);
}

#[test]
fn global_memory_tracker_default() {
    let tracker = GlobalMemoryTracker::default();
    assert_eq!(tracker.total(), 0);
}

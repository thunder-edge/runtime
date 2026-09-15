use super::*;

fn deterministic_test_config() -> ConnectionManagerConfig {
    ConnectionManagerConfig {
        // Keep unit tests independent from host FD limits and current process FD usage.
        fd_reserved_absolute: 0,
        fd_reserved_ratio: 0.0,
        queue_max_waiters: 8,
        default_wait_timeout: Duration::from_millis(50),
        per_tenant_max_active: 8,
        lease_hard_ttl: Duration::from_secs(5),
        min_token_refill_per_sec: 1_000.0,
    }
}

#[tokio::test]
async fn acquire_and_release_updates_snapshot() {
    let manager = ConnectionManager::new(deterministic_test_config());

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
        default_wait_timeout: Duration::from_millis(25),
        ..deterministic_test_config()
    });

    let lease_id = manager
        .acquire_lease("tenant-limited".to_string(), "exec-a".to_string(), None)
        .await
        .expect("first lease should be acquired");

    let second = manager
        .acquire_lease(
            "tenant-limited".to_string(),
            "exec-b".to_string(),
            Some(Duration::from_millis(25)),
        )
        .await;
    assert!(matches!(second, Err(AcquireError::Timeout)));

    assert!(manager.release_lease(lease_id));
}

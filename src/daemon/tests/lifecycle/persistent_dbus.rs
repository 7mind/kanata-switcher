use super::*;

#[test]
fn test_dbus_reconnect_delay_caps_at_two_seconds() {
    assert_eq!(
        dbus_reconnect_delay(0),
        std::time::Duration::from_millis(250)
    );
    assert_eq!(
        dbus_reconnect_delay(1),
        std::time::Duration::from_millis(1000)
    );
    assert_eq!(
        dbus_reconnect_delay(2),
        std::time::Duration::from_millis(2000)
    );
    assert_eq!(
        dbus_reconnect_delay(3),
        std::time::Duration::from_millis(2000)
    );
    assert_eq!(
        dbus_reconnect_delay(32),
        std::time::Duration::from_millis(2000)
    );
}

#[tokio::test]
async fn test_wait_for_dbus_reconnect_retry_backoffs_for_name_lost_monitor_setup_errors() {
    with_test_timeout(async {
        let restart_handle = RestartHandle::new();
        let shutdown_handle = ShutdownHandle::new();
        let mut restart_receiver = restart_handle.subscribe();
        let mut shutdown_receiver = shutdown_handle.subscribe();
        let mut reconnect_attempt = 0usize;

        let first_start = std::time::Instant::now();
        wait_for_dbus_reconnect_retry(
            &mut reconnect_attempt,
            &mut shutdown_receiver,
            &mut restart_receiver,
            "[DBus] Failed to create DBus proxy for name-loss monitoring",
            std::io::Error::other("proxy setup failure").to_string(),
        )
        .await;
        let first_elapsed = first_start.elapsed();

        assert!(
            first_elapsed >= std::time::Duration::from_millis(200),
            "proxy setup failure should back off before retrying (elapsed: {:?})",
            first_elapsed
        );
        assert_eq!(reconnect_attempt, 1);

        reconnect_attempt = 20;
        let capped_start = std::time::Instant::now();
        wait_for_dbus_reconnect_retry(
            &mut reconnect_attempt,
            &mut shutdown_receiver,
            &mut restart_receiver,
            "[DBus] Failed to subscribe to NameLost",
            std::io::Error::other("NameLost subscription setup failure").to_string(),
        )
        .await;
        let capped_elapsed = capped_start.elapsed();

        assert!(
            capped_elapsed >= std::time::Duration::from_millis(1900),
            "NameLost setup failure should use capped backoff before retrying (elapsed: {:?})",
            capped_elapsed
        );
        assert_eq!(reconnect_attempt, 21);
    })
    .await;
}

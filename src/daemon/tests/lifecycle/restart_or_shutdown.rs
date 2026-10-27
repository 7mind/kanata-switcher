use super::*;

#[tokio::test]
async fn test_wait_for_restart_or_shutdown_returns_restart() {
    with_test_timeout(async {
        let restart_handle = RestartHandle::new();
        let shutdown_handle = ShutdownHandle::new();

        let wait_future = wait_for_restart_or_shutdown(&restart_handle, &shutdown_handle);
        let trigger_future = async {
            tokio::task::yield_now().await;
            restart_handle.request();
        };
        let (outcome, _) = tokio::join!(wait_future, trigger_future);
        assert_eq!(outcome, RunOutcome::Restart);
    })
    .await;
}

#[tokio::test]
async fn test_wait_for_restart_or_shutdown_returns_exit() {
    with_test_timeout(async {
        let restart_handle = RestartHandle::new();
        let shutdown_handle = ShutdownHandle::new();

        shutdown_handle.request();

        let outcome = wait_for_restart_or_shutdown(&restart_handle, &shutdown_handle).await;
        assert_eq!(outcome, RunOutcome::Exit);
    })
    .await;
}

#[tokio::test]
async fn test_wait_for_restart_or_shutdown_shutdown_wins() {
    with_test_timeout(async {
        let restart_handle = RestartHandle::new();
        let shutdown_handle = ShutdownHandle::new();

        shutdown_handle.request();
        restart_handle.request();

        let outcome = wait_for_restart_or_shutdown(&restart_handle, &shutdown_handle).await;
        assert_eq!(outcome, RunOutcome::Exit);
    })
    .await;
}

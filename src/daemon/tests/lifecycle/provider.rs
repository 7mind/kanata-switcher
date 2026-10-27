use super::*;

#[tokio::test]
async fn test_lifecycle_provider_startup_variant_emits_once() {
    with_test_timeout(async {
        let mut provider =
            LifecycleProvider::Startup(StartupSnapshotProvider::new(Environment::X11));
        assert!(!provider.is_continuous());

        let first = provider
            .next_snapshot()
            .await
            .expect("startup provider should emit initial snapshot");
        assert_eq!(first.session_kind, SessionKind::GraphicalX11);
        assert!(provider.next_snapshot().await.is_none());
    })
    .await;
}

#[tokio::test]
async fn test_lifecycle_provider_build_falls_back_to_startup_when_logind_init_fails() {
    with_test_timeout(async {
        let mut provider =
            LifecycleProvider::build_with_logind_factory(Environment::Wayland, || async {
                Err(std::io::Error::other(
                    "org.freedesktop.DBus.Error.ServiceUnknown: org.freedesktop.login1",
                )
                .into())
            })
            .await;

        assert!(!provider.is_continuous());
        let first = provider
            .next_snapshot()
            .await
            .expect("startup fallback provider should emit initial snapshot");
        assert_eq!(first.session_kind, SessionKind::GraphicalWayland);
        assert!(provider.next_snapshot().await.is_none());
    })
    .await;
}

#[tokio::test]
async fn test_lifecycle_provider_build_uses_logind_provider_when_init_succeeds() {
    with_test_timeout(async {
        let (sender, receiver) = mpsc::unbounded_channel();
        sender
            .send(LifecycleSnapshot {
                active: true,
                session_type: "tty".to_string(),
                session_kind: SessionKind::NativeTerminal,
            })
            .expect("snapshot send should succeed");
        drop(sender);

        let mut provider =
            LifecycleProvider::build_with_logind_factory(Environment::Unknown, || async move {
                Ok(LogindLifecycleProvider { receiver })
            })
            .await;

        assert!(provider.is_continuous());
        let first = provider
            .next_snapshot()
            .await
            .expect("logind provider should emit snapshot");
        assert_eq!(first.session_kind, SessionKind::NativeTerminal);
        assert!(provider.next_snapshot().await.is_none());
    })
    .await;
}

#[tokio::test]
async fn test_lifecycle_provider_startup_is_not_continuous_after_exhaustion() {
    with_test_timeout(async {
        let mut provider =
            LifecycleProvider::Startup(StartupSnapshotProvider::new(Environment::Unknown));
        assert!(!provider.is_continuous());
        let _ = provider.next_snapshot().await;
        assert!(!provider.is_continuous());
        assert!(provider.next_snapshot().await.is_none());
    })
    .await;
}

#[tokio::test]
async fn test_startup_snapshot_provider_emits_once() {
    with_test_timeout(async {
        let mut provider = StartupSnapshotProvider::new(Environment::Wayland);
        let first = provider
            .next_snapshot()
            .await
            .expect("startup snapshot should emit once");
        assert_eq!(first.session_kind, SessionKind::GraphicalWayland);
        assert!(provider.next_snapshot().await.is_none());
    })
    .await;
}

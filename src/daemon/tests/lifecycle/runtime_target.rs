use super::*;

#[test]
fn test_resolve_runtime_target_matrix() {
    let none = DesktopCapabilities {
        gnome_owner: false,
        kde_owner: false,
    };
    let gnome = DesktopCapabilities {
        gnome_owner: true,
        kde_owner: false,
    };
    let gnome_and_kde = DesktopCapabilities {
        gnome_owner: true,
        kde_owner: true,
    };
    let kde = DesktopCapabilities {
        gnome_owner: false,
        kde_owner: true,
    };

    assert_eq!(
        resolve_runtime_target(SessionKind::NoSession, none),
        RuntimeTarget::Idle
    );
    assert_eq!(
        resolve_runtime_target(SessionKind::NativeTerminal, none),
        RuntimeTarget::Backend(BackendKind::LinuxConsole)
    );
    assert_eq!(
        resolve_runtime_target(SessionKind::GraphicalX11, none),
        RuntimeTarget::Backend(BackendKind::X11)
    );
    assert_eq!(
        resolve_runtime_target(SessionKind::GraphicalWayland, gnome),
        RuntimeTarget::Backend(BackendKind::Gnome)
    );
    assert_eq!(
        resolve_runtime_target(SessionKind::GraphicalWayland, kde),
        RuntimeTarget::Backend(BackendKind::Kde)
    );
    assert_eq!(
        resolve_runtime_target(SessionKind::GraphicalWayland, gnome_and_kde),
        RuntimeTarget::Backend(BackendKind::Gnome)
    );
    assert_eq!(
        resolve_runtime_target(
            SessionKind::GraphicalWayland,
            DesktopCapabilities {
                gnome_owner: false,
                kde_owner: false,
            }
        ),
        RuntimeTarget::Backend(BackendKind::Wayland)
    );
}

#[test]
fn test_startup_snapshot_wayland_prefers_gnome_when_gnome_shell_is_present() {
    let snapshot = startup_environment_to_snapshot(Environment::Wayland);
    assert_eq!(snapshot.session_kind, SessionKind::GraphicalWayland);

    let target = resolve_runtime_target(
        snapshot.session_kind,
        DesktopCapabilities {
            gnome_owner: true,
            kde_owner: false,
        },
    );
    assert_eq!(target, RuntimeTarget::Backend(BackendKind::Gnome));
}

#[test]
fn test_target_requires_session_bus() {
    assert!(target_requires_session_bus(RuntimeTarget::Backend(
        BackendKind::Gnome
    )));
    assert!(target_requires_session_bus(RuntimeTarget::Backend(
        BackendKind::Kde
    )));
    assert!(target_requires_session_bus(RuntimeTarget::Backend(
        BackendKind::Wayland
    )));
    assert!(target_requires_session_bus(RuntimeTarget::Backend(
        BackendKind::X11
    )));
    assert!(!target_requires_session_bus(RuntimeTarget::Backend(
        BackendKind::LinuxConsole
    )));
    assert!(!target_requires_session_bus(RuntimeTarget::Idle));
}

#[test]
fn test_startup_environment_to_snapshot_mapping() {
    let gnome = startup_environment_to_snapshot(Environment::Gnome);
    assert!(gnome.active);
    assert_eq!(gnome.session_type, "gnome");
    assert_eq!(gnome.session_kind, SessionKind::GraphicalWayland);

    let x11 = startup_environment_to_snapshot(Environment::X11);
    assert!(x11.active);
    assert_eq!(x11.session_type, "x11");
    assert_eq!(x11.session_kind, SessionKind::GraphicalX11);

    let linux_console = startup_environment_to_snapshot(Environment::LinuxConsoleWithLogind);
    assert!(linux_console.active);
    assert_eq!(linux_console.session_type, "tty");
    assert_eq!(linux_console.session_kind, SessionKind::NativeTerminal);

    let unknown = startup_environment_to_snapshot(Environment::Unknown);
    assert!(!unknown.active);
    assert_eq!(unknown.session_type, "");
    assert_eq!(unknown.session_kind, SessionKind::NoSession);

    let kde = startup_environment_to_snapshot(Environment::Kde);
    assert!(kde.active);
    assert_eq!(kde.session_type, "kde");
    assert_eq!(kde.session_kind, SessionKind::GraphicalWayland);

    let wayland = startup_environment_to_snapshot(Environment::Wayland);
    assert!(wayland.active);
    assert_eq!(wayland.session_type, "wayland");
    assert_eq!(wayland.session_kind, SessionKind::GraphicalWayland);
}

#[test]
fn test_runtime_target_label_is_stable() {
    assert_eq!(runtime_target_label(RuntimeTarget::Idle), "idle");
    assert_eq!(
        runtime_target_label(RuntimeTarget::Backend(BackendKind::Gnome)),
        "gnome"
    );
    assert_eq!(
        runtime_target_label(RuntimeTarget::Backend(BackendKind::Kde)),
        "kde"
    );
    assert_eq!(
        runtime_target_label(RuntimeTarget::Backend(BackendKind::Wayland)),
        "wayland"
    );
    assert_eq!(
        runtime_target_label(RuntimeTarget::Backend(BackendKind::X11)),
        "x11"
    );
    assert_eq!(
        runtime_target_label(RuntimeTarget::Backend(BackendKind::LinuxConsole)),
        "linux-console"
    );
}

#[test]
fn test_runtime_target_to_environment_mapping() {
    assert_eq!(
        runtime_target_to_environment(RuntimeTarget::Backend(BackendKind::Gnome)),
        Environment::Gnome
    );
    assert_eq!(
        runtime_target_to_environment(RuntimeTarget::Backend(BackendKind::Kde)),
        Environment::Kde
    );
    assert_eq!(
        runtime_target_to_environment(RuntimeTarget::Backend(BackendKind::Wayland)),
        Environment::Wayland
    );
    assert_eq!(
        runtime_target_to_environment(RuntimeTarget::Backend(BackendKind::X11)),
        Environment::X11
    );
    assert_eq!(
        runtime_target_to_environment(RuntimeTarget::Backend(BackendKind::LinuxConsole)),
        Environment::LinuxConsoleWithLogind
    );
    assert_eq!(
        runtime_target_to_environment(RuntimeTarget::Idle),
        Environment::Unknown
    );
}

#[test]
fn test_resolve_desktop_flavor_wayland_precedence() {
    let gnome_and_kde = DesktopCapabilities {
        gnome_owner: true,
        kde_owner: true,
    };
    assert_eq!(
        resolve_desktop_flavor(SessionKind::GraphicalWayland, gnome_and_kde),
        DesktopFlavor::Gnome
    );

    let kde = DesktopCapabilities {
        gnome_owner: false,
        kde_owner: true,
    };
    assert_eq!(
        resolve_desktop_flavor(SessionKind::GraphicalWayland, kde),
        DesktopFlavor::Kde
    );

    let none = DesktopCapabilities {
        gnome_owner: false,
        kde_owner: false,
    };
    assert_eq!(
        resolve_desktop_flavor(SessionKind::GraphicalWayland, none),
        DesktopFlavor::GenericWayland
    );
    assert_eq!(
        resolve_desktop_flavor(SessionKind::GraphicalX11, none),
        DesktopFlavor::X11
    );
    assert_eq!(
        resolve_desktop_flavor(SessionKind::NoSession, none),
        DesktopFlavor::Unknown
    );
}

#[tokio::test]
async fn test_resolve_runtime_target_for_non_wayland_snapshot() {
    with_test_timeout(async {
        let x11_snapshot = LifecycleSnapshot {
            active: true,
            session_type: "x11".to_string(),
            session_kind: SessionKind::GraphicalX11,
        };
        let x11_target = resolve_runtime_target_for_snapshot(&x11_snapshot)
            .await
            .expect("x11 snapshot resolution should succeed");
        assert_eq!(x11_target, RuntimeTarget::Backend(BackendKind::X11));

        let idle_snapshot = LifecycleSnapshot {
            active: false,
            session_type: "".to_string(),
            session_kind: SessionKind::NoSession,
        };
        let idle_target = resolve_runtime_target_for_snapshot(&idle_snapshot)
            .await
            .expect("idle snapshot resolution should succeed");
        assert_eq!(idle_target, RuntimeTarget::Idle);
    })
    .await;
}

#[tokio::test]
async fn test_resolve_runtime_target_for_startup_gnome_snapshot_uses_explicit_hint() {
    with_test_timeout(async {
        let gnome_snapshot = LifecycleSnapshot {
            active: true,
            session_type: "gnome".to_string(),
            session_kind: SessionKind::GraphicalWayland,
        };
        let gnome_target = resolve_runtime_target_for_snapshot(&gnome_snapshot)
            .await
            .expect("gnome startup hint should resolve without capability probe");
        assert_eq!(gnome_target, RuntimeTarget::Backend(BackendKind::Gnome));
    })
    .await;
}

#[tokio::test]
async fn test_resolve_runtime_target_for_startup_kde_snapshot_uses_explicit_hint() {
    with_test_timeout(async {
        let kde_snapshot = LifecycleSnapshot {
            active: true,
            session_type: "kde".to_string(),
            session_kind: SessionKind::GraphicalWayland,
        };
        let kde_target = resolve_runtime_target_for_snapshot(&kde_snapshot)
            .await
            .expect("kde startup hint should resolve without capability probe");
        assert_eq!(kde_target, RuntimeTarget::Backend(BackendKind::Kde));
    })
    .await;
}

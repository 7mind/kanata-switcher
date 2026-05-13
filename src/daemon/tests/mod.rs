use super::*;
use clap::Parser;
use proptest::prelude::*;
use std::future::Future;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, watch};
use zbus::Message;
use zbus::zvariant::OwnedObjectPath;


mod common;
pub(crate) use common::*;

mod focus_flow;
mod focus_pipeline;
mod focus_property;
mod autostart;
mod dbus_naming;
mod control_commands;
mod kde_script_paths;
mod sni_presentation;
mod gnome_ext_state;
mod config_parsing;

// === Misfiled tests (to be moved to lifecycle/ in M2-PR-03) ===

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

#[test]
fn test_logind_no_session_error_detection() {
    use zbus::names::OwnedErrorName;
    use zbus::{Error as ZbusError, Message};

    let name = OwnedErrorName::try_from(LOGIND_ERROR_NO_SESSION_FOR_PID).unwrap();
    let reply = Message::method_call("/org/freedesktop/login1", "GetSessionByPID")
        .unwrap()
        .build(&())
        .unwrap();
    let error = ZbusError::MethodError(name, Some("no session".to_string()), reply);

    assert!(is_logind_no_session_error(&error));
}

#[test]
fn test_logind_no_session_error_detection_false() {
    use zbus::names::OwnedErrorName;
    use zbus::{Error as ZbusError, Message};

    let name = OwnedErrorName::try_from("org.freedesktop.login1.NoSuchSession").unwrap();
    let reply = Message::method_call("/org/freedesktop/login1", "GetSession")
        .unwrap()
        .build(&())
        .unwrap();
    let error = ZbusError::MethodError(name, Some("no session".to_string()), reply);

    assert!(!is_logind_no_session_error(&error));
}

#[test]
fn test_logind_empty_object_path_detection() {
    let empty = OwnedObjectPath::try_from(LOGIND_EMPTY_OBJECT_PATH).unwrap();
    let non_empty = OwnedObjectPath::try_from("/org/freedesktop/login1/session/_1").unwrap();

    assert!(is_logind_empty_object_path(&empty));
    assert!(!is_logind_empty_object_path(&non_empty));
}

#[test]
fn test_parse_logind_object_path_value() {
    use zbus::zvariant::ObjectPath;

    let path = ObjectPath::try_from("/org/freedesktop/login1/session/_1").unwrap();
    let value = OwnedValue::from(path);
    let parsed = parse_logind_object_path(value, "test").unwrap();

    assert_eq!(parsed.as_str(), "/org/freedesktop/login1/session/_1");
}

#[test]
fn test_parse_logind_object_path_structure_single_field() {
    use zbus::zvariant::{ObjectPath, StructureBuilder};

    let path = ObjectPath::try_from("/org/freedesktop/login1/session/_1").unwrap();
    let structure = StructureBuilder::new().add_field(path).build().unwrap();
    let value = OwnedValue::try_from(structure).unwrap();
    let parsed = parse_logind_object_path(value, "test").unwrap();

    assert_eq!(parsed.as_str(), "/org/freedesktop/login1/session/_1");
}

#[test]
fn test_parse_logind_object_path_string() {
    use zbus::zvariant::Str;

    let value = OwnedValue::from(Str::from("/org/freedesktop/login1/session/_1"));
    let parsed = parse_logind_object_path(value, "test").unwrap();

    assert_eq!(parsed.as_str(), "/org/freedesktop/login1/session/_1");
}

#[test]
fn test_decode_logind_object_path_reply_object_path() {
    use zbus::zvariant::ObjectPath;

    let path = ObjectPath::try_from("/org/freedesktop/login1/session/_1").unwrap();
    let reply = Message::method_call("/org/freedesktop/login1", "GetSession")
        .unwrap()
        .build(&path)
        .unwrap();
    let parsed = decode_logind_object_path_reply(&reply, "test").unwrap();

    assert_eq!(parsed.as_str(), "/org/freedesktop/login1/session/_1");
}

#[test]
fn test_decode_logind_object_path_reply_structure() {
    use zbus::zvariant::{ObjectPath, StructureBuilder};

    let path = ObjectPath::try_from("/org/freedesktop/login1/session/_1").unwrap();
    let structure = StructureBuilder::new().add_field(path).build().unwrap();
    let reply = Message::method_call("/org/freedesktop/login1", "GetSession")
        .unwrap()
        .build(&structure)
        .unwrap();
    let parsed = decode_logind_object_path_reply(&reply, "test").unwrap();

    assert_eq!(parsed.as_str(), "/org/freedesktop/login1/session/_1");
}

#[test]
fn test_decode_logind_object_path_reply_structure_multi_field() {
    use zbus::zvariant::{ObjectPath, StructureBuilder};

    let path = ObjectPath::try_from("/org/freedesktop/login1/session/_311").unwrap();
    let structure = StructureBuilder::new()
        .add_field("11")
        .add_field(path)
        .build()
        .unwrap();
    let reply = Message::method_call("/org/freedesktop/login1", "GetSession")
        .unwrap()
        .build(&structure)
        .unwrap();
    let parsed = decode_logind_object_path_reply(&reply, "test").unwrap();

    assert_eq!(parsed.as_str(), "/org/freedesktop/login1/session/_311");
}

#[test]
fn test_decode_logind_object_path_reply_variant() {
    use zbus::zvariant::{ObjectPath, Value};

    let path = ObjectPath::try_from("/org/freedesktop/login1/session/_1").unwrap();
    let value = OwnedValue::try_from(Value::from(path)).unwrap();
    let reply = Message::method_call("/org/freedesktop/login1", "GetSession")
        .unwrap()
        .build(&value)
        .unwrap();
    let parsed = decode_logind_object_path_reply(&reply, "test").unwrap();

    assert_eq!(parsed.as_str(), "/org/freedesktop/login1/session/_1");
}


// === Runtime Lifecycle Tests ===

fn test_backend_context_with_gnome_setup<F>(gnome_setup_hook: F) -> BackendContext
where
    F: Fn(bool) + Send + Sync + 'static,
{
    let status_broadcaster = StatusBroadcaster::new();
    BackendContext {
        kanata: KanataClient::new(
            "127.0.0.1",
            10000,
            Some("default".to_string()),
            true,
            status_broadcaster.clone(),
        ),
        handler: Arc::new(Mutex::new(FocusHandler::new(Vec::new(), None, true))),
        status_broadcaster,
        restart_handle: RestartHandle::new(),
        pause_broadcaster: PauseBroadcaster::new(),
        runtime_environment: RuntimeEnvironmentBroadcaster::new(Environment::Unknown),
        install_gnome_extension: true,
        gnome_setup_completed: Arc::new(AtomicBool::new(false)),
        gnome_setup_hook: Arc::new(gnome_setup_hook),
        effective_dbus_name: effective_dbus_name(&derive_default_dbus_suffix("127.0.0.1", 10000)),
    }
}

fn test_backend_context() -> BackendContext {
    test_backend_context_with_gnome_setup(|_| {})
}

fn test_running_backend_handle(kind: BackendKind, stopped: Arc<AtomicBool>) -> BackendHandle {
    let shutdown_handle = ShutdownHandle::new();
    let mut receiver = shutdown_handle.subscribe();
    let (finished_tx, finished_rx) = watch::channel(false);
    let join_handle = tokio::spawn(async move {
        while !*receiver.borrow() {
            if receiver.changed().await.is_err() {
                break;
            }
        }
        stopped.store(true, Ordering::SeqCst);
        let _ = finished_tx.send(true);
        Ok(BackendExit::Exit)
    });
    BackendHandle {
        kind,
        shutdown_handle,
        join_handle: Some(join_handle),
        finished_rx,
    }
}

fn test_finished_backend_handle(kind: BackendKind, exit: BackendExit) -> BackendHandle {
    let shutdown_handle = ShutdownHandle::new();
    let (finished_tx, finished_rx) = watch::channel(false);
    let join_handle = tokio::spawn(async move {
        let _ = finished_tx.send(true);
        Ok(exit)
    });
    BackendHandle {
        kind,
        shutdown_handle,
        join_handle: Some(join_handle),
        finished_rx,
    }
}

#[test]
fn test_session_type_to_session_kind_mappings() {
    assert!(session_type_indicates_native_terminal("tty"));
    assert!(!session_type_indicates_native_terminal("wayland"));
    assert_eq!(
        session_type_to_session_kind(true, "tty"),
        SessionKind::NativeTerminal
    );
    assert_eq!(
        session_type_to_session_kind(true, "x11"),
        SessionKind::GraphicalX11
    );
    assert_eq!(
        session_type_to_session_kind(true, "wayland"),
        SessionKind::GraphicalWayland
    );
    assert_eq!(
        session_type_to_session_kind(true, "gnome"),
        SessionKind::GraphicalWayland
    );
    assert_eq!(
        session_type_to_session_kind(true, "kde"),
        SessionKind::GraphicalWayland
    );
    assert_eq!(
        session_type_to_session_kind(true, "mir"),
        SessionKind::NoSession
    );
    assert_eq!(
        session_type_to_session_kind(false, "wayland"),
        SessionKind::NoSession
    );
}

#[test]
fn test_validate_active_logind_session_type_rejects_empty_for_active_session() {
    let result = validate_active_logind_session_type(true, "");
    assert_eq!(
        result,
        Err("[Lifecycle] logind Type property is empty for an active session")
    );
}

#[test]
fn test_validate_active_logind_session_type_allows_empty_for_inactive_session() {
    let result = validate_active_logind_session_type(false, "");
    assert_eq!(result, Ok(()));
}

#[test]
fn test_validate_active_logind_session_type_allows_non_empty_for_active_session() {
    let result = validate_active_logind_session_type(true, "wayland");
    assert_eq!(result, Ok(()));
}

#[test]
fn test_active_unknown_session_type_resolves_to_idle_target() {
    let session_kind = session_type_to_session_kind(true, "mir");
    assert_eq!(session_kind, SessionKind::NoSession);
    let target = resolve_runtime_target(
        session_kind,
        DesktopCapabilities {
            gnome_owner: false,
            kde_owner: false,
        },
    );
    assert_eq!(target, RuntimeTarget::Idle);
}

#[test]
fn test_normalize_display_override_rejects_x11_style_for_wayland() {
    assert_eq!(normalize_display_override(BackendKind::Wayland, ":0"), None);
    assert_eq!(normalize_display_override(BackendKind::Wayland, ":1"), None);
    assert_eq!(
        normalize_display_override(BackendKind::Wayland, "nested/socket"),
        None
    );
}

#[test]
fn test_normalize_display_override_accepts_wayland_socket_values() {
    assert_eq!(
        normalize_display_override(BackendKind::Wayland, "wayland-0"),
        Some("wayland-0".to_string())
    );
    assert_eq!(
        normalize_display_override(BackendKind::Wayland, " /run/user/1000/wayland-1 "),
        Some("/run/user/1000/wayland-1".to_string())
    );
}

#[test]
fn test_normalize_display_override_preserves_x11_display_values() {
    assert_eq!(
        normalize_display_override(BackendKind::X11, ":0"),
        Some(":0".to_string())
    );
}

#[test]
fn test_decode_logind_change_emits_on_type_change_without_active_change() {
    use zbus::zvariant::{Str, Value};

    let type_value = Value::from(Str::from("x11"));
    let snapshot =
        decode_logind_lifecycle_snapshot_change(true, "wayland", None, Some(&type_value))
            .expect("logind decode should succeed")
            .expect("type change should emit snapshot");
    assert!(snapshot.active);
    assert_eq!(snapshot.session_type, "x11");
    assert_eq!(snapshot.session_kind, SessionKind::GraphicalX11);
}

#[test]
fn test_decode_logind_change_skips_duplicate_active_and_type_values() {
    use zbus::zvariant::{Str, Value};

    let active_value = Value::from(true);
    let type_value = Value::from(Str::from("wayland"));
    assert!(
        decode_logind_lifecycle_snapshot_change(
            true,
            "wayland",
            Some(&active_value),
            Some(&type_value)
        )
        .expect("logind decode should succeed")
        .is_none(),
        "duplicate values should not emit snapshots"
    );
}

#[test]
fn test_decode_logind_change_errors_on_invalid_active_value() {
    use zbus::zvariant::{Str, Value};

    let active_value = Value::from(Str::from("true"));
    let result =
        decode_logind_lifecycle_snapshot_change(true, "wayland", Some(&active_value), None);
    assert_eq!(
        result,
        Err("[Lifecycle] Failed to parse logind Active property".to_string())
    );
}

#[test]
fn test_decode_logind_change_errors_on_invalid_type_value() {
    use zbus::zvariant::Value;

    let type_value = Value::from(7i32);
    let result = decode_logind_lifecycle_snapshot_change(true, "wayland", None, Some(&type_value));
    assert_eq!(
        result,
        Err("[Lifecycle] Failed to parse logind Type property".to_string())
    );
}

#[test]
fn test_decode_logind_change_errors_when_active_snapshot_has_empty_type() {
    use zbus::zvariant::Value;

    let active_value = Value::from(true);
    let result = decode_logind_lifecycle_snapshot_change(false, "", Some(&active_value), None);
    assert_eq!(
        result,
        Err("[Lifecycle] logind Type property is empty for an active session".to_string())
    );
}

#[test]
fn test_decode_logind_display_path_change_returns_unchanged_without_display_update() {
    assert_eq!(
        decode_logind_display_path_change(None).expect("decode should succeed"),
        LogindDisplayPathChange::Unchanged
    );
}

#[test]
fn test_decode_logind_display_path_change_parses_object_path() {
    use zbus::zvariant::{ObjectPath, Value};

    let display =
        ObjectPath::try_from("/org/freedesktop/login1/session/_42").expect("valid object path");
    let value = Value::from(display);
    assert_eq!(
        decode_logind_display_path_change(Some(&value)).expect("decode should succeed"),
        LogindDisplayPathChange::Path(
            OwnedObjectPath::try_from("/org/freedesktop/login1/session/_42")
                .expect("valid object path")
        )
    );
}

#[test]
fn test_decode_logind_display_path_change_reports_empty_path() {
    use zbus::zvariant::{ObjectPath, Value};

    let display = ObjectPath::try_from("/").expect("valid object path");
    let value = Value::from(display);
    assert_eq!(
        decode_logind_display_path_change(Some(&value)).expect("decode should succeed"),
        LogindDisplayPathChange::Empty
    );
}

#[test]
fn test_decode_logind_display_path_change_parses_variant_wrapped_structure() {
    use zbus::zvariant::{ObjectPath, StructureBuilder, Value};

    let display =
        ObjectPath::try_from("/org/freedesktop/login1/session/_42").expect("valid object path");
    let structure = StructureBuilder::new()
        .add_field(display)
        .build()
        .expect("structure should build");
    let value = Value::Value(Box::new(Value::from(structure)));
    assert_eq!(
        decode_logind_display_path_change(Some(&value)).expect("decode should succeed"),
        LogindDisplayPathChange::Path(
            OwnedObjectPath::try_from("/org/freedesktop/login1/session/_42")
                .expect("valid object path")
        )
    );
}

#[test]
fn test_decode_logind_display_path_change_parses_variant_wrapped_structure_empty_path() {
    use zbus::zvariant::{ObjectPath, StructureBuilder, Value};

    let display = ObjectPath::try_from("/").expect("valid object path");
    let structure = StructureBuilder::new()
        .add_field(display)
        .build()
        .expect("structure should build");
    let value = Value::Value(Box::new(Value::from(structure)));
    assert_eq!(
        decode_logind_display_path_change(Some(&value)).expect("decode should succeed"),
        LogindDisplayPathChange::Empty
    );
}

#[test]
fn test_decode_logind_display_path_change_errors_on_invalid_value() {
    use zbus::zvariant::Value;

    let value = Value::from(42u32);
    assert_eq!(
        decode_logind_display_path_change(Some(&value)),
        Err("[Lifecycle] Failed to parse logind User.Display property change".to_string())
    );
}

#[test]
fn test_apply_logind_display_change_reattaches_for_new_display_session_path() {
    let current =
        OwnedObjectPath::try_from("/org/freedesktop/login1/session/_1").expect("valid object path");
    let next =
        OwnedObjectPath::try_from("/org/freedesktop/login1/session/_2").expect("valid object path");
    assert_eq!(
        apply_logind_display_change(
            &current,
            true,
            false,
            "",
            LogindDisplayPathChange::Path(next.clone())
        ),
        LogindDisplayChangeAction::Reattach(next)
    );
}

#[test]
fn test_apply_logind_display_change_ignores_same_display_session_path() {
    let current =
        OwnedObjectPath::try_from("/org/freedesktop/login1/session/_1").expect("valid object path");
    assert_eq!(
        apply_logind_display_change(
            &current,
            true,
            false,
            "",
            LogindDisplayPathChange::Path(current.clone())
        ),
        LogindDisplayChangeAction::Ignore
    );
}

#[test]
fn test_apply_logind_display_change_emits_no_session_when_display_disappears() {
    let current =
        OwnedObjectPath::try_from("/org/freedesktop/login1/session/_1").expect("valid object path");
    assert_eq!(
        apply_logind_display_change(
            &current,
            true,
            true,
            "wayland",
            LogindDisplayPathChange::Empty
        ),
        LogindDisplayChangeAction::EmitNoSessionAndDetach(LifecycleSnapshot {
            active: false,
            session_type: String::new(),
            session_kind: SessionKind::NoSession,
        })
    );
}

#[test]
fn test_apply_logind_display_change_detaches_stale_session_when_already_no_session() {
    let current =
        OwnedObjectPath::try_from("/org/freedesktop/login1/session/_1").expect("valid object path");
    assert_eq!(
        apply_logind_display_change(&current, true, false, "", LogindDisplayPathChange::Empty),
        LogindDisplayChangeAction::DetachSessionMonitor
    );
}

#[test]
fn test_apply_logind_display_change_reattaches_same_path_when_session_monitor_detached() {
    let current =
        OwnedObjectPath::try_from("/org/freedesktop/login1/session/_1").expect("valid object path");
    assert_eq!(
        apply_logind_display_change(
            &current,
            false,
            false,
            "",
            LogindDisplayPathChange::Path(current.clone())
        ),
        LogindDisplayChangeAction::Reattach(current)
    );
}

#[test]
#[should_panic(expected = "mapped-boom")]
fn test_expect_or_fail_fast_uses_fail_handler_for_error_results() {
    let _: u8 = expect_or_fail_fast(
        Err::<u8, _>("boom"),
        |error| format!("mapped-{}", error),
        |message| panic!("{}", message),
    );
}

#[test]
#[should_panic(expected = "stream-ended")]
fn test_expect_some_or_fail_fast_uses_fail_handler_for_none() {
    let _: u8 = expect_some_or_fail_fast(None, "stream-ended".to_string(), |message| {
        panic!("{}", message)
    });
}

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

#[test]
fn test_sni_control_mode_tracks_runtime_environment_changes() {
    let runtime_environment = RuntimeEnvironmentBroadcaster::new(Environment::Unknown);
    assert_eq!(
        sni_control_mode_for_environment(runtime_environment.current()),
        None
    );

    runtime_environment.set_current(Environment::Wayland);
    assert_eq!(
        sni_control_mode_for_environment(runtime_environment.current()),
        Some(SniControlMode::Local)
    );

    runtime_environment.set_current(Environment::X11);
    assert_eq!(
        sni_control_mode_for_environment(runtime_environment.current()),
        Some(SniControlMode::Local)
    );

    runtime_environment.set_current(Environment::Kde);
    assert_eq!(
        sni_control_mode_for_environment(runtime_environment.current()),
        Some(SniControlMode::Dbus)
    );

    runtime_environment.set_current(Environment::Gnome);
    assert_eq!(
        sni_control_mode_for_environment(runtime_environment.current()),
        None
    );
}

#[test]
fn test_plan_sni_runtime_transition_restarts_on_environment_change() {
    assert_eq!(
        plan_sni_runtime_transition(None, None, Environment::Unknown),
        SniRuntimeTransitionPlan::Keep
    );
    assert_eq!(
        plan_sni_runtime_transition(None, None, Environment::Wayland),
        SniRuntimeTransitionPlan::Start(SniControlMode::Local)
    );
    assert_eq!(
        plan_sni_runtime_transition(
            Some(SniControlMode::Local),
            Some(Environment::X11),
            Environment::Wayland
        ),
        SniRuntimeTransitionPlan::Restart(SniControlMode::Local)
    );
    assert_eq!(
        plan_sni_runtime_transition(
            Some(SniControlMode::Local),
            Some(Environment::Wayland),
            Environment::Wayland
        ),
        SniRuntimeTransitionPlan::Keep
    );
    assert_eq!(
        plan_sni_runtime_transition(
            Some(SniControlMode::Local),
            Some(Environment::Wayland),
            Environment::Kde
        ),
        SniRuntimeTransitionPlan::Restart(SniControlMode::Dbus)
    );
    assert_eq!(
        plan_sni_runtime_transition(
            Some(SniControlMode::Dbus),
            Some(Environment::Kde),
            Environment::Gnome
        ),
        SniRuntimeTransitionPlan::Stop
    );
}

#[tokio::test]
async fn test_sni_local_control_quit_triggers_shutdown_handle() {
    with_test_timeout(async {
        let status_broadcaster = StatusBroadcaster::new();
        let shutdown_handle = ShutdownHandle::new();
        let mut shutdown_receiver = shutdown_handle.subscribe();
        assert!(!*shutdown_receiver.borrow());

        let control = SniLocalControl {
            runtime_handle: tokio::runtime::Handle::current(),
            kanata: KanataClient::new("127.0.0.1", 10000, None, true, status_broadcaster.clone()),
            handler: Arc::new(Mutex::new(FocusHandler::new(Vec::new(), None, true))),
            status_broadcaster,
            pause_broadcaster: PauseBroadcaster::new(),
            restart_handle: RestartHandle::new(),
            shutdown_handle,
            unpause_context: local_sni_unpause_context(Environment::Wayland),
        };
        let control = SniControl::Local(control);

        control.quit();

        assert!(
            *shutdown_receiver.borrow_and_update(),
            "SNI Local quit must trigger the daemon shutdown handle"
        );
    })
    .await;
}

#[tokio::test]
async fn test_sni_local_control_unpause_uses_creation_environment_during_transition_race() {
    with_test_timeout(async {
        let status_broadcaster = StatusBroadcaster::new();
        let runtime_environment = RuntimeEnvironmentBroadcaster::new(Environment::Wayland);
        let control = SniLocalControl {
            runtime_handle: tokio::runtime::Handle::current(),
            kanata: KanataClient::new("127.0.0.1", 10000, None, true, status_broadcaster.clone()),
            handler: Arc::new(Mutex::new(FocusHandler::new(Vec::new(), None, true))),
            status_broadcaster,
            pause_broadcaster: PauseBroadcaster::new(),
            restart_handle: RestartHandle::new(),
            shutdown_handle: ShutdownHandle::new(),
            unpause_context: local_sni_unpause_context(Environment::Wayland),
        };
        let control = SniControl::Local(control);

        runtime_environment.set_current(Environment::Kde);
        let _ = take_unpause_request_environment_for_test();
        control.unpause();
        assert_eq!(
            take_unpause_request_environment_for_test(),
            Some(Environment::Wayland),
            "local SNI unpause must use the control's creation environment, not runtime_environment.current()"
        );
    })
    .await;
}

#[tokio::test]
async fn test_sni_runtime_managed_transitions_do_not_leak_watcher_tasks() {
    with_test_timeout(async {
        let _sni_lock = SNI_WATCHER_TEST_LOCK.lock().unwrap();

        async fn assert_sni_watcher_count_eventually(expected: usize, label: &str) {
            for _ in 0..100 {
                if sni_watcher_task_count() == expected {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            panic!(
                "expected {} watcher tasks after {}, got {}",
                expected,
                label,
                sni_watcher_task_count()
            );
        }

        let baseline = sni_watcher_task_count();
        let runtime_environment = RuntimeEnvironmentBroadcaster::new(Environment::Unknown);
        let status_broadcaster = StatusBroadcaster::new();
        let pause_broadcaster = PauseBroadcaster::new();
        let restart_handle = RestartHandle::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            10000,
            Some("default".to_string()),
            true,
            status_broadcaster.clone(),
        );
        let handler = Arc::new(Mutex::new(FocusHandler::new(Vec::new(), None, true)));

        let guard = SniGuard::runtime_managed(
            runtime_environment.clone(),
            tokio::runtime::Handle::current(),
            kanata,
            handler,
            status_broadcaster,
            pause_broadcaster,
            restart_handle,
            ShutdownHandle::new(),
            None,
            effective_dbus_name(&derive_default_dbus_suffix("127.0.0.1", 10000)),
        );

        assert_sni_watcher_count_eventually(baseline, "initial unknown state").await;

        runtime_environment.set_current(Environment::Wayland);
        assert_sni_watcher_count_eventually(baseline + 3, "first indicator start").await;

        runtime_environment.set_current(Environment::X11);
        assert_sni_watcher_count_eventually(baseline + 3, "same-mode restart").await;

        runtime_environment.set_current(Environment::Unknown);
        assert_sni_watcher_count_eventually(baseline, "indicator stop").await;

        runtime_environment.set_current(Environment::Wayland);
        assert_sni_watcher_count_eventually(baseline + 3, "second indicator start").await;

        drop(guard);
        assert_sni_watcher_count_eventually(baseline, "guard drop").await;
    })
    .await;
}

#[tokio::test]
async fn test_sni_runtime_managed_retries_after_transient_start_failure() {
    with_test_timeout(async {
        let _sni_lock = SNI_WATCHER_TEST_LOCK.lock().unwrap();

        async fn assert_sni_watcher_count_eventually(expected: usize, label: &str) {
            for _ in 0..100 {
                if sni_watcher_task_count() == expected {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            panic!(
                "expected {} watcher tasks after {}, got {}",
                expected,
                label,
                sni_watcher_task_count()
            );
        }

        let baseline = sni_watcher_task_count();
        let runtime_environment = RuntimeEnvironmentBroadcaster::new(Environment::Unknown);
        let status_broadcaster = StatusBroadcaster::new();
        let pause_broadcaster = PauseBroadcaster::new();
        let restart_handle = RestartHandle::new();
        let kanata = KanataClient::new(
            "127.0.0.1",
            10000,
            Some("default".to_string()),
            true,
            status_broadcaster.clone(),
        );
        let handler = Arc::new(Mutex::new(FocusHandler::new(Vec::new(), None, true)));
        let build_attempts = Arc::new(AtomicUsize::new(0));
        let build_attempts_for_builder = build_attempts.clone();

        let guard = SniGuard::runtime_managed_with_builder(
            runtime_environment.clone(),
            tokio::runtime::Handle::current(),
            kanata,
            handler,
            status_broadcaster,
            pause_broadcaster,
            restart_handle,
            ShutdownHandle::new(),
            None,
            std::time::Duration::from_millis(20),
            move |mode,
                  runtime_handle,
                  kanata,
                  handler,
                  status_broadcaster,
                  pause_broadcaster,
                  restart_handle,
                  shutdown_handle,
                  control_environment,
                  daemon_bus_name| {
                let build_attempts = build_attempts_for_builder.clone();
                async move {
                    let attempt = build_attempts.fetch_add(1, Ordering::SeqCst);
                    if attempt == 0 {
                        None
                    } else {
                        build_sni_control_for_mode(
                            mode,
                            runtime_handle,
                            kanata,
                            handler,
                            status_broadcaster,
                            pause_broadcaster,
                            restart_handle,
                            shutdown_handle,
                            control_environment,
                            daemon_bus_name,
                        )
                        .await
                    }
                }
            },
            effective_dbus_name(&derive_default_dbus_suffix("127.0.0.1", 10000)),
        );

        runtime_environment.set_current(Environment::Wayland);
        assert_sni_watcher_count_eventually(baseline + 3, "retry-based indicator recovery").await;
        assert!(
            build_attempts.load(Ordering::SeqCst) >= 2,
            "runtime-managed SNI should retry control construction without an environment change"
        );

        drop(guard);
        assert_sni_watcher_count_eventually(baseline, "guard drop").await;
    })
    .await;
}

#[test]
fn test_map_run_outcome_to_backend_exit() {
    assert_eq!(
        map_run_outcome_to_backend_exit(RunOutcome::Restart),
        BackendExit::Restart
    );
    assert_eq!(
        map_run_outcome_to_backend_exit(RunOutcome::Exit),
        BackendExit::Exit
    );
}

#[tokio::test]
async fn test_transition_runtime_target_updates_runtime_environment() {
    with_test_timeout(async {
        let context = test_backend_context();
        let mut state = SupervisorState::new();
        assert_eq!(context.runtime_environment.current(), Environment::Unknown);

        transition_runtime_target_with_starter(
            &mut state,
            RuntimeTarget::Backend(BackendKind::X11),
            &context,
            "to-x11",
            |kind, _| async move {
                Ok(test_running_backend_handle(
                    kind,
                    Arc::new(AtomicBool::new(false)),
                ))
            },
        )
        .await
        .expect("transition to x11 should succeed");
        assert_eq!(context.runtime_environment.current(), Environment::X11);

        transition_runtime_target_with_starter(
            &mut state,
            RuntimeTarget::Backend(BackendKind::LinuxConsole),
            &context,
            "to-linux-console",
            |kind, _| async move {
                Ok(test_running_backend_handle(
                    kind,
                    Arc::new(AtomicBool::new(false)),
                ))
            },
        )
        .await
        .expect("transition to linux console should succeed");
        assert_eq!(
            context.runtime_environment.current(),
            Environment::LinuxConsoleWithLogind
        );

        transition_runtime_target_with_starter(
            &mut state,
            RuntimeTarget::Idle,
            &context,
            "to-idle",
            |kind, _| async move {
                Ok(test_running_backend_handle(
                    kind,
                    Arc::new(AtomicBool::new(false)),
                ))
            },
        )
        .await
        .expect("transition to idle should succeed");
        assert_eq!(context.runtime_environment.current(), Environment::Unknown);
    })
    .await;
}

#[tokio::test]
async fn test_transition_runtime_target_runs_gnome_setup_on_runtime_transition() {
    with_test_timeout(async {
        let setup_calls = Arc::new(AtomicUsize::new(0));
        let setup_calls_for_hook = setup_calls.clone();
        let context = test_backend_context_with_gnome_setup(move |_| {
            setup_calls_for_hook.fetch_add(1, Ordering::SeqCst);
        });
        let mut state = SupervisorState::new();

        let starter = |kind: BackendKind, _context: BackendContext| async move {
            Ok(test_running_backend_handle(
                kind,
                Arc::new(AtomicBool::new(false)),
            ))
        };

        transition_runtime_target_with_starter(
            &mut state,
            RuntimeTarget::Backend(BackendKind::X11),
            &context,
            "to-x11",
            &starter,
        )
        .await
        .expect("transition to x11 should succeed");
        assert_eq!(setup_calls.load(Ordering::SeqCst), 0);

        transition_runtime_target_with_starter(
            &mut state,
            RuntimeTarget::Backend(BackendKind::Gnome),
            &context,
            "to-gnome-runtime-transition",
            &starter,
        )
        .await
        .expect("transition to gnome should succeed");
        assert_eq!(setup_calls.load(Ordering::SeqCst), 1);

        transition_runtime_target_with_starter(
            &mut state,
            RuntimeTarget::Idle,
            &context,
            "to-idle",
            &starter,
        )
        .await
        .expect("transition to idle should succeed");

        transition_runtime_target_with_starter(
            &mut state,
            RuntimeTarget::Backend(BackendKind::Gnome),
            &context,
            "to-gnome-runtime-transition-second-time",
            &starter,
        )
        .await
        .expect("second transition to gnome should succeed");
        assert_eq!(
            setup_calls.load(Ordering::SeqCst),
            1,
            "gnome setup should be cached after first runtime setup"
        );

        stop_current_backend(&mut state, &context)
            .await
            .expect("stopping test backend should succeed");
    })
    .await;
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

#[tokio::test]
async fn test_transition_runtime_target_noop_on_same_target() {
    with_test_timeout(async {
        let context = test_backend_context();

        let mut state = SupervisorState::new();
        transition_runtime_target(&mut state, RuntimeTarget::Idle, &context, "test-noop")
            .await
            .expect("noop transition should succeed");
        assert_eq!(state.current_target, RuntimeTarget::Idle);
        assert!(state.backend.is_none());
    })
    .await;
}

#[tokio::test]
async fn test_transition_runtime_target_no_churn_on_same_target_with_running_backend() {
    with_test_timeout(async {
        let context = test_backend_context();
        let stopped = Arc::new(AtomicBool::new(false));
        let backend = test_running_backend_handle(BackendKind::LinuxConsole, stopped.clone());

        let mut state = SupervisorState::new();
        state.current_target = RuntimeTarget::Backend(BackendKind::LinuxConsole);
        state.backend = Some(backend);

        transition_runtime_target(
            &mut state,
            RuntimeTarget::Backend(BackendKind::LinuxConsole),
            &context,
            "same-target",
        )
        .await
        .expect("same-target transition should be noop");

        assert_eq!(
            state.current_target,
            RuntimeTarget::Backend(BackendKind::LinuxConsole)
        );
        assert!(state.backend.is_some());
        assert!(
            !stopped.load(Ordering::SeqCst),
            "backend should not be stopped on same-target transition"
        );

        stop_current_backend(&mut state, &context)
            .await
            .expect("cleanup stop should succeed");
        assert!(stopped.load(Ordering::SeqCst));
    })
    .await;
}

#[tokio::test]
async fn test_transition_runtime_target_stops_old_before_starting_new() {
    with_test_timeout(async {
        let context = test_backend_context();
        let old_stopped = Arc::new(AtomicBool::new(false));
        let old_backend = test_running_backend_handle(BackendKind::X11, old_stopped.clone());

        let mut state = SupervisorState::new();
        state.current_target = RuntimeTarget::Backend(BackendKind::X11);
        state.backend = Some(old_backend);

        let started = Arc::new(AtomicBool::new(false));
        let started_clone = started.clone();
        let old_stopped_clone = old_stopped.clone();
        transition_runtime_target_with_starter(
            &mut state,
            RuntimeTarget::Backend(BackendKind::LinuxConsole),
            &context,
            "ordering-test",
            move |kind, _| {
                let started = started_clone.clone();
                let old_stopped = old_stopped_clone.clone();
                async move {
                    assert!(
                        old_stopped.load(Ordering::SeqCst),
                        "starter must run only after previous backend is fully stopped"
                    );
                    started.store(true, Ordering::SeqCst);
                    Ok(test_running_backend_handle(
                        kind,
                        Arc::new(AtomicBool::new(false)),
                    ))
                }
            },
        )
        .await
        .expect("transition should succeed");

        assert!(old_stopped.load(Ordering::SeqCst));
        assert!(started.load(Ordering::SeqCst));
        assert_eq!(
            state.current_target,
            RuntimeTarget::Backend(BackendKind::LinuxConsole)
        );
        assert!(state.backend.is_some());

        stop_current_backend(&mut state, &context)
            .await
            .expect("cleanup stop should succeed");
    })
    .await;
}

#[tokio::test]
async fn test_transition_runtime_target_desktop_sequence_is_restart_equivalent() {
    with_test_timeout(async {
        let context = test_backend_context();
        let mut state = SupervisorState::new();
        let started_kinds = Arc::new(Mutex::new(Vec::<BackendKind>::new()));
        let stopped_kinds = Arc::new(Mutex::new(Vec::<BackendKind>::new()));

        let starter = {
            let started_kinds = started_kinds.clone();
            let stopped_kinds = stopped_kinds.clone();
            move |kind: BackendKind, _context: BackendContext| {
                let started_kinds = started_kinds.clone();
                let stopped_kinds = stopped_kinds.clone();
                async move {
                    started_kinds.lock().unwrap().push(kind);
                    let shutdown_handle = ShutdownHandle::new();
                    let mut receiver = shutdown_handle.subscribe();
                    let (finished_tx, finished_rx) = watch::channel(false);
                    let join_handle = tokio::spawn(async move {
                        while !*receiver.borrow() {
                            if receiver.changed().await.is_err() {
                                break;
                            }
                        }
                        stopped_kinds.lock().unwrap().push(kind);
                        let _ = finished_tx.send(true);
                        Ok(BackendExit::Exit)
                    });
                    Ok(BackendHandle {
                        kind,
                        shutdown_handle,
                        join_handle: Some(join_handle),
                        finished_rx,
                    })
                }
            }
        };

        transition_runtime_target_with_starter(
            &mut state,
            RuntimeTarget::Backend(BackendKind::X11),
            &context,
            "x11",
            &starter,
        )
        .await
        .expect("x11 transition should succeed");
        assert_eq!(context.runtime_environment.current(), Environment::X11);

        transition_runtime_target_with_starter(
            &mut state,
            RuntimeTarget::Backend(BackendKind::Wayland),
            &context,
            "wayland",
            &starter,
        )
        .await
        .expect("wayland transition should succeed");
        assert_eq!(context.runtime_environment.current(), Environment::Wayland);

        transition_runtime_target_with_starter(
            &mut state,
            RuntimeTarget::Backend(BackendKind::Kde),
            &context,
            "kde",
            &starter,
        )
        .await
        .expect("kde transition should succeed");
        assert_eq!(context.runtime_environment.current(), Environment::Kde);

        transition_runtime_target_with_starter(
            &mut state,
            RuntimeTarget::Idle,
            &context,
            "idle",
            &starter,
        )
        .await
        .expect("idle transition should succeed");
        assert_eq!(context.runtime_environment.current(), Environment::Unknown);
        assert!(state.backend.is_none());

        assert_eq!(
            started_kinds.lock().unwrap().as_slice(),
            &[BackendKind::X11, BackendKind::Wayland, BackendKind::Kde,]
        );
        assert_eq!(
            stopped_kinds.lock().unwrap().as_slice(),
            &[BackendKind::X11, BackendKind::Wayland, BackendKind::Kde,]
        );
    })
    .await;
}

#[tokio::test]
async fn test_stop_current_backend_with_no_backend_is_noop() {
    with_test_timeout(async {
        let context = test_backend_context();
        let mut state = SupervisorState::new();
        state.current_target = RuntimeTarget::Backend(BackendKind::LinuxConsole);

        stop_current_backend(&mut state, &context)
            .await
            .expect("stop with no backend should succeed");

        assert_eq!(state.current_target, RuntimeTarget::Idle);
        assert!(state.backend.is_none());
    })
    .await;
}

#[tokio::test]
async fn test_run_lifecycle_supervisor_restart_after_startup_provider_exhausted() {
    with_test_timeout(async {
        let provider =
            LifecycleProvider::Startup(StartupSnapshotProvider::new(Environment::Unknown));
        let context = test_backend_context();
        let restart_handle = RestartHandle::new();
        let shutdown_handle = ShutdownHandle::new();

        let supervisor = tokio::spawn(run_lifecycle_supervisor(
            provider,
            context,
            restart_handle.clone(),
            shutdown_handle,
        ));

        tokio::task::yield_now().await;
        restart_handle.request();

        let outcome = supervisor
            .await
            .expect("supervisor task join")
            .expect("supervisor should return outcome");
        assert_eq!(outcome, RunOutcome::Restart);
    })
    .await;
}

#[tokio::test]
async fn test_run_lifecycle_supervisor_shutdown_wins_when_both_pre_set() {
    with_test_timeout(async {
        let provider =
            LifecycleProvider::Startup(StartupSnapshotProvider::new(Environment::Unknown));
        let context = test_backend_context();
        let restart_handle = RestartHandle::new();
        let shutdown_handle = ShutdownHandle::new();
        shutdown_handle.request();
        restart_handle.request();

        let outcome = run_lifecycle_supervisor(provider, context, restart_handle, shutdown_handle)
            .await
            .expect("supervisor should return outcome");
        assert_eq!(outcome, RunOutcome::Exit);
    })
    .await;
}

#[tokio::test]
async fn test_run_lifecycle_supervisor_shutdown_after_startup_provider_exhausted() {
    with_test_timeout(async {
        let provider =
            LifecycleProvider::Startup(StartupSnapshotProvider::new(Environment::Unknown));
        let context = test_backend_context();
        let restart_handle = RestartHandle::new();
        let shutdown_handle = ShutdownHandle::new();

        let supervisor = tokio::spawn(run_lifecycle_supervisor(
            provider,
            context,
            restart_handle,
            shutdown_handle.clone(),
        ));

        tokio::task::yield_now().await;
        shutdown_handle.request();

        let outcome = supervisor
            .await
            .expect("supervisor task join")
            .expect("supervisor should return outcome");
        assert_eq!(outcome, RunOutcome::Exit);
    })
    .await;
}

#[tokio::test]
async fn test_run_lifecycle_supervisor_handles_shutdown_before_first_logind_snapshot() {
    with_test_timeout(async {
        let context = test_backend_context();
        let restart_handle = RestartHandle::new();
        let shutdown_handle = ShutdownHandle::new();
        let (_sender, receiver) = mpsc::unbounded_channel();
        let provider = LifecycleProvider::Logind(LogindLifecycleProvider { receiver });

        shutdown_handle.request();
        let outcome =
            run_lifecycle_supervisor(provider, context, restart_handle, shutdown_handle).await;
        assert_eq!(outcome.unwrap(), RunOutcome::Exit);
    })
    .await;
}

#[tokio::test]
async fn test_poll_finished_backend_outcome_returns_restart() {
    with_test_timeout(async {
        let mut state = SupervisorState::new();
        state.current_target = RuntimeTarget::Backend(BackendKind::LinuxConsole);
        state.backend = Some(test_finished_backend_handle(
            BackendKind::LinuxConsole,
            BackendExit::Restart,
        ));
        for _ in 0..10 {
            if state
                .backend
                .as_ref()
                .expect("backend should be set")
                .is_finished()
            {
                break;
            }
            tokio::task::yield_now().await;
        }

        let outcome = poll_finished_backend_outcome(&mut state)
            .await
            .expect("finished backend poll should succeed");
        assert_eq!(outcome, Some(RunOutcome::Restart));
        assert!(state.backend.is_none());
        assert_eq!(state.current_target, RuntimeTarget::Idle);
    })
    .await;
}

#[tokio::test]
async fn test_poll_finished_backend_outcome_errors_on_unexpected_exit() {
    with_test_timeout(async {
        let mut state = SupervisorState::new();
        state.current_target = RuntimeTarget::Backend(BackendKind::X11);
        state.backend = Some(test_finished_backend_handle(
            BackendKind::X11,
            BackendExit::Exit,
        ));
        for _ in 0..10 {
            if state
                .backend
                .as_ref()
                .expect("backend should be set")
                .is_finished()
            {
                break;
            }
            tokio::task::yield_now().await;
        }

        let result = poll_finished_backend_outcome(&mut state).await;
        assert!(
            result.is_err(),
            "unexpected backend exit should be an error"
        );
        let message = result.err().expect("error expected").to_string();
        assert!(
            message.contains("exited unexpectedly"),
            "unexpected exit error should mention regression context"
        );
        assert!(state.backend.is_none());
        assert_eq!(state.current_target, RuntimeTarget::Idle);
    })
    .await;
}

#[tokio::test]
async fn test_run_lifecycle_supervisor_shutdown_wins_race_while_backend_running() {
    with_test_timeout(async {
        let (sender, receiver) = mpsc::unbounded_channel();
        sender
            .send(LifecycleSnapshot {
                active: true,
                session_type: "tty".to_string(),
                session_kind: SessionKind::NativeTerminal,
            })
            .expect("snapshot send should succeed");
        let provider = LifecycleProvider::Logind(LogindLifecycleProvider { receiver });
        let context = test_backend_context();
        let restart_handle = RestartHandle::new();
        let shutdown_handle = ShutdownHandle::new();

        let supervisor = tokio::spawn(run_lifecycle_supervisor(
            provider,
            context,
            restart_handle.clone(),
            shutdown_handle.clone(),
        ));

        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        restart_handle.request();
        shutdown_handle.request();

        let outcome = supervisor
            .await
            .expect("supervisor task join")
            .expect("supervisor should return outcome");
        assert_eq!(outcome, RunOutcome::Exit);
    })
    .await;
}

#[tokio::test]
async fn test_run_lifecycle_supervisor_wakes_on_backend_completion_after_provider_exhausts() {
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
        let provider = LifecycleProvider::Logind(LogindLifecycleProvider { receiver });
        let context = test_backend_context();
        let restart_handle = RestartHandle::new();
        let shutdown_handle = ShutdownHandle::new();

        let outcome = run_lifecycle_supervisor_with_starter(
            provider,
            context,
            restart_handle,
            shutdown_handle,
            |kind, _context| async move {
                let shutdown_handle = ShutdownHandle::new();
                let (finished_tx, finished_rx) = watch::channel(false);
                let join_handle = tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    let _ = finished_tx.send(true);
                    Ok(BackendExit::Restart)
                });

                Ok(BackendHandle {
                    kind,
                    shutdown_handle,
                    join_handle: Some(join_handle),
                    finished_rx,
                })
            },
        )
        .await
        .expect("supervisor should observe backend completion");

        assert_eq!(outcome, RunOutcome::Restart);
    })
    .await;
}

#[tokio::test]
async fn test_run_lifecycle_supervisor_rechecks_wayland_capabilities_without_new_snapshots() {
    with_test_timeout(async {
        let (sender, receiver) = mpsc::unbounded_channel();
        sender
            .send(LifecycleSnapshot {
                active: true,
                session_type: "wayland".to_string(),
                session_kind: SessionKind::GraphicalWayland,
            })
            .expect("snapshot send should succeed");
        drop(sender);
        let provider = LifecycleProvider::Logind(LogindLifecycleProvider { receiver });
        let context = test_backend_context();
        let restart_handle = RestartHandle::new();
        let shutdown_handle = ShutdownHandle::new();

        let resolver_calls = Arc::new(AtomicUsize::new(0));
        let resolver_calls_clone = resolver_calls.clone();
        let started_kinds = Arc::new(Mutex::new(Vec::<BackendKind>::new()));
        let started_kinds_clone = started_kinds.clone();

        let supervisor = tokio::spawn(run_lifecycle_supervisor_with_starter_and_resolver(
            provider,
            context,
            restart_handle,
            shutdown_handle.clone(),
            move |kind, _| {
                let started_kinds = started_kinds_clone.clone();
                async move {
                    started_kinds.lock().unwrap().push(kind);
                    Ok(test_running_backend_handle(
                        kind,
                        Arc::new(AtomicBool::new(false)),
                    ))
                }
            },
            move |_snapshot| {
                let resolver_calls = resolver_calls_clone.clone();
                async move {
                    let call_index = resolver_calls.fetch_add(1, Ordering::SeqCst);
                    if call_index == 0 {
                        Ok(RuntimeTarget::Backend(BackendKind::Wayland))
                    } else {
                        Ok(RuntimeTarget::Backend(BackendKind::Gnome))
                    }
                }
            },
            std::time::Duration::from_millis(20),
        ));

        tokio::time::sleep(std::time::Duration::from_millis(90)).await;
        shutdown_handle.request();

        let outcome = supervisor
            .await
            .expect("supervisor task join")
            .expect("supervisor should return outcome");
        assert_eq!(outcome, RunOutcome::Exit);

        let kinds = started_kinds.lock().unwrap().clone();
        assert_eq!(kinds.first(), Some(&BackendKind::Wayland));
        assert!(
            kinds.contains(&BackendKind::Gnome),
            "capability recheck should promote from generic wayland to gnome"
        );
        assert!(
            resolver_calls.load(Ordering::SeqCst) >= 2,
            "resolver should be called again without new lifecycle snapshots"
        );
    })
    .await;
}

#[tokio::test]
async fn test_run_lifecycle_supervisor_startup_mode_skips_wayland_capability_rechecks() {
    with_test_timeout(async {
        let provider =
            LifecycleProvider::Startup(StartupSnapshotProvider::new(Environment::Wayland));
        let context = test_backend_context();
        let restart_handle = RestartHandle::new();
        let shutdown_handle = ShutdownHandle::new();
        let resolver_calls = Arc::new(AtomicUsize::new(0));
        let resolver_calls_clone = resolver_calls.clone();
        let started_kinds = Arc::new(Mutex::new(Vec::<BackendKind>::new()));
        let started_kinds_clone = started_kinds.clone();

        let supervisor = tokio::spawn(run_lifecycle_supervisor_with_starter_and_resolver(
            provider,
            context,
            restart_handle,
            shutdown_handle.clone(),
            move |kind, _| {
                let started_kinds = started_kinds_clone.clone();
                async move {
                    started_kinds.lock().unwrap().push(kind);
                    Ok(test_running_backend_handle(
                        kind,
                        Arc::new(AtomicBool::new(false)),
                    ))
                }
            },
            move |_snapshot| {
                let resolver_calls = resolver_calls_clone.clone();
                async move {
                    let call_index = resolver_calls.fetch_add(1, Ordering::SeqCst);
                    if call_index == 0 {
                        Ok(RuntimeTarget::Backend(BackendKind::Wayland))
                    } else {
                        Ok(RuntimeTarget::Backend(BackendKind::Gnome))
                    }
                }
            },
            std::time::Duration::from_millis(20),
        ));

        tokio::time::sleep(std::time::Duration::from_millis(90)).await;
        shutdown_handle.request();

        let outcome = supervisor
            .await
            .expect("supervisor task join")
            .expect("supervisor should return outcome");
        assert_eq!(outcome, RunOutcome::Exit);
        assert_eq!(resolver_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            started_kinds.lock().unwrap().as_slice(),
            &[BackendKind::Wayland]
        );
    })
    .await;
}

#[tokio::test]
async fn test_run_lifecycle_supervisor_startup_mode_selects_gnome_without_focus_readiness_gate() {
    with_test_timeout(async {
        let provider =
            LifecycleProvider::Startup(StartupSnapshotProvider::new(Environment::Wayland));
        let setup_calls = Arc::new(AtomicUsize::new(0));
        let setup_calls_for_hook = setup_calls.clone();
        let context = test_backend_context_with_gnome_setup(move |_| {
            setup_calls_for_hook.fetch_add(1, Ordering::SeqCst);
        });
        let restart_handle = RestartHandle::new();
        let shutdown_handle = ShutdownHandle::new();
        let started_kinds = Arc::new(Mutex::new(Vec::<BackendKind>::new()));
        let started_kinds_clone = started_kinds.clone();

        let supervisor = tokio::spawn(run_lifecycle_supervisor_with_starter_and_resolver(
            provider,
            context,
            restart_handle,
            shutdown_handle.clone(),
            move |kind, _| {
                let started_kinds = started_kinds_clone.clone();
                async move {
                    started_kinds.lock().unwrap().push(kind);
                    Ok(test_running_backend_handle(
                        kind,
                        Arc::new(AtomicBool::new(false)),
                    ))
                }
            },
            move |_snapshot| async move {
                Ok(resolve_runtime_target(
                    SessionKind::GraphicalWayland,
                    DesktopCapabilities {
                        gnome_owner: true,
                        kde_owner: false,
                    },
                ))
            },
            std::time::Duration::from_millis(20),
        ));

        tokio::time::sleep(std::time::Duration::from_millis(60)).await;
        shutdown_handle.request();

        let outcome = supervisor
            .await
            .expect("supervisor task join")
            .expect("supervisor should return outcome");
        assert_eq!(outcome, RunOutcome::Exit);
        assert_eq!(
            started_kinds.lock().unwrap().as_slice(),
            &[BackendKind::Gnome]
        );
        assert_eq!(
            setup_calls.load(Ordering::SeqCst),
            1,
            "startup-only mode must run GNOME setup before starting GNOME backend"
        );
    })
    .await;
}

#[tokio::test]
async fn test_run_lifecycle_supervisor_startup_mode_falls_back_to_generic_wayland_on_initial_resolver_error()
 {
    with_test_timeout(async {
        let provider =
            LifecycleProvider::Startup(StartupSnapshotProvider::new(Environment::Wayland));
        let context = test_backend_context();
        let restart_handle = RestartHandle::new();
        let shutdown_handle = ShutdownHandle::new();
        let resolver_calls = Arc::new(AtomicUsize::new(0));
        let resolver_calls_clone = resolver_calls.clone();
        let started_kinds = Arc::new(Mutex::new(Vec::<BackendKind>::new()));
        let started_kinds_clone = started_kinds.clone();

        let supervisor = tokio::spawn(run_lifecycle_supervisor_with_starter_and_resolver(
            provider,
            context,
            restart_handle,
            shutdown_handle.clone(),
            move |kind, _| {
                let started_kinds = started_kinds_clone.clone();
                async move {
                    started_kinds.lock().unwrap().push(kind);
                    Ok(test_running_backend_handle(
                        kind,
                        Arc::new(AtomicBool::new(false)),
                    ))
                }
            },
            move |_snapshot| {
                let resolver_calls = resolver_calls_clone.clone();
                async move {
                    resolver_calls.fetch_add(1, Ordering::SeqCst);
                    Err(std::io::Error::other("transient startup wayland resolver failure").into())
                }
            },
            std::time::Duration::from_millis(20),
        ));

        tokio::time::sleep(std::time::Duration::from_millis(60)).await;
        shutdown_handle.request();

        let outcome = supervisor
            .await
            .expect("supervisor task join")
            .expect("startup wayland resolver errors should not terminate startup provider");
        assert_eq!(outcome, RunOutcome::Exit);
        assert_eq!(
            started_kinds.lock().unwrap().as_slice(),
            &[BackendKind::Wayland],
            "startup mode should fall back to generic wayland on transient wayland resolver errors"
        );
        assert_eq!(
            resolver_calls.load(Ordering::SeqCst),
            1,
            "startup provider should still be one-shot after fallback"
        );
    })
    .await;
}

#[tokio::test]
async fn test_run_lifecycle_supervisor_startup_mode_non_wayland_resolver_error_is_fatal() {
    with_test_timeout(async {
        let provider = LifecycleProvider::Startup(StartupSnapshotProvider::new(Environment::X11));
        let context = test_backend_context();
        let restart_handle = RestartHandle::new();
        let shutdown_handle = ShutdownHandle::new();
        let started_kinds = Arc::new(Mutex::new(Vec::<BackendKind>::new()));
        let started_kinds_clone = started_kinds.clone();

        let result = run_lifecycle_supervisor_with_starter_and_resolver(
            provider,
            context,
            restart_handle,
            shutdown_handle,
            move |kind, _| {
                let started_kinds = started_kinds_clone.clone();
                async move {
                    started_kinds.lock().unwrap().push(kind);
                    Ok(test_running_backend_handle(
                        kind,
                        Arc::new(AtomicBool::new(false)),
                    ))
                }
            },
            move |_snapshot| async move {
                Err(std::io::Error::other("startup resolver failure").into())
            },
            std::time::Duration::from_millis(20),
        )
        .await;

        assert!(
            result.is_err(),
            "non-wayland startup resolver failures should still fail fast"
        );
        let error = result.err().expect("error expected").to_string();
        assert!(
            error.contains("Startup lifecycle target resolution failed"),
            "error should explain startup-only resolution failure"
        );
        assert!(
            started_kinds.lock().unwrap().is_empty(),
            "no backend should start when startup resolver fails"
        );
    })
    .await;
}

#[tokio::test]
async fn test_run_lifecycle_supervisor_recovers_after_transient_wayland_resolver_error() {
    with_test_timeout(async {
        let (sender, receiver) = mpsc::unbounded_channel();
        sender
            .send(LifecycleSnapshot {
                active: true,
                session_type: "wayland".to_string(),
                session_kind: SessionKind::GraphicalWayland,
            })
            .expect("snapshot send should succeed");
        drop(sender);
        let provider = LifecycleProvider::Logind(LogindLifecycleProvider { receiver });
        let context = test_backend_context();
        let restart_handle = RestartHandle::new();
        let shutdown_handle = ShutdownHandle::new();
        let resolver_calls = Arc::new(AtomicUsize::new(0));
        let resolver_calls_clone = resolver_calls.clone();
        let started_kinds = Arc::new(Mutex::new(Vec::<BackendKind>::new()));
        let started_kinds_clone = started_kinds.clone();

        let supervisor = tokio::spawn(run_lifecycle_supervisor_with_starter_and_resolver(
            provider,
            context,
            restart_handle,
            shutdown_handle.clone(),
            move |kind, _| {
                let started_kinds = started_kinds_clone.clone();
                async move {
                    started_kinds.lock().unwrap().push(kind);
                    Ok(test_running_backend_handle(
                        kind,
                        Arc::new(AtomicBool::new(false)),
                    ))
                }
            },
            move |_snapshot| {
                let resolver_calls = resolver_calls_clone.clone();
                async move {
                    let call_index = resolver_calls.fetch_add(1, Ordering::SeqCst);
                    if call_index == 0 {
                        Ok(RuntimeTarget::Backend(BackendKind::Wayland))
                    } else if call_index == 1 {
                        Err(std::io::Error::other("transient capability probe failure").into())
                    } else {
                        Ok(RuntimeTarget::Backend(BackendKind::Gnome))
                    }
                }
            },
            std::time::Duration::from_millis(20),
        ));

        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        shutdown_handle.request();

        let outcome = supervisor
            .await
            .expect("supervisor task join")
            .expect("supervisor should remain alive after transient resolver error");
        assert_eq!(outcome, RunOutcome::Exit);

        let kinds = started_kinds.lock().unwrap().clone();
        assert_eq!(kinds.first(), Some(&BackendKind::Wayland));
        assert!(
            kinds.contains(&BackendKind::Gnome),
            "supervisor should recover and transition once resolver succeeds again"
        );
        assert!(
            resolver_calls.load(Ordering::SeqCst) >= 3,
            "resolver should continue running after transient failures"
        );
    })
    .await;
}

#[tokio::test]
async fn test_run_lifecycle_supervisor_continuous_wayland_resolver_error_falls_back_to_generic_wayland()
 {
    with_test_timeout(async {
        let (sender, receiver) = mpsc::unbounded_channel();
        sender
            .send(LifecycleSnapshot {
                active: true,
                session_type: "wayland".to_string(),
                session_kind: SessionKind::GraphicalWayland,
            })
            .expect("snapshot send should succeed");
        drop(sender);

        let provider = LifecycleProvider::Logind(LogindLifecycleProvider { receiver });
        let context = test_backend_context();
        let restart_handle = RestartHandle::new();
        let shutdown_handle = ShutdownHandle::new();
        let started_kinds = Arc::new(Mutex::new(Vec::<BackendKind>::new()));
        let started_kinds_clone = started_kinds.clone();
        let resolver_calls = Arc::new(AtomicUsize::new(0));
        let resolver_calls_clone = resolver_calls.clone();

        let supervisor = tokio::spawn(run_lifecycle_supervisor_with_starter_and_resolver(
            provider,
            context,
            restart_handle,
            shutdown_handle.clone(),
            move |kind, _| {
                let started_kinds = started_kinds_clone.clone();
                async move {
                    started_kinds.lock().unwrap().push(kind);
                    Ok(test_running_backend_handle(
                        kind,
                        Arc::new(AtomicBool::new(false)),
                    ))
                }
            },
            move |_snapshot| {
                let resolver_calls = resolver_calls_clone.clone();
                async move {
                    resolver_calls.fetch_add(1, Ordering::SeqCst);
                    Err(std::io::Error::other("continuous wayland resolver failure").into())
                }
            },
            std::time::Duration::from_secs(5),
        ));

        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        shutdown_handle.request();

        let outcome = supervisor
            .await
            .expect("supervisor task join")
            .expect("continuous resolver failure should still fall back to wayland");
        assert_eq!(outcome, RunOutcome::Exit);
        assert_eq!(
            started_kinds.lock().unwrap().as_slice(),
            &[BackendKind::Wayland],
            "continuous wayland resolver failure should trigger generic wayland fallback immediately"
        );
        assert_eq!(
            resolver_calls.load(Ordering::SeqCst),
            1,
            "fallback should happen on snapshot resolver error without waiting for periodic recheck"
        );
    })
    .await;
}

#[tokio::test]
async fn test_run_lifecycle_supervisor_continuous_wayland_resolver_error_keeps_active_gnome_backend()
 {
    with_test_timeout(async {
        let (sender, receiver) = mpsc::unbounded_channel();
        sender
            .send(LifecycleSnapshot {
                active: true,
                session_type: "wayland".to_string(),
                session_kind: SessionKind::GraphicalWayland,
            })
            .expect("snapshot send should succeed");
        drop(sender);

        let provider = LifecycleProvider::Logind(LogindLifecycleProvider { receiver });
        let context = test_backend_context();
        let restart_handle = RestartHandle::new();
        let shutdown_handle = ShutdownHandle::new();
        let started_kinds = Arc::new(Mutex::new(Vec::<BackendKind>::new()));
        let started_kinds_clone = started_kinds.clone();
        let resolver_calls = Arc::new(AtomicUsize::new(0));
        let resolver_calls_clone = resolver_calls.clone();

        let supervisor = tokio::spawn(run_lifecycle_supervisor_with_starter_and_resolver(
            provider,
            context,
            restart_handle,
            shutdown_handle.clone(),
            move |kind, _| {
                let started_kinds = started_kinds_clone.clone();
                async move {
                    started_kinds.lock().unwrap().push(kind);
                    Ok(test_running_backend_handle(
                        kind,
                        Arc::new(AtomicBool::new(false)),
                    ))
                }
            },
            move |_snapshot| {
                let resolver_calls = resolver_calls_clone.clone();
                async move {
                    let call_index = resolver_calls.fetch_add(1, Ordering::SeqCst);
                    if call_index == 0 {
                        Ok(RuntimeTarget::Backend(BackendKind::Gnome))
                    } else {
                        Err(std::io::Error::other("transient wayland capability probe failure").into())
                    }
                }
            },
            std::time::Duration::from_millis(20),
        ));

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        shutdown_handle.request();

        let outcome = supervisor
            .await
            .expect("supervisor task join")
            .expect("transient resolver failures should not tear down active GNOME backend");
        assert_eq!(outcome, RunOutcome::Exit);

        let kinds = started_kinds.lock().unwrap().clone();
        assert_eq!(
            kinds.as_slice(),
            &[BackendKind::Gnome],
            "continuous resolver errors should keep active wayland-family backend instead of downgrading to generic wayland"
        );
        assert!(
            resolver_calls.load(Ordering::SeqCst) >= 2,
            "resolver should keep retrying after transient failures"
        );
    })
    .await;
}

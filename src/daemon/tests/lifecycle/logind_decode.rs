use super::*;

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

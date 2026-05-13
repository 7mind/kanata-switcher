use super::*;

// === GNOME Extension State Parsing Tests ===

#[test]
fn test_gnome_extension_state_enabled_f64() {
    // GNOME Shell returns state as f64, 1.0 = ENABLED
    use zbus::zvariant::{OwnedValue, Value};

    let mut body = HashMap::new();
    body.insert(
        "state".to_string(),
        OwnedValue::try_from(Value::F64(1.0)).unwrap(),
    );

    let status = parse_gnome_extension_state(&body);
    assert!(status.active, "state=1.0 should be active");
    assert!(status.enabled, "state=1.0 should be enabled");
    assert!(status.installed, "D-Bus response means installed");
}

#[test]
fn test_gnome_extension_state_disabled_f64() {
    // State 2.0 = DISABLED
    use zbus::zvariant::{OwnedValue, Value};

    let mut body = HashMap::new();
    body.insert(
        "state".to_string(),
        OwnedValue::try_from(Value::F64(2.0)).unwrap(),
    );

    let status = parse_gnome_extension_state(&body);
    assert!(!status.active, "state=2.0 should not be active");
    assert!(!status.enabled, "state=2.0 should not be enabled");
}

#[test]
fn test_gnome_extension_state_missing() {
    // No state field - should default to not enabled
    let body = HashMap::new();

    let status = parse_gnome_extension_state(&body);
    assert!(!status.active, "missing state should not be active");
    assert!(!status.enabled, "missing state should not be enabled");
}

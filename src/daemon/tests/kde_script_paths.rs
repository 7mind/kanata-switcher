use super::*;

#[test]
fn test_kwin_query_script_path_scopes_to_uuid_and_query_id() {
    let path = kwin_query_script_path(7);
    assert!(path.starts_with("/tmp/kanata-switcher-kwin-query-"));
    let tail = path
        .strip_prefix("/tmp/kanata-switcher-kwin-query-")
        .and_then(|tail| tail.strip_suffix(".js"))
        .expect("failed to parse query script tail");
    let mut parts = tail.splitn(3, '-');
    let _uid = parts.next().expect("missing uid");
    let query_id = parts.next().expect("missing query id");
    let request_id = parts.next().expect("missing query request id");
    assert_eq!(query_id, "7");
    assert!(Uuid::parse_str(request_id).is_ok());
    assert_ne!(kwin_query_script_path(7), kwin_query_script_path(8));
}

#[test]
fn test_kwin_query_probe_script_path_scopes_to_uuid_and_probe_id() {
    let path = kwin_query_probe_script_path(11);
    assert!(path.starts_with("/tmp/kanata-switcher-kwin-query-probe-"));
    let tail = path
        .strip_prefix("/tmp/kanata-switcher-kwin-query-probe-")
        .and_then(|tail| tail.strip_suffix(".js"))
        .expect("failed to parse probe script tail");
    let mut parts = tail.splitn(3, '-');
    let _uid = parts.next().expect("missing uid");
    let probe_id = parts.next().expect("missing probe id");
    let request_id = parts.next().expect("missing probe request id");
    assert_eq!(probe_id, "11");
    assert!(Uuid::parse_str(request_id).is_ok());
    assert_ne!(
        kwin_query_probe_script_path(11),
        kwin_query_probe_script_path(12)
    );
}

#[test]
fn test_kwin_runtime_script_path_scopes_to_uuid() {
    let path = kwin_runtime_script_path();
    assert!(path.starts_with("/tmp/kanata-switcher-kwin-"));
    assert!(path.ends_with(".js"));
    let request_id = path
        .strip_prefix("/tmp/kanata-switcher-kwin-")
        .and_then(|tail| tail.strip_suffix(".js"))
        .and_then(|tail| tail.splitn(2, '-').nth(1))
        .expect("failed to parse runtime uuid");
    assert!(Uuid::parse_str(request_id).is_ok());
}

#[test]
fn test_build_kde_focus_push_script_targets_instance_name() {
    let script = build_kde_focus_push_script(
        "com.github.kanata.Switcher.instances.kinesis",
        "windowActivated",
        "activeWindow",
    );
    assert!(
        script.contains("\"com.github.kanata.Switcher.instances.kinesis\""),
        "KWin script should reference per-instance bus name; got:\n{}",
        script
    );
    assert!(script.contains("\"com.github.kanata.Switcher\""));
    assert!(script.contains("WindowFocus"));
    assert!(script.contains("workspace.windowActivated"));
    assert!(script.contains("workspace.activeWindow"));
}

use super::*;

#[test]
fn test_autostart_passthrough_args_skip_oneshot() {
    let matches = Args::command().get_matches_from([
        "kanata-switcher",
        "--install-autostart",
        "-p",
        "12000",
        "--quiet-focus",
        "--no-indicator",
    ]);
    let args = Args::from_arg_matches(&matches).unwrap();
    let exec_args = autostart_passthrough_args(&matches, &args);
    assert_eq!(
        exec_args,
        vec![
            "-p".to_string(),
            "12000".to_string(),
            "--quiet-focus".to_string(),
            "--no-indicator".to_string()
        ]
    );
}

#[test]
fn test_autostart_passthrough_args_indicator_focus_only() {
    let matches = Args::command().get_matches_from([
        "kanata-switcher",
        "--install-autostart",
        "--indicator-focus-only",
        "false",
    ]);
    let args = Args::from_arg_matches(&matches).unwrap();
    let exec_args = autostart_passthrough_args(&matches, &args);
    assert_eq!(
        exec_args,
        vec!["--indicator-focus-only".to_string(), "false".to_string()]
    );
}

#[test]
fn test_autostart_desktop_content_escapes_exec() {
    let exec_path = Path::new("/tmp/kanata switcher");
    let exec_args = vec![
        "--quiet-focus".to_string(),
        "-c".to_string(),
        "/tmp/config%file.json".to_string(),
    ];
    let content = build_autostart_desktop_content(exec_path, &exec_args);
    assert!(content.contains("Type=Application\n"));
    assert!(content.contains("Name=Kanata Switcher\n"));
    assert!(content.contains("X-GNOME-Autostart-enabled=true\n"));
    assert!(content.contains(
        "Exec=\"/tmp/kanata switcher\" \"--quiet-focus\" \"-c\" \"/tmp/config%%file.json\"\n"
    ));
    assert!(content.contains("TryExec=\"/tmp/kanata switcher\"\n"));
}

use super::*;

#[test]
fn test_control_command_restart() {
    let args = Args::parse_from(["kanata-switcher", "--restart"]);
    assert_eq!(
        resolve_control_command(&args),
        Some(ControlCommand::Restart)
    );
}

#[test]
fn test_control_command_pause() {
    let args = Args::parse_from(["kanata-switcher", "--pause"]);
    assert_eq!(resolve_control_command(&args), Some(ControlCommand::Pause));
}

#[test]
fn test_control_command_unpause() {
    let args = Args::parse_from(["kanata-switcher", "--unpause"]);
    assert_eq!(
        resolve_control_command(&args),
        Some(ControlCommand::Unpause)
    );
}

#[test]
fn test_control_command_none() {
    let args = Args::parse_from(["kanata-switcher"]);
    assert_eq!(resolve_control_command(&args), None);
}

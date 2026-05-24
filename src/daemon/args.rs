use clap::{ArgMatches, Parser, ValueEnum};
use std::path::PathBuf;
use crate::dbus_naming::sanitize_dbus_suffix;
use crate::control::ControlCommand;

// === CLI ===

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum TrayFocusOnly {
    True,
    False,
}

impl TrayFocusOnly {
    pub(crate) fn as_bool(self) -> bool {
        matches!(self, TrayFocusOnly::True)
    }

    pub(crate) fn as_arg(self) -> &'static str {
        match self {
            TrayFocusOnly::True => "true",
            TrayFocusOnly::False => "false",
        }
    }
}

#[derive(Parser)]
#[command(name = "kanata-switcher")]
#[command(about = "Switch kanata layers based on focused window")]
pub(crate) struct Args {
    #[arg(short = 'p', long, default_value = "10000")]
    pub(crate) port: u16,

    #[arg(short = 'H', long, default_value = "127.0.0.1")]
    pub(crate) host: String,

    #[arg(short = 'c', long)]
    pub(crate) config: Option<PathBuf>,

    /// Quiet mode: suppress focus and layer-switch messages
    #[arg(short = 'q', long)]
    pub(crate) quiet: bool,

    /// Suppress focus messages only
    #[arg(long)]
    pub(crate) quiet_focus: bool,

    /// Auto-install GNOME extension if missing (default behavior)
    #[arg(long)]
    pub(crate) install_gnome_extension: bool,

    /// Do not auto-install GNOME extension
    #[arg(long)]
    pub(crate) no_install_gnome_extension: bool,

    /// Disable the StatusNotifier (SNI) indicator on non-GNOME desktops
    #[arg(long)]
    pub(crate) no_indicator: bool,

    /// Override SNI focus-only mode (true/false). When set, GSettings is not read.
    #[arg(long, value_enum, value_name = "true|false")]
    pub(crate) indicator_focus_only: Option<TrayFocusOnly>,

    /// Install autostart desktop entry and exit
    #[arg(long, conflicts_with_all = ["uninstall_autostart", "restart", "pause", "unpause"])]
    pub(crate) install_autostart: bool,

    /// Uninstall autostart desktop entry and exit
    #[arg(long, conflicts_with_all = ["install_autostart", "restart", "pause", "unpause"])]
    pub(crate) uninstall_autostart: bool,

    /// Send Restart request to an existing daemon and exit
    #[arg(long, conflicts_with_all = ["pause", "unpause"])]
    pub(crate) restart: bool,

    /// Send Pause request to an existing daemon and exit
    #[arg(long, conflicts_with_all = ["restart", "unpause"])]
    pub(crate) pause: bool,

    /// Send Unpause request to an existing daemon and exit
    #[arg(long, conflicts_with_all = ["restart", "pause"])]
    pub(crate) unpause: bool,

    /// DBus suffix used as the last name element of this daemon's well-known bus
    /// name (`com.github.kanata.Switcher.instances.<suffix>`). When omitted the
    /// suffix is auto-derived from `--host` and `--port`. Control CLI commands
    /// target this exact suffix when set; otherwise they broadcast.
    #[arg(long, value_name = "SUFFIX", value_parser = parse_dbus_suffix_arg)]
    pub(crate) dbus_suffix: Option<String>,
}

pub(crate) fn parse_dbus_suffix_arg(raw: &str) -> Result<String, String> {
    sanitize_dbus_suffix(raw).map_err(|error| error.to_string())
}

pub(crate) fn resolve_install_gnome_extension(matches: &ArgMatches) -> bool {
    use clap::parser::ValueSource;

    let install_from_cli =
        matches.value_source("install_gnome_extension") == Some(ValueSource::CommandLine);
    let no_install_from_cli =
        matches.value_source("no_install_gnome_extension") == Some(ValueSource::CommandLine);

    match (install_from_cli, no_install_from_cli) {
        (false, false) => true,
        (true, false) => true,
        (false, true) => false,
        (true, true) => {
            let install_idx = matches.index_of("install_gnome_extension");
            let no_install_idx = matches.index_of("no_install_gnome_extension");
            match (install_idx, no_install_idx) {
                (Some(i), Some(n)) => i > n,
                _ => true,
            }
        }
    }
}

pub(crate) fn resolve_control_command(args: &Args) -> Option<ControlCommand> {
    if args.restart {
        return Some(ControlCommand::Restart);
    }
    if args.pause {
        return Some(ControlCommand::Pause);
    }
    if args.unpause {
        return Some(ControlCommand::Unpause);
    }
    None
}

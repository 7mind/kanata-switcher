use clap::ArgMatches;
use std::env;
use std::path::{Path, PathBuf};
use crate::args::Args;
use crate::errors::DynError;

pub(crate) const AUTOSTART_DESKTOP_FILENAME: &str = "kanata-switcher.desktop";
pub(crate) const AUTOSTART_PASSTHROUGH_OPTIONS: &[&str] = &[
    "port",
    "host",
    "config",
    "quiet",
    "quiet_focus",
    "install_gnome_extension",
    "no_install_gnome_extension",
    "no_indicator",
    "indicator_focus_only",
    "dbus_suffix",
];
pub(crate) const AUTOSTART_ONESHOT_OPTIONS: &[&str] = &[
    "restart",
    "pause",
    "unpause",
    "install_autostart",
    "uninstall_autostart",
];

pub(crate) fn resolve_binary_path() -> Result<PathBuf, DynError> {
    let exe_path = env::current_exe()?;
    let canonical = exe_path.canonicalize()?;
    Ok(canonical)
}

pub(crate) fn autostart_dir() -> Result<PathBuf, DynError> {
    if let Ok(xdg_config_home) = env::var("XDG_CONFIG_HOME") {
        if xdg_config_home.is_empty() {
            return Err("XDG_CONFIG_HOME is empty".into());
        }
        return Ok(PathBuf::from(xdg_config_home).join("autostart"));
    }
    let home = env::var("HOME")?;
    if home.is_empty() {
        return Err("HOME is empty".into());
    }
    Ok(PathBuf::from(home).join(".config").join("autostart"))
}

pub(crate) fn autostart_desktop_path() -> Result<PathBuf, DynError> {
    Ok(autostart_dir()?.join(AUTOSTART_DESKTOP_FILENAME))
}

pub(crate) fn escape_desktop_exec_arg(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '%' => escaped.push_str("%%"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            _ => escaped.push(ch),
        }
    }
    escaped.push('"');
    escaped
}

pub(crate) fn build_autostart_desktop_content(exec_path: &Path, exec_args: &[String]) -> String {
    let exec_path_str = exec_path
        .to_str()
        .expect("autostart exec path contains invalid UTF-8");
    let mut exec_parts = Vec::with_capacity(exec_args.len() + 1);
    exec_parts.push(escape_desktop_exec_arg(exec_path_str));
    for arg in exec_args {
        exec_parts.push(escape_desktop_exec_arg(arg));
    }
    let exec_line = exec_parts.join(" ");
    format!(
        "[Desktop Entry]\nType=Application\nName=Kanata Switcher\nExec={}\nTryExec={}\nX-GNOME-Autostart-enabled=true\n",
        exec_line,
        escape_desktop_exec_arg(exec_path_str)
    )
}

pub(crate) fn autostart_passthrough_args(matches: &ArgMatches, args: &Args) -> Vec<String> {
    use clap::parser::ValueSource;

    let mut exec_args = Vec::new();

    for &name in AUTOSTART_PASSTHROUGH_OPTIONS {
        if matches.value_source(name) != Some(ValueSource::CommandLine) {
            continue;
        }
        match name {
            "port" => {
                exec_args.push("-p".to_string());
                exec_args.push(args.port.to_string());
            }
            "host" => {
                exec_args.push("-H".to_string());
                exec_args.push(args.host.clone());
            }
            "config" => {
                let config = args
                    .config
                    .as_ref()
                    .expect("config missing after command-line input");
                exec_args.push("-c".to_string());
                exec_args.push(config.to_string_lossy().to_string());
            }
            "quiet" => {
                exec_args.push("-q".to_string());
            }
            "quiet_focus" => {
                exec_args.push("--quiet-focus".to_string());
            }
            "install_gnome_extension" => {
                exec_args.push("--install-gnome-extension".to_string());
            }
            "no_install_gnome_extension" => {
                exec_args.push("--no-install-gnome-extension".to_string());
            }
            "no_indicator" => {
                exec_args.push("--no-indicator".to_string());
            }
            "indicator_focus_only" => {
                let value = args
                    .indicator_focus_only
                    .expect("indicator_focus_only missing after command-line input");
                exec_args.push("--indicator-focus-only".to_string());
                exec_args.push(value.as_arg().to_string());
            }
            "dbus_suffix" => {
                let suffix = args
                    .dbus_suffix
                    .as_ref()
                    .expect("dbus_suffix missing after command-line input");
                exec_args.push("--dbus-suffix".to_string());
                exec_args.push(suffix.clone());
            }
            _ => {
                panic!("autostart passthrough option missing handler: {}", name);
            }
        }
    }

    exec_args
}

pub(crate) fn install_autostart_desktop(
    matches: &ArgMatches,
    args: &Args,
) -> Result<(), DynError> {
    for option in AUTOSTART_ONESHOT_OPTIONS {
        if AUTOSTART_PASSTHROUGH_OPTIONS.contains(option) {
            return Err(format!("autostart option lists overlap: {}", option).into());
        }
    }
    let exec_path = resolve_binary_path()?;
    let exec_args = autostart_passthrough_args(matches, args);
    let content = build_autostart_desktop_content(&exec_path, &exec_args);

    let autostart_dir = autostart_dir()?;
    std::fs::create_dir_all(&autostart_dir)?;
    let desktop_path = autostart_dir.join(AUTOSTART_DESKTOP_FILENAME);

    std::fs::write(&desktop_path, content)?;
    println!("[Autostart] Installed {}", desktop_path.display());
    Ok(())
}

pub(crate) fn uninstall_autostart_desktop() -> Result<(), DynError> {
    let desktop_path = autostart_desktop_path()?;
    if !desktop_path.exists() {
        return Err(format!("autostart entry not found: {}", desktop_path.display()).into());
    }
    std::fs::remove_file(&desktop_path)?;
    println!("[Autostart] Removed {}", desktop_path.display());
    Ok(())
}

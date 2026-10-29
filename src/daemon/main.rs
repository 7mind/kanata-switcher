// On non-Linux platforms many types and imports are suppressed by #[cfg] gates.
// Silence the resulting dead-code / unused warnings without polluting every site.
#![cfg_attr(not(target_os = "linux"), allow(dead_code, unused_imports, unused_variables))]

use clap::{CommandFactory, FromArgMatches};

// Existing Linux tests share these fixtures through `super::*`.
#[cfg(all(test, target_os = "linux"))]
#[allow(unused_imports)]
use {
    futures_util::StreamExt,
    std::{collections::HashMap, env, fs, os::{fd::AsFd, unix::net::UnixStream},
          path::{Path, PathBuf}, process::Command,
          sync::{Arc, Mutex, atomic::{AtomicBool, AtomicU64, Ordering}}, time::Duration},
    tokio::sync::{Mutex as TokioMutex, mpsc, oneshot, watch},
    uuid::Uuid,
    zbus::{Connection, zvariant::{OwnedObjectPath, OwnedValue, Str, Structure, Value}},
};

mod constants;
mod errors;
mod environ;
mod dbus_naming;
mod config;
mod focus;
mod args;
mod broadcasters;
mod kanata;
mod control;
#[cfg(target_os = "linux")]
mod pause;
mod focus_pipeline;
mod backends;

#[cfg(target_os = "linux")]
mod autostart;
#[cfg(target_os = "linux")]
mod lifecycle;
#[cfg(target_os = "linux")]
mod display_override;
#[cfg(target_os = "linux")]
mod supervisor;
#[cfg(target_os = "linux")]
mod sni;
#[cfg(target_os = "linux")]
mod gnome_ext;

use args::Args;
use config::load_config;
use environ::{detect_environment, RunOutcome};

mod platform;

#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use crate::{constants::*, errors::*, environ::*, dbus_naming::*, config::*, focus::*, args::*, broadcasters::*, kanata::*, control::*, focus_pipeline::*, backends::*};
#[cfg(test)]
#[cfg(target_os = "linux")]
pub(crate) use crate::pause::*;
#[cfg(test)]
#[cfg(target_os = "linux")]
#[allow(unused_imports)]
pub(crate) use crate::{control::client::*, control::server::*, control::persistent::*, autostart::*, lifecycle::*, lifecycle::logind::*, lifecycle::startup::*, display_override::*, supervisor::*, supervisor::capabilities::*, backends::gnome::*, backends::kde::*, backends::kde::script::*, backends::kde::probe::*, backends::wayland::*, sni::*, sni::settings::*, sni::state::*, sni::indicator::*, sni::control_local::*, sni::control_dbus::*, sni::control_ops::*, sni::guard::*, gnome_ext::*, gnome_ext::detection::*, gnome_ext::install::*};
#[cfg(test)]
#[cfg(target_os = "macos")]
pub(crate) use crate::backends::macos::*;
#[cfg(test)]
#[cfg(target_os = "windows")]
pub(crate) use crate::backends::windows::*;

#[cfg(not(target_os = "macos"))]
#[tokio::main]
async fn main() {
    run_main().await;
}

#[cfg(target_os = "macos")]
fn main() {
    platform::run_macos_main(run_main());
}

async fn run_main() {
    loop {
        match run_once().await {
            Ok(RunOutcome::Restart) => {
                println!("[Restart] Restarting daemon");
            }
            Ok(RunOutcome::Exit) => break,
            Err(e) => {
                eprintln!("[Fatal] {}", e);
                std::process::exit(1);
            }
        }
    }
}

async fn run_once() -> Result<RunOutcome, Box<dyn std::error::Error + Send + Sync>> {
    let matches = Args::command().get_matches();
    let args = Args::from_arg_matches(&matches)?;
    if platform::run_command(&matches, &args).await? {
        return Ok(RunOutcome::Exit);
    }

    let detected_env = detect_environment();
    println!("[Init] Detected environment: {}", detected_env.as_str());

    let config = load_config(args.config.as_deref());
    if config.rules.is_empty() && config.native_terminal_rule.is_none() {
        eprintln!("[Config] Error: No rules found in config file");
        eprintln!();
        eprintln!("Example config (~/.config/kanata/kanata-switcher.json):");
        eprintln!(
            r#"[
  {{"default": "base"}},
  {{"on_native_terminal": "tty"}},
  {{"class": "firefox", "layer": "browser"}},
  {{"class": "alacritty", "title": "vim", "layer": "vim"}}
]"#
        );
        std::process::exit(1);
    }

    let quiet_focus = args.quiet || args.quiet_focus;

    platform::run(args, config, quiet_focus).await
}

#[cfg(test)]
#[cfg(target_os = "linux")]
mod tests;

#[cfg(test)]
mod integration_tests;

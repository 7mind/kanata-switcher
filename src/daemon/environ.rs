// Many environment variants and types are Linux-only.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::env;

// === Environment Detection ===

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Environment {
    Gnome,
    Kde,
    Wayland,
    X11,
    MacOS,
    Windows,
    LinuxConsoleWithLogind,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunOutcome {
    Restart,
    Exit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionKind {
    NoSession,
    GraphicalX11,
    GraphicalWayland,
    NativeTerminal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DesktopFlavor {
    Gnome,
    Kde,
    GenericWayland,
    X11,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackendKind {
    Gnome,
    Kde,
    Wayland,
    X11,
    MacOS,
    Windows,
    LinuxConsole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeTarget {
    Backend(BackendKind),
    Idle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DesktopCapabilities {
    pub(crate) gnome_owner: bool,
    pub(crate) kde_owner: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LifecycleSnapshot {
    pub(crate) active: bool,
    pub(crate) session_type: String,
    pub(crate) session_kind: SessionKind,
}

pub(crate) fn session_type_to_session_kind(active: bool, session_type: &str) -> SessionKind {
    if session_type_indicates_native_terminal(session_type) {
        return if active {
            SessionKind::NativeTerminal
        } else {
            SessionKind::NoSession
        };
    }
    match session_type {
        "x11" => {
            if active {
                SessionKind::GraphicalX11
            } else {
                SessionKind::NativeTerminal
            }
        }
        "wayland" | "gnome" | "kde" => {
            if active {
                SessionKind::GraphicalWayland
            } else {
                SessionKind::NativeTerminal
            }
        }
        _ => SessionKind::NoSession,
    }
}

pub(crate) fn session_type_indicates_native_terminal(session_type: &str) -> bool {
    session_type == "tty"
}

pub(crate) fn resolve_desktop_flavor(
    session_kind: SessionKind,
    capabilities: DesktopCapabilities,
) -> DesktopFlavor {
    match session_kind {
        SessionKind::GraphicalX11 => DesktopFlavor::X11,
        SessionKind::GraphicalWayland => {
            if capabilities.gnome_owner {
                DesktopFlavor::Gnome
            } else if capabilities.kde_owner {
                DesktopFlavor::Kde
            } else {
                DesktopFlavor::GenericWayland
            }
        }
        SessionKind::NativeTerminal | SessionKind::NoSession => DesktopFlavor::Unknown,
    }
}

pub(crate) fn resolve_runtime_target(
    session_kind: SessionKind,
    capabilities: DesktopCapabilities,
) -> RuntimeTarget {
    match session_kind {
        SessionKind::NoSession => RuntimeTarget::Idle,
        SessionKind::NativeTerminal => RuntimeTarget::Backend(BackendKind::LinuxConsole),
        SessionKind::GraphicalX11 => RuntimeTarget::Backend(BackendKind::X11),
        SessionKind::GraphicalWayland => match resolve_desktop_flavor(session_kind, capabilities) {
            DesktopFlavor::Gnome => RuntimeTarget::Backend(BackendKind::Gnome),
            DesktopFlavor::Kde => RuntimeTarget::Backend(BackendKind::Kde),
            DesktopFlavor::GenericWayland => RuntimeTarget::Backend(BackendKind::Wayland),
            DesktopFlavor::X11 => RuntimeTarget::Backend(BackendKind::X11),
            DesktopFlavor::Unknown => RuntimeTarget::Idle,
        },
    }
}

pub(crate) fn runtime_target_from_wayland_startup_session_type_hint(
    session_type: &str,
) -> Option<RuntimeTarget> {
    match session_type {
        "gnome" => Some(RuntimeTarget::Backend(BackendKind::Gnome)),
        "kde" => Some(RuntimeTarget::Backend(BackendKind::Kde)),
        _ => None,
    }
}

pub(crate) fn target_requires_session_bus(target: RuntimeTarget) -> bool {
    match target {
        RuntimeTarget::Backend(BackendKind::Gnome)
        | RuntimeTarget::Backend(BackendKind::Kde)
        | RuntimeTarget::Backend(BackendKind::Wayland)
        | RuntimeTarget::Backend(BackendKind::X11) => true,
        RuntimeTarget::Backend(BackendKind::MacOS)
        | RuntimeTarget::Backend(BackendKind::Windows)
        | RuntimeTarget::Backend(BackendKind::LinuxConsole)
        | RuntimeTarget::Idle => false,
    }
}

pub(crate) fn startup_environment_to_snapshot(env: Environment) -> LifecycleSnapshot {
    let (active, session_type) = match env {
        Environment::Gnome => (true, "gnome"),
        Environment::Kde => (true, "kde"),
        Environment::Wayland => (true, "wayland"),
        Environment::X11 => (true, "x11"),
        Environment::MacOS => (true, "macos"),
        Environment::Windows => (true, "windows"),
        Environment::LinuxConsoleWithLogind => (true, "tty"),
        Environment::Unknown => (false, ""),
    };
    let session_type = session_type.to_string();
    let session_kind = session_type_to_session_kind(active, &session_type);
    LifecycleSnapshot {
        active,
        session_type,
        session_kind,
    }
}

impl Environment {
    pub fn as_str(&self) -> &'static str {
        match self {
            Environment::Gnome => "gnome",
            Environment::Kde => "kde",
            Environment::Wayland => "wayland",
            Environment::X11 => "x11",
            Environment::MacOS => "macos",
            Environment::Windows => "windows",
            Environment::LinuxConsoleWithLogind => "linux-console-with-logind",
            Environment::Unknown => "unknown",
        }
    }
}

pub(crate) fn detect_environment() -> Environment {
    detect_environment_impl()
}

#[cfg(target_os = "macos")]
fn detect_environment_impl() -> Environment {
    Environment::MacOS
}

#[cfg(target_os = "windows")]
fn detect_environment_impl() -> Environment {
    Environment::Windows
}

#[cfg(target_os = "linux")]
fn detect_environment_impl() -> Environment {
    let desktop = env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_lowercase();

    if desktop.contains("gnome-greeter") {
        return Environment::Unknown;
    }

    // GNOME - needs special DBus extension
    if desktop.contains("gnome") || env::var("GNOME_SETUP_DISPLAY").is_ok() {
        return Environment::Gnome;
    }

    // KDE - needs KWin script injection
    if env::var("KDE_SESSION_VERSION").is_ok() {
        return Environment::Kde;
    }

    // Wayland compositors (wlr-based or COSMIC) - use toplevel protocol
    if env::var("WAYLAND_DISPLAY").is_ok() {
        return Environment::Wayland;
    }

    // X11 fallback
    if env::var("DISPLAY").is_ok() {
        return Environment::X11;
    }

    Environment::Unknown
}

// Impl for any platform not covered above
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn detect_environment_impl() -> Environment {
    Environment::Unknown
}

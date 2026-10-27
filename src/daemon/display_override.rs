use std::path::Path;
use zbus::Connection;
use crate::constants::*;
use crate::environ::*;
use crate::errors::DynError;
use crate::lifecycle::logind::*;

pub(crate) fn display_override_expected_session_type(kind: BackendKind) -> Option<&'static str> {
    match kind {
        BackendKind::Wayland => Some("wayland"),
        BackendKind::X11 => Some("x11"),
        BackendKind::Gnome | BackendKind::Kde | BackendKind::LinuxConsole => None,
    }
}

pub(crate) fn is_valid_wayland_display_override(display: &str) -> bool {
    if Path::new(display).is_absolute() {
        return true;
    }
    if display.starts_with(':') {
        return false;
    }
    if display.contains('/') {
        return false;
    }
    true
}

pub(crate) fn normalize_display_override(kind: BackendKind, display: &str) -> Option<String> {
    let trimmed = display.trim();
    if trimmed.is_empty() {
        return None;
    }
    if kind == BackendKind::Wayland && !is_valid_wayland_display_override(trimmed) {
        return None;
    }
    Some(trimmed.to_string())
}

pub(crate) async fn resolve_display_override_from_logind(
    kind: BackendKind,
) -> Result<Option<String>, DynError> {
    let expected_type = match display_override_expected_session_type(kind) {
        Some(value) => value,
        None => return Ok(None),
    };

    let system_connection = Connection::system().await?;
    let session_path = match resolve_logind_session_path(&system_connection).await {
        Ok(path) => path,
        Err(LogindSessionPathResolutionError::DisplayNotReady) => return Ok(None),
        Err(LogindSessionPathResolutionError::Fatal(error)) => return Err(error),
    };
    let session_proxy = zbus::Proxy::new(
        &system_connection,
        LOGIND_BUS_NAME,
        session_path.as_str(),
        LOGIND_SESSION_INTERFACE,
    )
    .await?;
    let session_type: String = session_proxy.get_property("Type").await?;
    if session_type != expected_type {
        return Ok(None);
    }

    let display: String = session_proxy.get_property("Display").await?;
    let normalized = normalize_display_override(kind, &display);
    if kind == BackendKind::Wayland && normalized.is_none() && !display.trim().is_empty() {
        eprintln!(
            "[Lifecycle] Ignoring logind Wayland Display override '{}': not a valid Wayland socket value",
            display.trim()
        );
    }
    Ok(normalized)
}

pub(crate) fn display_override_backend_kind_for_environment(env: Environment) -> Option<BackendKind> {
    match env {
        Environment::Wayland => Some(BackendKind::Wayland),
        Environment::X11 => Some(BackendKind::X11),
        Environment::Gnome
        | Environment::Kde
        | Environment::LinuxConsoleWithLogind
        | Environment::Unknown => None,
    }
}

#[cfg(test)]
pub(crate) static TEST_X11_FOCUS_QUERY_DISPLAY_OVERRIDE: std::sync::Mutex<Option<String>> =
    std::sync::Mutex::new(None);
#[cfg(test)]
pub(crate) static TEST_WAYLAND_FOCUS_QUERY_DISPLAY_OVERRIDE: std::sync::Mutex<Option<String>> =
    std::sync::Mutex::new(None);

#[cfg(test)]
pub(crate) struct TestFocusQueryDisplayOverrideGuard {
    pub(crate) env: Environment,
    pub(crate) previous: Option<String>,
}

#[cfg(test)]
impl Drop for TestFocusQueryDisplayOverrideGuard {
    fn drop(&mut self) {
        if let Some(slot) = display_override_test_slot(self.env) {
            *slot.lock().unwrap() = self.previous.clone();
        }
    }
}

#[cfg(test)]
pub(crate) fn display_override_test_slot(
    env: Environment,
) -> Option<&'static std::sync::Mutex<Option<String>>> {
    match env {
        Environment::X11 => Some(&TEST_X11_FOCUS_QUERY_DISPLAY_OVERRIDE),
        Environment::Wayland => Some(&TEST_WAYLAND_FOCUS_QUERY_DISPLAY_OVERRIDE),
        Environment::Gnome
        | Environment::Kde
        | Environment::LinuxConsoleWithLogind
        | Environment::Unknown => None,
    }
}

#[cfg(test)]
pub(crate) fn set_test_focus_query_display_override(
    env: Environment,
    override_value: Option<&str>,
) -> TestFocusQueryDisplayOverrideGuard {
    let slot = display_override_test_slot(env)
        .expect("focus-query test display override is only valid for X11/Wayland");
    let mut guard = slot.lock().unwrap();
    let previous = guard.clone();
    *guard = override_value.map(str::to_string);
    TestFocusQueryDisplayOverrideGuard { env, previous }
}

#[cfg(test)]
pub(crate) fn resolve_test_focus_query_display_override(env: Environment) -> Option<String> {
    let slot = match display_override_test_slot(env) {
        Some(slot) => slot,
        None => return None,
    };
    slot.lock().unwrap().clone()
}

#[cfg(not(test))]
pub(crate) fn resolve_test_focus_query_display_override(_env: Environment) -> Option<String> {
    None
}

pub(crate) async fn resolve_display_override_for_backend_kind(
    kind: BackendKind,
    context_label: &str,
) -> Option<String> {
    let expected_type = match display_override_expected_session_type(kind) {
        Some(value) => value,
        None => return None,
    };
    match resolve_display_override_from_logind(kind).await {
        Ok(Some(display)) => {
            println!(
                "[{}] Refreshed {} display endpoint from logind: {}",
                context_label, expected_type, display
            );
            Some(display)
        }
        Ok(None) => None,
        Err(error) => {
            eprintln!(
                "[{}] Failed to refresh {} display endpoint from logind: {}",
                context_label, expected_type, error
            );
            None
        }
    }
}

pub(crate) async fn resolve_display_override_for_environment(
    env: Environment,
    context_label: &str,
) -> Option<String> {
    if let Some(display) = resolve_test_focus_query_display_override(env) {
        return Some(display);
    }
    let kind = match display_override_backend_kind_for_environment(env) {
        Some(kind) => kind,
        None => return None,
    };
    resolve_display_override_for_backend_kind(kind, context_label).await
}

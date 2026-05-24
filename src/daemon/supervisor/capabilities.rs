use zbus::Connection;
use crate::constants::*;
use crate::environ::*;
use crate::errors::DynError;

pub(crate) async fn detect_desktop_capabilities() -> Result<DesktopCapabilities, DynError> {
    let connection = Connection::session().await?;
    let dbus = zbus::fdo::DBusProxy::new(&connection).await?;
    let gnome_owner = crate::gnome_ext::detection::session_bus_name_has_owner(&dbus, GNOME_SHELL_BUS_NAME).await;
    let kde_owner = crate::gnome_ext::detection::session_bus_name_has_owner(&dbus, KDE_KWIN_BUS_NAME).await;
    Ok(DesktopCapabilities {
        gnome_owner,
        kde_owner,
    })
}

pub(crate) async fn resolve_runtime_target_for_snapshot(
    snapshot: &LifecycleSnapshot,
) -> Result<RuntimeTarget, DynError> {
    let capabilities = if snapshot.session_kind == SessionKind::GraphicalWayland {
        if let Some(hinted_target) =
            runtime_target_from_wayland_startup_session_type_hint(snapshot.session_type.as_str())
        {
            return Ok(hinted_target);
        }
        detect_desktop_capabilities().await?
    } else {
        DesktopCapabilities {
            gnome_owner: false,
            kde_owner: false,
        }
    };
    Ok(resolve_runtime_target(snapshot.session_kind, capabilities))
}

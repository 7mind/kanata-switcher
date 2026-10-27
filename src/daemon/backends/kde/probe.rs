use std::fs;
use std::time::Duration;
use std::sync::atomic::Ordering;
use tokio::sync::{Mutex as TokioMutex, oneshot};
use zbus::Connection;
use crate::constants::*;
use crate::environ::Environment;
use crate::config::WindowInfo;
use super::script::{
    KDE_QUERY_COUNTER, KdeFocusQueryService,
    build_kde_query_script, kwin_query_script_path, kwin_query_probe_script_path,
    load_kwin_script,
};
use super::KwinScriptGuard;

pub(crate) async fn resolve_kde_runtime_query_mode_with_retry(
    connection: &Connection,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let mut last_error: Option<Box<dyn std::error::Error + Send + Sync>> = None;

    for attempt in 0..KDE_RUNTIME_QUERY_MODE_MAX_ATTEMPTS {
        match ensure_kde_scripting_ready(connection).await {
            Ok(()) => match resolve_kde_runtime_query_mode(connection).await {
                Ok(is_kde6) => return Ok(is_kde6),
                Err(error) => {
                    eprintln!(
                        "[KDE] Runtime query mode probe attempt {}/{} failed: {}",
                        attempt + 1,
                        KDE_RUNTIME_QUERY_MODE_MAX_ATTEMPTS,
                        error
                    );
                    last_error = Some(error);
                }
            },
            Err(error) => {
                eprintln!(
                    "[KDE] Scripting readiness check attempt {}/{} failed: {}",
                    attempt + 1,
                    KDE_RUNTIME_QUERY_MODE_MAX_ATTEMPTS,
                    error
                );
                last_error = Some(error);
            }
        }

        if attempt + 1 < KDE_RUNTIME_QUERY_MODE_MAX_ATTEMPTS {
            tokio::time::sleep(KDE_RUNTIME_QUERY_MODE_RETRY_DELAY).await;
        }
    }

    Err(last_error.unwrap_or_else(|| {
        std::io::Error::other("[KDE] Runtime query mode probe failed without error").into()
    }))
}

pub(crate) async fn ensure_kde_scripting_ready(
    connection: &Connection,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let dbus = zbus::fdo::DBusProxy::new(connection).await?;
    let has_owner = dbus
        .name_has_owner(KDE_KWIN_BUS_NAME.try_into().unwrap())
        .await?;
    if !has_owner {
        return Err(
            std::io::Error::other(format!("[KDE] {} is not owned", KDE_KWIN_BUS_NAME)).into(),
        );
    }

    if dbus
        .name_has_owner(KDE_KWIN_SCRIPTING_INTERFACE.try_into().unwrap())
        .await?
    {
        return Ok(());
    }

    let introspection = connection
        .call_method(
            Some(KDE_KWIN_BUS_NAME),
            KDE_KWIN_SCRIPTING_PATH,
            Some(DBUS_INTROSPECTABLE_INTERFACE),
            "Introspect",
            &(),
        )
        .await?;
    let introspection_xml: String = introspection.body().deserialize()?;
    if !introspection_xml.contains(KDE_KWIN_SCRIPTING_INTERFACE) {
        return Err(std::io::Error::other(format!(
            "[KDE] {} is not exported on {}",
            KDE_KWIN_SCRIPTING_INTERFACE, KDE_KWIN_SCRIPTING_PATH
        ))
        .into());
    }

    Ok(())
}

pub(crate) async fn resolve_kde_runtime_query_mode(
    connection: &Connection,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let probe_id = KDE_QUERY_COUNTER.fetch_add(1, Ordering::SeqCst);
    let script_path = kwin_query_probe_script_path(probe_id);
    fs::write(&script_path, "function kanataSwitcherProbe() {}\n")?;

    let load_result = connection
        .call_method(
            Some(KDE_KWIN_BUS_NAME),
            KDE_KWIN_SCRIPTING_PATH,
            Some(KDE_KWIN_SCRIPTING_INTERFACE),
            "loadScript",
            &(&script_path,),
        )
        .await;
    let load_reply = match load_result {
        Ok(reply) => reply,
        Err(error) => {
            let _ = remove_kwin_probe_script_file(&script_path);
            return Err(Box::new(error));
        }
    };

    let script_num: i32 = match load_reply.body().deserialize() {
        Ok(script_num) => script_num,
        Err(error) => {
            let _ = unload_kwin_script_by_path(connection, &script_path).await;
            let _ = remove_kwin_probe_script_file(&script_path);
            return Err(Box::new(error));
        }
    };

    let kde6_path = format!("/Scripting/Script{}", script_num);
    let kde5_path = format!("/{}", script_num);
    let kde6_path_exists = kwin_object_path_exists(connection, kde6_path.as_str()).await;
    let kde5_path_exists = kwin_object_path_exists(connection, kde5_path.as_str()).await;

    unload_kwin_script_by_path(connection, &script_path).await?;
    remove_kwin_probe_script_file(&script_path)?;

    match (kde6_path_exists, kde5_path_exists) {
        (true, false) => Ok(true),
        (false, true) => Ok(false),
        (true, true) => Err(std::io::Error::other(
            "[KDE] Runtime query mode probe found both KDE5 and KDE6 script paths",
        )
        .into()),
        (false, false) => Err(std::io::Error::other(
            "[KDE] Runtime query mode probe found no known KWin script path",
        )
        .into()),
    }
}

pub(crate) async fn kwin_object_path_exists(connection: &Connection, object_path: &str) -> bool {
    connection
        .call_method(
            Some(KDE_KWIN_BUS_NAME),
            object_path,
            Some("org.freedesktop.DBus.Introspectable"),
            "Introspect",
            &(),
        )
        .await
        .is_ok()
}

pub(crate) async fn unload_kwin_script_by_path(
    connection: &Connection,
    script_path: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    connection
        .call_method(
            Some(KDE_KWIN_BUS_NAME),
            KDE_KWIN_SCRIPTING_PATH,
            Some(KDE_KWIN_SCRIPTING_INTERFACE),
            "unloadScript",
            &(&script_path,),
        )
        .await?;
    Ok(())
}

pub(crate) fn remove_kwin_probe_script_file(
    script_path: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match fs::remove_file(script_path) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Box::new(error)),
    }
}

pub(crate) fn environment_requires_focus_query_connection(env: Environment) -> bool {
    matches!(
        env,
        Environment::Gnome | Environment::Kde | Environment::Wayland | Environment::X11
    )
}

pub(crate) async fn query_kde_focus(
    connection: &Connection,
    is_kde6: bool,
) -> Result<WindowInfo, Box<dyn std::error::Error + Send + Sync>> {
    let unique_name = connection
        .unique_name()
        .ok_or("KDE focus query requires a unique DBus name")?;
    let query_id = KDE_QUERY_COUNTER.fetch_add(1, Ordering::SeqCst);
    let query_path = format!("/com/github/kanata/Switcher/KdeQuery{}", query_id);
    let (sender, receiver) = oneshot::channel();
    let service = KdeFocusQueryService {
        sender: TokioMutex::new(Some(sender)),
    };
    connection
        .object_server()
        .at(query_path.as_str(), service)
        .await?;

    let script_path = kwin_query_script_path(query_id);
    let script = build_kde_query_script(is_kde6, unique_name.as_str(), query_path.as_str());
    fs::write(&script_path, script)?;

    let (script_obj_path, script_interface) =
        load_kwin_script(connection, &script_path, is_kde6, false).await?;

    let _kwin_query_guard = KwinScriptGuard::new(
        connection.clone(),
        tokio::runtime::Handle::current(),
        script_path.clone(),
        script_obj_path.clone(),
        script_interface,
    );

    connection
        .call_method(
            Some("org.kde.KWin"),
            script_obj_path,
            Some(script_interface),
            "run",
            &(),
        )
        .await?;

    let win = tokio::time::timeout(Duration::from_secs(5), receiver)
        .await
        .map_err(|_| "Timed out waiting for KDE focus callback")?
        .map_err(|_| "KDE focus callback sender dropped")?;

    Ok(win)
}

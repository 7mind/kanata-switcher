use std::time::Duration;
use std::sync::atomic::AtomicU64;
use tokio::sync::{Mutex as TokioMutex, oneshot};
use uuid::Uuid;
use zbus::Connection;
use zbus::zvariant::OwnedObjectPath;
use crate::constants::*;
use crate::config::WindowInfo;

pub(crate) static KDE_QUERY_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(crate) fn kwin_query_script_path(query_id: u64) -> String {
    let uid = unsafe { libc::getuid() };
    let request_id = Uuid::new_v4().hyphenated().to_string();
    format!(
        "/tmp/kanata-switcher-kwin-query-{}-{}-{}.js",
        uid, query_id, request_id
    )
}

pub(crate) fn kwin_query_probe_script_path(probe_id: u64) -> String {
    let uid = unsafe { libc::getuid() };
    let request_id = Uuid::new_v4().hyphenated().to_string();
    format!(
        "/tmp/kanata-switcher-kwin-query-probe-{}-{}-{}.js",
        uid, probe_id, request_id
    )
}

pub(crate) fn kwin_runtime_script_path() -> String {
    let uid = unsafe { libc::getuid() };
    let request_id = Uuid::new_v4().hyphenated().to_string();
    format!("/tmp/kanata-switcher-kwin-{}-{}.js", uid, request_id)
}

#[derive(Debug)]
pub(crate) struct KdeFocusQueryService {
    pub(crate) sender: TokioMutex<Option<oneshot::Sender<WindowInfo>>>,
}

#[zbus::interface(name = "com.github.kanata.Switcher.KdeQuery")]
impl KdeFocusQueryService {
    #[allow(non_snake_case)]
    async fn Focus(&self, window_class: &str, window_title: &str) {
        let win = WindowInfo {
            class: window_class.to_string(),
            title: window_title.to_string(),
            is_native_terminal: false,
        };
        let mut sender = self.sender.lock().await;
        if let Some(tx) = sender.take() {
            let _ = tx.send(win);
        }
    }
}

pub(crate) fn kwin_script_object_path(
    script_num: i32,
    is_kde6: bool,
) -> Result<OwnedObjectPath, Box<dyn std::error::Error + Send + Sync>> {
    let path = if is_kde6 {
        format!("/Scripting/Script{}", script_num)
    } else {
        format!("/{}", script_num)
    };
    let obj_path: OwnedObjectPath = path.as_str().try_into()?;
    Ok(obj_path)
}

pub(crate) async fn load_kwin_script(
    connection: &Connection,
    script_path: &str,
    is_kde6: bool,
    cleanup_existing: bool,
) -> Result<(OwnedObjectPath, &'static str), Box<dyn std::error::Error + Send + Sync>> {
    if cleanup_existing {
        for _ in 0..5 {
            let result = connection
                .call_method(
                    Some("org.kde.KWin"),
                    "/Scripting",
                    Some("org.kde.kwin.Scripting"),
                    "loadScript",
                    &(&script_path,),
                )
                .await;

            if result.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }

        let _ = connection
            .call_method(
                Some("org.kde.KWin"),
                "/Scripting",
                Some("org.kde.kwin.Scripting"),
                "unloadScript",
                &(&script_path,),
            )
            .await;
    }

    let load_result = connection
        .call_method(
            Some("org.kde.KWin"),
            "/Scripting",
            Some("org.kde.kwin.Scripting"),
            "loadScript",
            &(&script_path,),
        )
        .await?;

    let script_num: i32 = load_result.body().deserialize()?;
    let obj_path = kwin_script_object_path(script_num, is_kde6)?;
    Ok((obj_path, "org.kde.kwin.Script"))
}

pub(crate) fn build_kde_query_script(is_kde6: bool, bus_name: &str, object_path: &str) -> String {
    let active_window = if is_kde6 {
        "activeWindow"
    } else {
        "activeClient"
    };
    format!(
        r#"function reportFocus(client) {{
  callDBus(
    "{bus}",
    "{path}",
    "{iface}",
    "{method}",
    client ? (client.resourceClass || "") : "",
    client ? (client.caption || "") : ""
  );
}}
reportFocus(workspace.{active});
"#,
        bus = bus_name,
        path = object_path,
        iface = KDE_QUERY_INTERFACE,
        method = KDE_QUERY_METHOD,
        active = active_window
    )
}

/// Build the KWin focus-push script body. Targets the per-instance daemon
/// bus name so multiple daemons coexist on KDE with isolated push channels.
pub(crate) fn build_kde_focus_push_script(bus_name: &str, api: &str, active_window: &str) -> String {
    format!(
        r#"function notifyFocus(client) {{
  callDBus(
    "{bus}",
    "{path}",
    "{iface}",
    "WindowFocus",
    client ? (client.resourceClass || "") : "",
    client ? (client.caption || "") : ""
  );
}}
workspace.{api}.connect(notifyFocus);
notifyFocus(workspace.{active});
"#,
        bus = bus_name,
        path = DBUS_PATH,
        iface = DBUS_INTERFACE,
        api = api,
        active = active_window
    )
}

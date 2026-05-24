use super::*;

pub(crate) fn start_wayland_test_server() -> (
    std::sync::MutexGuard<'static, ()>,
    super::super::wayland::wayland_mock::WaylandMockServer,
) {
    let lock = WAYLAND_ENV_LOCK.lock().unwrap();
    let server = super::super::wayland::wayland_mock::WaylandMockServer::start();
    (lock, server)
}

pub(crate) async fn pause_daemon_direct(
    pause_broadcaster: &PauseBroadcaster,
    handler: &Arc<Mutex<FocusHandler>>,
    status_broadcaster: &StatusBroadcaster,
    kanata: &KanataClient,
    request_label: &str,
) {
    if !pause_broadcaster.set_paused(true) {
        println!("[Pause] Pause requested {} (already paused)", request_label);
        return;
    }
    println!("[Pause] Pausing daemon");
    let virtual_keys = {
        let mut handler = handler.lock().unwrap();
        let keys = handler.current_virtual_keys();
        handler.reset();
        keys
    };
    let default_layer = kanata.default_layer().await.unwrap_or_default();

    for vk in virtual_keys.iter().rev() {
        kanata.act_on_fake_key(vk, "Release").await;
    }

    if !default_layer.is_empty() {
        let _ = kanata.change_layer(&default_layer).await;
    }

    status_broadcaster.set_paused_status(default_layer);
    kanata.pause_disconnect().await;
}

pub(crate) async fn unpause_daemon_direct(
    env: Environment,
    connection: Option<Connection>,
    is_kde6: bool,
    pause_broadcaster: &PauseBroadcaster,
    handler: &Arc<Mutex<FocusHandler>>,
    status_broadcaster: &StatusBroadcaster,
    kanata: &KanataClient,
    request_label: &str,
) {
    if !pause_broadcaster.set_paused(false) {
        println!(
            "[Pause] Unpause requested {} (already running)",
            request_label
        );
        return;
    }
    println!("[Pause] Resuming daemon");
    kanata.unpause_connect().await;
    if let Err(error) = apply_focus_for_env(
        env,
        connection.as_ref(),
        is_kde6,
        handler,
        status_broadcaster,
        pause_broadcaster,
        kanata,
    )
    .await
    {
        panic!("[Pause] Failed to refresh focus after unpause: {}", error);
    }
}

pub(crate) struct FocusService {
    pub(crate) call_count: Arc<std::sync::atomic::AtomicUsize>,
    pub(crate) class: String,
    pub(crate) title: String,
}

#[zbus::interface(name = "com.github.kanata.Switcher.extensions.GNOME")]
impl FocusService {
    #[allow(non_snake_case)]
    fn GetFocus(&self) -> (String, String) {
        self.call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        (self.class.clone(), self.title.clone())
    }

    #[zbus(signal, name = "FocusChanged")]
    pub async fn focus_changed(
        signal_emitter: &zbus::object_server::SignalEmitter<'_>,
        class: &str,
        title: &str,
    ) -> zbus::Result<()>;
}

/// Bus name used by integration tests when registering the daemon's control
/// service on a private session bus. All test sites that previously hardcoded
/// `com.github.kanata.Switcher` should use this constant so the per-instance
/// name plumbing is exercised consistently.
pub(crate) const TEST_DAEMON_DBUS_NAME: &str = "com.github.kanata.Switcher.instances.test";

pub(crate) async fn start_gnome_focus_service(
    address: &zbus::Address,
    class: &str,
    title: &str,
) -> (Connection, Arc<std::sync::atomic::AtomicUsize>) {
    use zbus::connection::Builder;

    let call_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let service_connection = Builder::address(address.clone())
        .expect("Failed to create connection builder")
        .name(GNOME_SHELL_BUS_NAME)
        .expect("Failed to set bus name")
        .serve_at(
            GNOME_FOCUS_OBJECT_PATH,
            FocusService {
                call_count: call_count.clone(),
                class: class.to_string(),
                title: title.to_string(),
            },
        )
        .expect("Failed to serve mock focus service")
        .build()
        .await
        .expect("Failed to build focus service connection");

    let dbus_proxy = zbus::fdo::DBusProxy::new(&service_connection)
        .await
        .expect("Failed to create DBus proxy");
    wait_for_async(|| {
        let proxy = dbus_proxy.clone();
        async move {
            proxy
                .name_has_owner(GNOME_SHELL_BUS_NAME.try_into().unwrap())
                .await
                .ok()
                .filter(|&has_owner| has_owner)
        }
    })
    .await
    .expect("Timeout waiting for GNOME focus service registration");

    (service_connection, call_count)
}

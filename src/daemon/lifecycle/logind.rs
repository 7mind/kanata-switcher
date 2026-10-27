use crate::constants::*;
use crate::environ::{LifecycleSnapshot, session_type_to_session_kind};
use crate::errors::DynError;
use crate::lifecycle::snapshot::snapshot_no_session;
use futures_util::StreamExt;
use tokio::sync::mpsc;
use zbus::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Str, Structure, Value};

pub(crate) async fn resolve_logind_session_path(
    connection: &Connection,
) -> Result<OwnedObjectPath, LogindSessionPathResolutionError> {
    let manager = zbus::Proxy::new(
        connection,
        LOGIND_BUS_NAME,
        LOGIND_MANAGER_PATH,
        LOGIND_MANAGER_INTERFACE,
    )
    .await
    .map_err(LogindSessionPathResolutionError::fatal)?;

    if let Ok(session_id) = std::env::var("XDG_SESSION_ID") {
        println!("[Logind] Using XDG_SESSION_ID={}", session_id);
        let reply = manager
            .call_method("GetSession", &(session_id))
            .await
            .map_err(LogindSessionPathResolutionError::fatal)?;
        let path = decode_logind_object_path_reply(&reply, "GetSession")
            .map_err(LogindSessionPathResolutionError::fatal)?;
        println!("[Logind] Using session path: {}", path.as_str());
        return Ok(path);
    }
    println!("[Logind] XDG_SESSION_ID not set; resolving session via logind");

    let pid = std::process::id();
    match manager.call_method("GetSessionByPID", &(pid)).await {
        Ok(reply) => {
            let path = decode_logind_object_path_reply(&reply, "GetSessionByPID")
                .map_err(LogindSessionPathResolutionError::fatal)?;
            println!("[Logind] Using session path: {}", path.as_str());
            Ok(path)
        }
        Err(error) => {
            if is_logind_no_session_error(&error) {
                return resolve_logind_display_session_path(&manager, connection, pid).await;
            }
            Err(LogindSessionPathResolutionError::fatal(error))
        }
    }
}

pub(crate) fn is_logind_no_session_error(error: &zbus::Error) -> bool {
    match error {
        zbus::Error::MethodError(name, _, _) => name.as_ref() == LOGIND_ERROR_NO_SESSION_FOR_PID,
        _ => false,
    }
}

pub(crate) enum LogindSessionPathResolutionError {
    DisplayNotReady,
    Fatal(Box<dyn std::error::Error + Send + Sync>),
}

impl LogindSessionPathResolutionError {
    fn fatal<E>(error: E) -> Self
    where
        E: Into<Box<dyn std::error::Error + Send + Sync>>,
    {
        Self::Fatal(error.into())
    }
}

pub(crate) fn is_logind_empty_object_path(path: &OwnedObjectPath) -> bool {
    path.as_str() == LOGIND_EMPTY_OBJECT_PATH
}

pub(crate) fn parse_logind_object_path(
    value: OwnedValue,
    context: &str,
) -> Result<OwnedObjectPath, Box<dyn std::error::Error + Send + Sync>> {
    let debug_value = format!("{:?}", value);
    if let Ok(path) = OwnedObjectPath::try_from(value.try_clone()?) {
        return Ok(path);
    }
    if let Ok(structure) = Structure::try_from(value.try_clone()?) {
        if let Some(path) = parse_logind_object_path_from_structure(&structure) {
            return Ok(path);
        }
    }
    if let Ok(text) = String::try_from(value) {
        return OwnedObjectPath::try_from(text).map_err(|error| {
            format!(
                "logind {} returned invalid object path string: {}",
                context, error
            )
            .into()
        });
    }
    Err(format!(
        "logind {} returned unexpected value: {}",
        context, debug_value
    )
    .into())
}

pub(crate) fn parse_logind_object_path_from_structure(structure: &Structure<'_>) -> Option<OwnedObjectPath> {
    let fields = structure.fields();
    if fields.is_empty() {
        return None;
    }
    fields
        .iter()
        .find_map(|field| logind_object_path_from_value(field))
}

pub(crate) fn decode_logind_object_path_reply(
    reply: &zbus::Message,
    context: &str,
) -> Result<OwnedObjectPath, Box<dyn std::error::Error + Send + Sync>> {
    let body = reply.body();
    let signature = body.signature().to_string();
    match signature.as_str() {
        "o" => Ok(body.deserialize_unchecked::<OwnedObjectPath>()?),
        "s" => {
            let text = body.deserialize_unchecked::<String>()?;
            OwnedObjectPath::try_from(text).map_err(|error| {
                format!(
                    "logind {} returned invalid object path string: {}",
                    context, error
                )
                .into()
            })
        }
        "v" => {
            let value = body.deserialize::<OwnedValue>()?;
            parse_logind_object_path(value, context)
        }
        _ => {
            if signature.starts_with('(') {
                let structure = body.deserialize::<Structure>()?;
                return parse_logind_object_path_from_structure(&structure).ok_or_else(|| {
                    format!(
                        "logind {} returned unexpected structure: {}",
                        context, signature
                    )
                    .into()
                });
            }
            Err(format!(
                "logind {} returned unexpected signature: {}",
                context, signature
            )
            .into())
        }
    }
}

pub(crate) fn logind_object_path_from_value(value: &Value<'_>) -> Option<OwnedObjectPath> {
    match value {
        Value::ObjectPath(path) => Some(OwnedObjectPath::from(path.clone())),
        Value::Str(text) => OwnedObjectPath::try_from(text.as_str()).ok(),
        Value::Structure(structure) => parse_logind_object_path_from_structure(structure),
        Value::Value(inner) => logind_object_path_from_value(inner),
        _ => None,
    }
}

pub(crate) async fn resolve_logind_display_session_path(
    manager: &zbus::Proxy<'_>,
    connection: &Connection,
    pid: u32,
) -> Result<OwnedObjectPath, LogindSessionPathResolutionError> {
    let user_reply = manager
        .call_method("GetUserByPID", &(pid))
        .await
        .map_err(LogindSessionPathResolutionError::fatal)?;
    let user_path = decode_logind_object_path_reply(&user_reply, "GetUserByPID")
        .map_err(LogindSessionPathResolutionError::fatal)?;
    let user_proxy = zbus::Proxy::new(
        connection,
        LOGIND_BUS_NAME,
        user_path.clone(),
        LOGIND_USER_INTERFACE,
    )
    .await
    .map_err(LogindSessionPathResolutionError::fatal)?;
    let display = parse_logind_object_path(
        user_proxy
            .get_property::<OwnedValue>("Display")
            .await
            .map_err(LogindSessionPathResolutionError::fatal)?,
        "User.Display",
    )
    .map_err(LogindSessionPathResolutionError::fatal)?;
    if is_logind_empty_object_path(&display) {
        return Err(LogindSessionPathResolutionError::DisplayNotReady);
    }
    println!("[Logind] Using display session path: {}", display.as_str());
    Ok(display)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LogindDisplayPathChange {
    Unchanged,
    Empty,
    Path(OwnedObjectPath),
}

pub(crate) fn decode_logind_display_path_change(
    value: Option<&Value<'_>>,
) -> Result<LogindDisplayPathChange, String> {
    let Some(value) = value else {
        return Ok(LogindDisplayPathChange::Unchanged);
    };
    let path = logind_object_path_from_value(value).ok_or_else(|| {
        "[Lifecycle] Failed to parse logind User.Display property change".to_string()
    })?;
    if is_logind_empty_object_path(&path) {
        return Ok(LogindDisplayPathChange::Empty);
    }
    Ok(LogindDisplayPathChange::Path(path))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LogindDisplayChangeAction {
    Ignore,
    DetachSessionMonitor,
    EmitNoSessionAndDetach(LifecycleSnapshot),
    Reattach(OwnedObjectPath),
}

pub(crate) fn apply_logind_display_change(
    current_session_path: &OwnedObjectPath,
    session_monitor_attached: bool,
    last_active: bool,
    last_type: &str,
    change: LogindDisplayPathChange,
) -> LogindDisplayChangeAction {
    match change {
        LogindDisplayPathChange::Unchanged => LogindDisplayChangeAction::Ignore,
        LogindDisplayPathChange::Empty => {
            if last_active || !last_type.is_empty() {
                LogindDisplayChangeAction::EmitNoSessionAndDetach(snapshot_no_session())
            } else if session_monitor_attached {
                LogindDisplayChangeAction::DetachSessionMonitor
            } else {
                LogindDisplayChangeAction::Ignore
            }
        }
        LogindDisplayPathChange::Path(path) => {
            if path == *current_session_path && session_monitor_attached {
                LogindDisplayChangeAction::Ignore
            } else {
                LogindDisplayChangeAction::Reattach(path)
            }
        }
    }
}

pub(crate) async fn wait_for_logind_display_session_path(
    user_proxy: &zbus::Proxy<'_>,
    signals: &mut zbus::fdo::PropertiesChangedStream,
) -> Result<OwnedObjectPath, DynError> {
    println!("[Logind] Waiting for display session to become ready");
    loop {
        let display =
            parse_logind_object_path(user_proxy.get_property("Display").await?, "User.Display")?;
        if !is_logind_empty_object_path(&display) {
            println!("[Logind] Using display session path: {}", display.as_str());
            return Ok(display);
        }

        let signal = expect_some_or_fail_fast(
            signals.next().await,
            "[Lifecycle] logind user properties-changed stream terminated".to_string(),
            fail_fast_lifecycle_monitor,
        );
        let args = expect_or_fail_fast(
            signal.args(),
            |error| format!("[Lifecycle] Failed to decode logind user signal: {}", error),
            fail_fast_lifecycle_monitor,
        );
        let display_change = expect_or_fail_fast(
            decode_logind_display_path_change(args.changed_properties.get("Display")),
            |error| error,
            fail_fast_lifecycle_monitor,
        );
        if let LogindDisplayPathChange::Path(display) = display_change {
            println!("[Logind] Using display session path: {}", display.as_str());
            return Ok(display);
        }
    }
}

#[derive(Debug)]
pub(crate) struct LogindLifecycleProvider {
    pub(crate) receiver: mpsc::UnboundedReceiver<LifecycleSnapshot>,
}

impl LogindLifecycleProvider {
    pub(crate) async fn new() -> Result<Self, DynError> {
        let connection = Connection::system().await?;
        verify_logind_lifecycle_monitor_prerequisites(&connection).await?;
        let (sender, receiver) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            if let Err(error) = monitor_logind_lifecycle(connection, sender).await {
                fail_fast_lifecycle_monitor::<()>(format!(
                    "[Lifecycle] Failed to initialize logind lifecycle monitor: {}",
                    error
                ));
            }
        });

        Ok(Self { receiver })
    }

    pub(crate) async fn next_snapshot(&mut self) -> Option<LifecycleSnapshot> {
        self.receiver.recv().await
    }
}

pub(crate) async fn verify_logind_lifecycle_monitor_prerequisites(
    connection: &Connection,
) -> Result<(), DynError> {
    let manager = zbus::Proxy::new(
        connection,
        LOGIND_BUS_NAME,
        LOGIND_MANAGER_PATH,
        LOGIND_MANAGER_INTERFACE,
    )
    .await?;
    let user_reply = manager
        .call_method("GetUserByPID", &(std::process::id()))
        .await?;
    let user_path = decode_logind_object_path_reply(&user_reply, "GetUserByPID")?;

    let _user_proxy = zbus::Proxy::new(
        connection,
        LOGIND_BUS_NAME,
        user_path.clone(),
        LOGIND_USER_INTERFACE,
    )
    .await?;
    let user_properties_proxy = zbus::fdo::PropertiesProxy::builder(connection)
        .destination(LOGIND_BUS_NAME)?
        .path(user_path)?
        .build()
        .await?;
    let _user_signals = user_properties_proxy.receive_properties_changed().await?;

    match resolve_logind_session_path(connection).await {
        Ok(session_path) => {
            let _ = open_logind_session_monitor(connection, &session_path).await?;
        }
        Err(LogindSessionPathResolutionError::DisplayNotReady) => {}
        Err(LogindSessionPathResolutionError::Fatal(error)) => return Err(error),
    }

    Ok(())
}

pub(crate) fn validate_active_logind_session_type(
    active: bool,
    session_type: &str,
) -> Result<(), &'static str> {
    if active && session_type.trim().is_empty() {
        return Err("[Lifecycle] logind Type property is empty for an active session");
    }
    Ok(())
}

pub(crate) fn fail_fast_lifecycle_monitor<T>(message: String) -> T {
    eprintln!("{}", message);
    std::process::exit(1);
}

pub(crate) fn expect_some_or_fail_fast<T, FF>(value: Option<T>, message: String, fail_fast: FF) -> T
where
    FF: FnOnce(String) -> T,
{
    match value {
        Some(value) => value,
        None => fail_fast(message),
    }
}

pub(crate) fn expect_or_fail_fast<T, E, MF, FF>(result: Result<T, E>, map_error: MF, fail_fast: FF) -> T
where
    MF: FnOnce(E) -> String,
    FF: FnOnce(String) -> T,
{
    match result {
        Ok(value) => value,
        Err(error) => fail_fast(map_error(error)),
    }
}

pub(crate) async fn monitor_logind_lifecycle(
    connection: Connection,
    sender: mpsc::UnboundedSender<LifecycleSnapshot>,
) -> Result<(), DynError> {
    let manager = zbus::Proxy::new(
        &connection,
        LOGIND_BUS_NAME,
        LOGIND_MANAGER_PATH,
        LOGIND_MANAGER_INTERFACE,
    )
    .await?;
    let user_reply = manager
        .call_method("GetUserByPID", &(std::process::id()))
        .await?;
    let user_path = decode_logind_object_path_reply(&user_reply, "GetUserByPID")?;

    let user_proxy = zbus::Proxy::new(
        &connection,
        LOGIND_BUS_NAME,
        user_path.clone(),
        LOGIND_USER_INTERFACE,
    )
    .await?;
    let user_properties_proxy = zbus::fdo::PropertiesProxy::builder(&connection)
        .destination(LOGIND_BUS_NAME)?
        .path(user_path)?
        .build()
        .await?;
    let mut user_signals = user_properties_proxy.receive_properties_changed().await?;

    let mut session_path = match resolve_logind_session_path(&connection).await {
        Ok(path) => path,
        Err(LogindSessionPathResolutionError::DisplayNotReady) => {
            wait_for_logind_display_session_path(&user_proxy, &mut user_signals).await?
        }
        Err(LogindSessionPathResolutionError::Fatal(error)) => return Err(error),
    };

    let (initial, session_signals) =
        open_logind_session_monitor(&connection, &session_path).await?;
    let mut session_signals = Some(session_signals);
    let mut last_active = initial.active;
    let mut last_type = initial.session_type.clone();
    sender
        .send(initial)
        .expect("lifecycle receiver dropped during provider init");

    loop {
        tokio::select! {
            user_signal = user_signals.next() => {
                let signal = expect_some_or_fail_fast(
                    user_signal,
                    "[Lifecycle] logind user properties-changed stream terminated".to_string(),
                    fail_fast_lifecycle_monitor,
                );
                let args = expect_or_fail_fast(
                    signal.args(),
                    |error| format!("[Lifecycle] Failed to decode logind user signal: {}", error),
                    fail_fast_lifecycle_monitor,
                );
                let change = expect_or_fail_fast(
                    decode_logind_display_path_change(args.changed_properties.get("Display")),
                    |error| error,
                    fail_fast_lifecycle_monitor,
                );
                match apply_logind_display_change(
                    &session_path,
                    session_signals.is_some(),
                    last_active,
                    &last_type,
                    change,
                ) {
                    LogindDisplayChangeAction::Ignore => {}
                    LogindDisplayChangeAction::DetachSessionMonitor => {
                        session_signals = None;
                    }
                    LogindDisplayChangeAction::EmitNoSessionAndDetach(snapshot) => {
                        session_signals = None;
                        last_active = snapshot.active;
                        last_type = snapshot.session_type.clone();
                        if sender.send(snapshot).is_err() {
                            return Ok(());
                        }
                    }
                    LogindDisplayChangeAction::Reattach(next_session_path) => {
                        println!(
                            "[Logind] Reattaching lifecycle monitor to display session path: {}",
                            next_session_path.as_str()
                        );
                        let (snapshot, next_signals) =
                            open_logind_session_monitor(&connection, &next_session_path).await?;
                        session_path = next_session_path;
                        session_signals = Some(next_signals);
                        last_active = snapshot.active;
                        last_type = snapshot.session_type.clone();
                        if sender.send(snapshot).is_err() {
                            return Ok(());
                        }
                    }
                }
            }
            session_signal = async {
                match session_signals.as_mut() {
                    Some(signals) => signals.next().await,
                    None => std::future::pending().await,
                }
            } => {
                let signal = expect_some_or_fail_fast(
                    session_signal,
                    "[Lifecycle] logind properties-changed stream terminated".to_string(),
                    fail_fast_lifecycle_monitor,
                );
                let args = expect_or_fail_fast(
                    signal.args(),
                    |error| format!("[Lifecycle] Failed to decode logind signal: {}", error),
                    fail_fast_lifecycle_monitor,
                );
                let snapshot = expect_or_fail_fast(
                    decode_logind_lifecycle_snapshot_change(
                        last_active,
                        &last_type,
                        args.changed_properties.get("Active"),
                        args.changed_properties.get("Type"),
                    ),
                    |error| error,
                    fail_fast_lifecycle_monitor,
                );
                let Some(snapshot) = snapshot else {
                    continue;
                };
                last_active = snapshot.active;
                last_type = snapshot.session_type.clone();
                if sender.send(snapshot).is_err() {
                    return Ok(());
                }
            }
        }
    }
}

pub(crate) async fn open_logind_session_monitor(
    connection: &Connection,
    session_path: &OwnedObjectPath,
) -> Result<(LifecycleSnapshot, zbus::fdo::PropertiesChangedStream), DynError> {
    let session_proxy = zbus::Proxy::new(
        &connection,
        LOGIND_BUS_NAME,
        session_path.clone(),
        LOGIND_SESSION_INTERFACE,
    )
    .await?;
    let active: bool = session_proxy.get_property("Active").await?;
    let session_type: String = session_proxy.get_property("Type").await?;
    validate_active_logind_session_type(active, &session_type).map_err(std::io::Error::other)?;

    let properties_proxy = zbus::fdo::PropertiesProxy::builder(&connection)
        .destination(LOGIND_BUS_NAME)?
        .path(session_path)?
        .build()
        .await?;
    let signals = properties_proxy.receive_properties_changed().await?;
    let initial = LifecycleSnapshot {
        active,
        session_type: session_type.clone(),
        session_kind: session_type_to_session_kind(active, &session_type),
    };
    Ok((initial, signals))
}

pub(crate) fn decode_logind_lifecycle_snapshot_change(
    last_active: bool,
    last_type: &str,
    active_value: Option<&Value<'_>>,
    type_value: Option<&Value<'_>>,
) -> Result<Option<LifecycleSnapshot>, String> {
    let mut next_active = last_active;
    let mut next_type = last_type.to_string();
    let mut changed = false;

    if let Some(value) = active_value {
        let parsed_active = value
            .downcast_ref::<bool>()
            .map_err(|_| "[Lifecycle] Failed to parse logind Active property".to_string())?;
        if parsed_active != last_active {
            next_active = parsed_active;
            changed = true;
        }
    }

    if let Some(value) = type_value {
        let parsed_type = if let Ok(parsed) = value.downcast_ref::<String>() {
            parsed
        } else if let Ok(parsed) = value.downcast_ref::<Str<'_>>() {
            parsed.to_string()
        } else {
            return Err("[Lifecycle] Failed to parse logind Type property".to_string());
        };
        if parsed_type != last_type {
            next_type = parsed_type;
            changed = true;
        }
    }

    if !changed {
        return Ok(None);
    }
    validate_active_logind_session_type(next_active, &next_type)
        .map_err(std::string::ToString::to_string)?;

    Ok(Some(LifecycleSnapshot {
        active: next_active,
        session_type: next_type.clone(),
        session_kind: session_type_to_session_kind(next_active, &next_type),
    }))
}

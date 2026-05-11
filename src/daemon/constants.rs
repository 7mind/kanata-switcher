use std::time::Duration;

pub(crate) const GNOME_EXTENSION_UUID: &str = "kanata-switcher@7mind.io";
pub(crate) const DCONF_FOCUS_ONLY_KEY: &str =
    "/org/gnome/shell/extensions/kanata-switcher/show-focus-layer-only";
/// Namespace root for daemon well-known bus names. Each daemon instance owns
/// a name `{DBUS_BASE_NAME}.{suffix}`. Extensions use a disjoint subtree
/// (`com.github.kanata.Switcher.extensions.*`) for interface/path identifiers
/// but never own a name in our namespace.
pub(crate) const DBUS_BASE_NAME: &str = "com.github.kanata.Switcher.instances";
/// Convenience prefix (DBUS_BASE_NAME + ".") for daemon-bus-name membership tests.
pub(crate) const DAEMON_BUS_NAME_PREFIX: &str = "com.github.kanata.Switcher.instances.";
pub(crate) const DBUS_PATH: &str = "/com/github/kanata/Switcher";
/// Control interface — same literal across all daemon instances. Interface
/// names don't collide across distinct bus-name owners and zbus's
/// `#[interface]` macro requires a literal.
pub(crate) const DBUS_INTERFACE: &str = "com.github.kanata.Switcher";
pub(crate) const GNOME_FOCUS_OBJECT_PATH: &str = "/com/github/kanata/Switcher/extensions/GNOME";
pub(crate) const GNOME_FOCUS_INTERFACE: &str = "com.github.kanata.Switcher.extensions.GNOME";
pub(crate) const GNOME_FOCUS_METHOD: &str = "GetFocus";
pub(crate) const GNOME_FOCUS_SIGNAL: &str = "FocusChanged";
pub(crate) const KDE_QUERY_INTERFACE: &str = "com.github.kanata.Switcher.KdeQuery";
pub(crate) const MAX_DBUS_SUFFIX_LEN: usize = 64;
pub(crate) const KDE_QUERY_METHOD: &str = "Focus";
pub(crate) const LOGIND_BUS_NAME: &str = "org.freedesktop.login1";
pub(crate) const LOGIND_MANAGER_PATH: &str = "/org/freedesktop/login1";
pub(crate) const LOGIND_MANAGER_INTERFACE: &str = "org.freedesktop.login1.Manager";
pub(crate) const LOGIND_SESSION_INTERFACE: &str = "org.freedesktop.login1.Session";
pub(crate) const LOGIND_USER_INTERFACE: &str = "org.freedesktop.login1.User";
pub(crate) const LOGIND_ERROR_NO_SESSION_FOR_PID: &str =
    "org.freedesktop.login1.NoSessionForPID";
pub(crate) const LOGIND_EMPTY_OBJECT_PATH: &str = "/";
pub(crate) const KDE_KWIN_BUS_NAME: &str = "org.kde.KWin";
pub(crate) const KDE_KWIN_SCRIPTING_PATH: &str = "/Scripting";
pub(crate) const KDE_KWIN_SCRIPTING_INTERFACE: &str = "org.kde.kwin.Scripting";
pub(crate) const DBUS_INTROSPECTABLE_INTERFACE: &str = "org.freedesktop.DBus.Introspectable";
pub(crate) const KDE_RUNTIME_QUERY_MODE_MAX_ATTEMPTS: usize = 5;
pub(crate) const KDE_RUNTIME_QUERY_MODE_RETRY_DELAY: Duration = Duration::from_secs(1);

/// Path to GNOME extension source relative to repository root
pub(crate) const GNOME_EXTENSION_SRC_PATH: &str = "src/gnome-extension";
pub(crate) const GNOME_EXTENSION_SCHEMA_FILE: &str =
    "schemas/org.gnome.shell.extensions.kanata-switcher.gschema.xml";
pub(crate) const GNOME_EXTENSION_SCHEMA_COMPILED: &str = "schemas/gschemas.compiled";

// D-Bus coordinates for GNOME Shell Extensions interface
pub(crate) const GNOME_SHELL_BUS_NAME: &str = "org.gnome.Shell";
pub(crate) const GNOME_SHELL_OBJECT_PATH: &str = "/org/gnome/Shell";
pub(crate) const GNOME_SHELL_EXTENSIONS_INTERFACE: &str = "org.gnome.Shell.Extensions";
pub(crate) const DBUS_ERROR_SERVICE_UNKNOWN: &str = "org.freedesktop.DBus.Error.ServiceUnknown";
pub(crate) const DBUS_ERROR_NAME_HAS_NO_OWNER: &str =
    "org.freedesktop.DBus.Error.NameHasNoOwner";
pub(crate) const DBUS_ERROR_UNKNOWN_METHOD: &str = "org.freedesktop.DBus.Error.UnknownMethod";

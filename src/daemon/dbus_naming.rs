use crate::constants::{DAEMON_BUS_NAME_PREFIX, DBUS_BASE_NAME, MAX_DBUS_SUFFIX_LEN};
use crate::errors::DbusSuffixError;

/// Sanitize a suffix to DBus name element rules.
/// Replaces every char not in `[A-Za-z0-9_-]` with `_`. Prepends `_` when the
/// first character is a digit (DBus name elements cannot start with a digit).
/// Rejects empty input and inputs whose final sanitized form exceeds
/// `MAX_DBUS_SUFFIX_LEN` chars (digit-start inputs can grow by 1 after the
/// underscore prepend, so a 64-char digit-start input is rejected).
pub(crate) fn sanitize_dbus_suffix(raw: &str) -> Result<String, DbusSuffixError> {
    if raw.is_empty() {
        return Err(DbusSuffixError::Empty);
    }
    let raw_len = raw.chars().count();
    if raw_len > MAX_DBUS_SUFFIX_LEN {
        return Err(DbusSuffixError::TooLong {
            length: raw_len,
            limit: MAX_DBUS_SUFFIX_LEN,
        });
    }
    let mut out = String::with_capacity(raw.len() + 1);
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out
        .chars()
        .next()
        .map(|c| c.is_ascii_digit())
        .unwrap_or(false)
    {
        out.insert(0, '_');
    }
    let out_len = out.chars().count();
    if out_len > MAX_DBUS_SUFFIX_LEN {
        return Err(DbusSuffixError::TooLong {
            length: out_len,
            limit: MAX_DBUS_SUFFIX_LEN,
        });
    }
    Ok(out)
}

/// Always returns a non-empty sanitized suffix derived from host + port.
/// Default host (`127.0.0.1`) maps to `p<port>`; non-default host maps to
/// `h<sanitized_host>_p<port>`.
pub(crate) fn derive_default_dbus_suffix(host: &str, port: u16) -> String {
    if host == "127.0.0.1" {
        let raw = format!("p{}", port);
        sanitize_dbus_suffix(&raw).expect("derived default suffix is always non-empty")
    } else {
        let raw = format!("h{}_p{}", host, port);
        sanitize_dbus_suffix(&raw).expect("derived default suffix is always non-empty")
    }
}

/// Resolve CLI flag + defaults into the effective suffix. Explicit CLI value
/// (after sanitization) wins; otherwise derived from host/port.
pub(crate) fn resolve_dbus_suffix(
    cli: Option<&str>,
    host: &str,
    port: u16,
) -> Result<String, DbusSuffixError> {
    match cli {
        Some(raw) => sanitize_dbus_suffix(raw),
        None => Ok(derive_default_dbus_suffix(host, port)),
    }
}

/// Compose the per-instance well-known name from a sanitized suffix.
pub(crate) fn effective_dbus_name(suffix: &str) -> String {
    format!("{}.{}", DBUS_BASE_NAME, suffix)
}

/// Test whether `name` is a daemon bus name (in the `instances.*` subtree).
/// The `extensions.*` subtree is disjoint by construction. For ASCII bus names
/// (the only realistic case) byte length and char count coincide.
pub(crate) fn is_daemon_bus_name(name: &str) -> bool {
    name.len() > DAEMON_BUS_NAME_PREFIX.len() && name.starts_with(DAEMON_BUS_NAME_PREFIX)
}

// Pure helpers for multi-instance daemon discovery + indicator presentation.
//
// Every daemon owns a well-known bus name in the
// `com.github.kanata.Switcher.instances.*` subtree. The extension enumerates
// these owners and shows one indicator per daemon.

export const DAEMON_BUS_NAME_PREFIX = 'com.github.kanata.Switcher.instances.';

/// Return the keyboard name encoded in a daemon bus name (substring after the
/// `instances.` prefix). Returns `""` for any non-matching input (defensive —
/// daemon discovery already filters by this prefix).
export function parseKeyboardName(busName) {
  if (typeof busName !== 'string') {
    return '';
  }
  if (!busName.startsWith(DAEMON_BUS_NAME_PREFIX)) {
    return '';
  }
  return busName.slice(DAEMON_BUS_NAME_PREFIX.length);
}

/// Filter a list of session-bus owners down to daemon bus names. Names in the
/// `extensions.*` subtree (or anything else that does not match the
/// `instances.*` prefix exactly) are rejected.
export function filterDaemonNames(busNames) {
  if (!Array.isArray(busNames)) {
    return [];
  }
  return busNames.filter((name) =>
    typeof name === 'string' &&
    name.startsWith(DAEMON_BUS_NAME_PREFIX) &&
    name.length > DAEMON_BUS_NAME_PREFIX.length
  );
}

/// Compose an indicator tooltip from optional keyboard + layer + virtualKeys
/// lines. Lines are joined with `\n`; empty/blank lines are omitted.
/// `virtualKeys` may be a string or array; arrays are comma-joined.
export function composeTooltip({ keyboard, layer, virtualKeys } = {}) {
  const lines = [];
  if (typeof keyboard === 'string' && keyboard.trim().length > 0) {
    lines.push(`Keyboard: ${keyboard}`);
  }
  if (typeof layer === 'string' && layer.trim().length > 0) {
    lines.push(`Layer: ${layer}`);
  }
  if (Array.isArray(virtualKeys)) {
    const joined = virtualKeys
      .filter((vk) => typeof vk === 'string' && vk.length > 0)
      .join(', ');
    if (joined.length > 0) {
      lines.push(`VKs: ${joined}`);
    }
  } else if (typeof virtualKeys === 'string' && virtualKeys.trim().length > 0) {
    lines.push(`VKs: ${virtualKeys}`);
  }
  return lines.join('\n');
}

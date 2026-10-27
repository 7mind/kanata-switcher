import GLib from 'gi://GLib';

function assertEqual(actual, expected, message) {
  if (actual !== expected) {
    throw new Error(`${message}: expected "${expected}", got "${actual}"`);
  }
}

function assertTrue(value, message) {
  if (!value) {
    throw new Error(message);
  }
}

async function main() {
  const srcRoot = GLib.getenv('KANATA_SWITCHER_SRC');
  if (!srcRoot) {
    throw new Error('KANATA_SWITCHER_SRC is not set');
  }

  const modulePath = GLib.build_filenamev([srcRoot, 'src/gnome-extension/focus.js']);
  const moduleUrl = GLib.filename_to_uri(modulePath, null);
  const module = await import(moduleUrl);
  const { extractFocus } = module;

  const empty = extractFocus(null);
  assertEqual(empty.windowClass, '', 'null window class');
  assertEqual(empty.windowTitle, '', 'null window title');

  const stubWin = {
    get_wm_class() { return 'Terminal'; },
    get_title() { return 'bash'; }
  };
  const focus = extractFocus(stubWin);
  assertEqual(focus.windowClass, 'Terminal', 'window class');
  assertEqual(focus.windowTitle, 'bash', 'window title');

  const missing = {
    get_wm_class() { return null; },
    get_title() { return undefined; }
  };
  const missingFocus = extractFocus(missing);
  assertEqual(missingFocus.windowClass, '', 'missing class');
  assertEqual(missingFocus.windowTitle, '', 'missing title');

  // Regression: extension.js declares the FocusChanged signal in its DBus XML.
  const extensionPath = GLib.build_filenamev([
    srcRoot,
    'src/gnome-extension/extension.js'
  ]);
  const [ok, contentsBytes] = GLib.file_get_contents(extensionPath);
  assertTrue(ok, `Failed to read ${extensionPath}`);
  const contents = new TextDecoder('utf-8').decode(contentsBytes);
  assertTrue(
    contents.includes("'/com/github/kanata/Switcher/extensions/GNOME'"),
    'extension.js uses renamed extensions.GNOME path'
  );
  assertTrue(
    contents.includes("'com.github.kanata.Switcher.extensions.GNOME'"),
    'extension.js uses renamed extensions.GNOME interface'
  );
  assertTrue(
    contents.includes('<signal name="FocusChanged">'),
    'extension.js declares FocusChanged signal in DBus XML'
  );
  assertTrue(
    contents.includes("emit_signal(\n      'FocusChanged'"),
    'extension.js emits FocusChanged signal for focus pushes'
  );
  // Regression: no direct Gio.DBus.session.call with a "WindowFocus" method —
  // the extension must push via signal, not method call.
  assertTrue(
    !/Gio\.DBus\.session\.call\([^)]*WindowFocus/.test(contents),
    'extension.js must not call WindowFocus method on the daemon'
  );

  // Regression: GetFocus method is still exposed on the renamed XML.
  assertTrue(
    contents.includes('<method name="GetFocus">'),
    'extension.js still exposes GetFocus method on extensions.GNOME interface'
  );

  // Regression: NameOwnerChanged subscription uses broker-side arg0namespace
  // filtering. Older code path used a DBusProxy + client-side filter, which
  // woke the extension on every name change on the session bus.
  assertTrue(
    contents.includes('signal_subscribe('),
    'extension.js uses Gio.DBus.session.signal_subscribe for NameOwnerChanged'
  );
  assertTrue(
    contents.includes('MATCH_ARG0_NAMESPACE'),
    'extension.js sets MATCH_ARG0_NAMESPACE on NameOwnerChanged subscription'
  );
  assertTrue(
    !/Gio\.DBusProxy\.new_for_bus_sync\([^)]*'org\.freedesktop\.DBus'/.test(contents),
    'extension.js must not subscribe to all NameOwnerChanged via a DBusProxy on org.freedesktop.DBus'
  );
}

main();

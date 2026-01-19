import GLib from 'gi://GLib';

function assertEqual(actual, expected, message) {
  if (actual !== expected) {
    throw new Error(`${message}: expected "${expected}", got "${actual}"`);
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
}

main();

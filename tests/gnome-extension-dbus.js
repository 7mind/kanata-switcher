import GLib from 'gi://GLib';

function assertEqual(actual, expected, message) {
  if (actual !== expected) {
    throw new Error(`${message}: expected "${expected}", got "${actual}"`);
  }
}

function assertThrows(fn, message) {
  let threw = false;
  try {
    fn();
  } catch (error) {
    threw = true;
    if (!error || !error.message.includes(message)) {
      throw new Error(`unexpected error message: ${error}`);
    }
  }
  if (!threw) {
    throw new Error('expected error, but none was thrown');
  }
}

async function main() {
  const srcRoot = GLib.getenv('KANATA_SWITCHER_SRC');
  if (!srcRoot) {
    throw new Error('KANATA_SWITCHER_SRC is not set');
  }

  const modulePath = GLib.build_filenamev([srcRoot, 'src/gnome-extension/dbus.js']);
  const moduleUrl = GLib.filename_to_uri(modulePath, null);
  const module = await import(moduleUrl);
  const { unpackSingleBoolean } = module;

  assertEqual(
    unpackSingleBoolean({ deep_unpack() { return [true]; } }),
    true,
    'unpack boolean'
  );

  assertThrows(
    () => unpackSingleBoolean({ deep_unpack() { return [1]; } }),
    'boolean'
  );
  assertThrows(
    () => unpackSingleBoolean({ deep_unpack() { return true; } }),
    'array'
  );
  assertThrows(
    () => unpackSingleBoolean({ deep_unpack() { return [true, false]; } }),
    '1 element'
  );
}

main();

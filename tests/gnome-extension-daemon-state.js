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

  const modulePath = GLib.build_filenamev([srcRoot, 'src/gnome-extension/daemon-state.js']);
  const moduleUrl = GLib.filename_to_uri(modulePath, null);
  const module = await import(moduleUrl);
  const { disconnectedState, isDaemonOwnerAvailable } = module;

  assertEqual(isDaemonOwnerAvailable(':1.23'), true, 'owner should be valid');
  assertEqual(isDaemonOwnerAvailable(''), false, 'empty owner should be invalid');
  assertEqual(isDaemonOwnerAvailable('   '), false, 'whitespace owner should be invalid');
  assertEqual(isDaemonOwnerAvailable(null), false, 'null owner should be invalid');
  assertThrows(
    () => isDaemonOwnerAvailable(123),
    'string'
  );

  const state = disconnectedState();
  assertEqual(state.status.layer, '', 'disconnected layer should be empty');
  assertEqual(state.status.source, 'external', 'disconnected source should be external');
  assertEqual(state.status.virtualKeys.length, 0, 'disconnected virtual keys should be empty');
  assertEqual(state.focusStatus.layer, '', 'focus layer should be empty');
  assertEqual(state.focusStatus.source, 'focus', 'focus source should be focus');
  assertEqual(state.focusStatus.virtualKeys.length, 0, 'focus virtual keys should be empty');
  assertTrue(state.lastStatus === state.status, 'lastStatus should mirror status');
  assertEqual(state.paused, false, 'paused should be false');
}

main();

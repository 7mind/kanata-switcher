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
  const {
    disconnectedState,
    initialFocusStatusState,
    initialStatusState,
    isDaemonOwnerAvailable
  } = module;

  assertEqual(isDaemonOwnerAvailable(':1.23'), true, 'owner should be valid');
  assertEqual(isDaemonOwnerAvailable(''), false, 'empty owner should be invalid');
  assertEqual(isDaemonOwnerAvailable('   '), false, 'whitespace owner should be invalid');
  assertEqual(isDaemonOwnerAvailable(null), false, 'null owner should be invalid');
  assertThrows(
    () => isDaemonOwnerAvailable(123),
    'string'
  );

  const lastStatus = initialStatusState();
  lastStatus.layer = 'terminal';
  lastStatus.virtualKeys = ['v-terminal-ctl'];

  const focusStatus = initialFocusStatusState();
  focusStatus.layer = 'terminal';
  focusStatus.virtualKeys = ['v-terminal-met'];

  const state = disconnectedState(lastStatus, focusStatus);
  assertEqual(state.status.layer, 'terminal', 'disconnected layer should preserve last status');
  assertEqual(state.status.source, 'external', 'disconnected source should preserve last source');
  assertEqual(state.status.virtualKeys.length, 1, 'disconnected virtual keys should be preserved');
  assertEqual(state.focusStatus.layer, 'terminal', 'focus layer should preserve focus status');
  assertEqual(state.focusStatus.source, 'focus', 'focus source should preserve focus source');
  assertEqual(state.focusStatus.virtualKeys.length, 1, 'focus virtual keys should be preserved');
  assertTrue(state.lastStatus === state.status, 'lastStatus should mirror disconnected status');
  assertTrue(state.status !== lastStatus, 'disconnected status should be a clone');
  assertTrue(state.focusStatus !== focusStatus, 'focus status should be a clone');
  assertEqual(state.paused, false, 'paused should be false');

  assertThrows(
    () => disconnectedState(null, focusStatus),
    'lastStatus must be an object'
  );
  assertThrows(
    () => disconnectedState(lastStatus, null),
    'focusStatus must be an object'
  );
}

main();

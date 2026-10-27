import GLib from 'gi://GLib';

function assertEqual(actual, expected, message) {
  if (actual !== expected) {
    throw new Error(`${message}: expected "${expected}", got "${actual}"`);
  }
}

function assertDeepEqual(actual, expected, message) {
  const a = JSON.stringify(actual);
  const e = JSON.stringify(expected);
  if (a !== e) {
    throw new Error(`${message}: expected ${e}, got ${a}`);
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

  const modulePath = GLib.build_filenamev([
    srcRoot,
    'src/gnome-extension/extension-multiplex.js'
  ]);
  const moduleUrl = GLib.filename_to_uri(modulePath, null);
  const module = await import(moduleUrl);
  const {
    composeTooltip,
    DAEMON_BUS_NAME_PREFIX,
    filterDaemonNames,
    parseKeyboardName
  } = module;

  // parseKeyboardName
  assertEqual(
    parseKeyboardName('com.github.kanata.Switcher.instances.kinesis'),
    'kinesis',
    'parseKeyboardName kinesis'
  );
  assertEqual(
    parseKeyboardName('com.github.kanata.Switcher.instances.p10000'),
    'p10000',
    'parseKeyboardName p10000'
  );
  assertEqual(
    parseKeyboardName('com.github.kanata.Switcher.extensions.GNOME'),
    '',
    'parseKeyboardName rejects extensions subtree'
  );
  assertEqual(parseKeyboardName('com.github.kanata.Switcher'), '', 'parseKeyboardName rejects root');
  assertEqual(parseKeyboardName(null), '', 'parseKeyboardName null');
  assertEqual(parseKeyboardName(123), '', 'parseKeyboardName non-string');

  assertEqual(
    DAEMON_BUS_NAME_PREFIX,
    'com.github.kanata.Switcher.instances.',
    'prefix constant matches plan'
  );

  // filterDaemonNames
  assertDeepEqual(
    filterDaemonNames([
      'com.github.kanata.Switcher.instances.p10000',
      'com.github.kanata.Switcher.instances.kinesis',
      'com.github.kanata.Switcher.extensions.GNOME',
      'com.github.kanata.Switcher.instances',
      'com.github.kanata.Switcher',
      'org.gnome.Shell',
      ''
    ]),
    [
      'com.github.kanata.Switcher.instances.p10000',
      'com.github.kanata.Switcher.instances.kinesis'
    ],
    'filterDaemonNames keeps only instances.* subtree'
  );

  assertDeepEqual(filterDaemonNames('not array'), [], 'filterDaemonNames non-array → []');
  assertDeepEqual(filterDaemonNames(null), [], 'filterDaemonNames null → []');

  // composeTooltip
  assertEqual(
    composeTooltip({ keyboard: 'kinesis', layer: 'browser', virtualKeys: ['vk_browser'] }),
    'Keyboard: kinesis\nLayer: browser\nVKs: vk_browser',
    'composeTooltip with all fields'
  );
  assertEqual(
    composeTooltip({ keyboard: 'kinesis' }),
    'Keyboard: kinesis',
    'composeTooltip with keyboard only'
  );
  assertEqual(
    composeTooltip({ layer: 'browser' }),
    'Layer: browser',
    'composeTooltip with layer only'
  );
  assertEqual(
    composeTooltip({ virtualKeys: ['v1', 'v2'] }),
    'VKs: v1, v2',
    'composeTooltip VK array'
  );
  assertEqual(
    composeTooltip({ keyboard: '', layer: '   ', virtualKeys: [] }),
    '',
    'composeTooltip omits empty/blank lines'
  );
  assertEqual(composeTooltip(), '', 'composeTooltip with no args → empty');

  // Round trip: any name passing filterDaemonNames must yield a non-empty
  // keyboard name from parseKeyboardName.
  for (const name of filterDaemonNames([
    'com.github.kanata.Switcher.instances.a',
    'com.github.kanata.Switcher.instances.framework13'
  ])) {
    const keyboard = parseKeyboardName(name);
    assertTrue(
      keyboard.length > 0,
      `parseKeyboardName(${name}) must return non-empty for a filtered name`
    );
  }
}

main();

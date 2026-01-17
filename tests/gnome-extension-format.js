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

  const formatPath = GLib.build_filenamev([srcRoot, 'src/gnome-extension/format.js']);
  const formatUrl = GLib.filename_to_uri(formatPath, null);
  const module = await import(formatUrl);
  const { formatLayerLetter, formatVirtualKeys, selectStatus } = module;

  assertEqual(formatLayerLetter('base'), 'B', 'layer basic');
  assertEqual(formatLayerLetter('  vim'), 'V', 'layer trim');
  assertEqual(formatLayerLetter(''), '?', 'layer empty');
  assertEqual(formatLayerLetter('   '), '?', 'layer whitespace');
  assertEqual(formatLayerLetter(null), '?', 'layer non-string');

  assertEqual(formatVirtualKeys([]), '', 'vk empty array');
  assertEqual(formatVirtualKeys(['vk_nav']), 'V', 'vk single');
  assertEqual(formatVirtualKeys(['  k']), 'K', 'vk single trim');
  assertEqual(formatVirtualKeys(['', 'vk_nav']), '2', 'vk multiple count');
  assertEqual(
    formatVirtualKeys(['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j']),
    '\u221e',
    'vk overflow'
  );

  const focusStatus = { layer: 'vim', virtualKeys: [], source: 'focus' };
  const lastStatus = { layer: 'browser', virtualKeys: [], source: 'external' };
  assertEqual(selectStatus(true, focusStatus, lastStatus).layer, 'vim', 'select focus status');
  assertEqual(selectStatus(false, focusStatus, lastStatus).layer, 'browser', 'select last status');
}

main();

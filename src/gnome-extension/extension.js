// Kanata Layer Switcher - GNOME Shell Extension
// Push-based: notifies daemon on focus changes via DBus

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import { Extension } from 'resource:///org/gnome/shell/extensions/extension.js';

const DBUS_NAME = 'com.github.kanata.Switcher';
const DBUS_PATH = '/com/github/kanata/Switcher';
const DBUS_INTERFACE = 'com.github.kanata.Switcher';

export default class KanataSwitcherExtension extends Extension {
  enable() {
    this._signalHandlerId = global.display.connect(
      'notify::focus-window',
      () => this._notifyFocus()
    );

    // Handle initial state at boot
    this._notifyFocus();

    console.log('[KanataSwitcher] Extension enabled (push mode)');
  }

  disable() {
    if (this._signalHandlerId) {
      global.display.disconnect(this._signalHandlerId);
      this._signalHandlerId = null;
    }

    console.log('[KanataSwitcher] Extension disabled');
  }

  _notifyFocus() {
    const win = global.display.focus_window;
    const windowClass = win?.get_wm_class() ?? '';
    const windowTitle = win?.get_title() ?? '';

    Gio.DBus.session.call(
      DBUS_NAME,
      DBUS_PATH,
      DBUS_INTERFACE,
      'WindowFocus',
      new GLib.Variant('(ss)', [windowClass, windowTitle]),
      null,
      Gio.DBusCallFlags.NO_AUTO_START,
      -1,
      null,
      null
    );
  }
}

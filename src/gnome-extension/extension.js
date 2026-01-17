// Kanata Layer Switcher - GNOME Shell Extension
// Push-based: notifies daemon on focus changes via DBus

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import St from 'gi://St';
import Clutter from 'gi://Clutter';
import { Extension } from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';

const DBUS_NAME = 'com.github.kanata.Switcher';
const DBUS_PATH = '/com/github/kanata/Switcher';
const DBUS_INTERFACE = 'com.github.kanata.Switcher';

const SETTINGS_KEY_SHOW_ICON = 'show-top-bar-icon';
const MAX_VK_COUNT_DIGIT = 9;
const MIN_MULTI_VK_COUNT = 2;
const INFINITY_SYMBOL = '\u221e';

export default class KanataSwitcherExtension extends Extension {
  enable() {
    this._settings = this.getSettings();
    this._status = {
      layer: '',
      virtualKeys: []
    };

    this._settingsChangedId = this._settings.connect(
      `changed::${SETTINGS_KEY_SHOW_ICON}`,
      () => this._syncIndicator()
    );

    this._daemonProxy = Gio.DBusProxy.new_for_bus_sync(
      Gio.BusType.SESSION,
      Gio.DBusProxyFlags.DO_NOT_AUTO_START,
      null,
      DBUS_NAME,
      DBUS_PATH,
      DBUS_INTERFACE,
      null
    );

    this._daemonProxySignalId = this._daemonProxy.connect(
      'g-signal',
      (_proxy, _sender, signalName, parameters) => {
        if (signalName !== 'StatusChanged') {
          return;
        }
        const [layer, virtualKeys] = parameters.deep_unpack();
        this._setStatus(layer, virtualKeys);
      }
    );

    this._signalHandlerId = global.display.connect(
      'notify::focus-window',
      () => this._notifyFocus()
    );

    // Handle initial state at boot
    this._notifyFocus();
    this._refreshStatusFromDaemon();
    this._syncIndicator();

    console.log('[KanataSwitcher] Extension enabled (push mode)');
  }

  disable() {
    if (this._signalHandlerId) {
      global.display.disconnect(this._signalHandlerId);
      this._signalHandlerId = null;
    }

    if (this._settingsChangedId) {
      this._settings.disconnect(this._settingsChangedId);
      this._settingsChangedId = null;
    }

    if (this._daemonProxySignalId) {
      this._daemonProxy.disconnect(this._daemonProxySignalId);
      this._daemonProxySignalId = null;
    }

    if (this._indicator) {
      this._indicator.destroy();
      this._indicator = null;
      this._layerLabel = null;
      this._vkLabel = null;
    }

    this._daemonProxy = null;
    this._settings = null;

    console.log('[KanataSwitcher] Extension disabled');
  }

  _notifyFocus() {
    const win = global.display.focus_window;
    let windowClass = '';
    let windowTitle = '';

    if (win) {
      const classValue = win.get_wm_class();
      const titleValue = win.get_title();
      if (classValue) {
        windowClass = classValue;
      }
      if (titleValue) {
        windowTitle = titleValue;
      }
    }

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

  _refreshStatusFromDaemon() {
    if (!this._daemonProxy) {
      return;
    }

    try {
      const result = this._daemonProxy.call_sync(
        'GetStatus',
        null,
        Gio.DBusCallFlags.NO_AUTO_START,
        -1,
        null
      );
      const [layer, virtualKeys] = result.deep_unpack();
      this._setStatus(layer, virtualKeys);
    } catch (error) {
      console.error(`[KanataSwitcher] Failed to read status: ${error}`);
    }
  }

  _syncIndicator() {
    const shouldShow = this._settings.get_boolean(SETTINGS_KEY_SHOW_ICON);

    if (shouldShow && !this._indicator) {
      this._createIndicator();
      return;
    }

    if (!shouldShow && this._indicator) {
      this._indicator.destroy();
      this._indicator = null;
      this._layerLabel = null;
      this._vkLabel = null;
    }
  }

  _createIndicator() {
    this._indicator = new PanelMenu.Button(0.0, 'Kanata Switcher', false);
    const box = new St.BoxLayout({
      style_class: 'panel-status-menu-box'
    });

    this._layerLabel = new St.Label({
      text: '?',
      y_align: Clutter.ActorAlign.CENTER
    });

    this._vkLabel = new St.Label({
      text: '',
      y_align: Clutter.ActorAlign.CENTER
    });
    this._vkLabel.set_style('color: #00ffff; padding-left: 2px;');

    box.add_child(this._layerLabel);
    box.add_child(this._vkLabel);
    this._indicator.add_child(box);

    Main.panel.addToStatusArea('kanata-switcher', this._indicator, 0, 'right');
    this._applyStatusToIndicator();
  }

  _setStatus(layer, virtualKeys) {
    this._status = {
      layer,
      virtualKeys
    };
    this._applyStatusToIndicator();
  }

  _applyStatusToIndicator() {
    if (!this._indicator) {
      return;
    }

    const layerText = this._formatLayerLetter(this._status.layer);
    const vkText = this._formatVirtualKeys(this._status.virtualKeys);

    this._layerLabel.set_text(layerText);
    this._vkLabel.set_text(vkText);
    this._vkLabel.visible = vkText.length > 0;
  }

  _formatLayerLetter(layerName) {
    if (!layerName) {
      return '?';
    }

    const trimmed = layerName.trim();
    if (!trimmed) {
      return '?';
    }

    return trimmed[0].toUpperCase();
  }

  _formatVirtualKeys(virtualKeys) {
    const count = Array.isArray(virtualKeys) ? virtualKeys.length : 0;

    if (count === 0) {
      return '';
    }

    if (count === 1) {
      const name = virtualKeys[0];
      if (!name) {
        return '';
      }
      const trimmed = name.trim();
      if (!trimmed) {
        return '';
      }
      return trimmed[0].toUpperCase();
    }

    if (count < MIN_MULTI_VK_COUNT) {
      return '';
    }

    if (count > MAX_VK_COUNT_DIGIT) {
      return INFINITY_SYMBOL;
    }

    return String(count);
  }
}

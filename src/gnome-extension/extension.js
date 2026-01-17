// Kanata Switcher - GNOME Shell Extension
// Push-based: notifies daemon on focus changes via DBus

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import St from 'gi://St';
import Clutter from 'gi://Clutter';
import { Extension } from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';
import { formatLayerLetter, formatVirtualKeys, selectStatus } from './format.js';

const DBUS_NAME = 'com.github.kanata.Switcher';
const DBUS_PATH = '/com/github/kanata/Switcher';
const DBUS_INTERFACE = 'com.github.kanata.Switcher';

const SETTINGS_KEY_SHOW_ICON = 'show-top-bar-icon';
const SETTINGS_KEY_FOCUS_ONLY = 'show-focus-layer-only';
export default class KanataSwitcherExtension extends Extension {
  enable() {
    this._settings = this.getSettings();
    this._status = {
      layer: '',
      virtualKeys: [],
      source: 'external'
    };
    this._focusStatus = {
      layer: '',
      virtualKeys: [],
      source: 'focus'
    };
    this._lastStatus = this._status;
    this._paused = false;
    this._isUpdatingPauseItem = false;

    this._settingsChangedId = this._settings.connect(
      `changed::${SETTINGS_KEY_SHOW_ICON}`,
      () => this._syncIndicator()
    );
    this._settingsFocusOnlyChangedId = this._settings.connect(
      `changed::${SETTINGS_KEY_FOCUS_ONLY}`,
      () => this._applyStatusToIndicator()
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
        if (signalName === 'StatusChanged') {
          const [layer, virtualKeys, source] = parameters.deep_unpack();
          this._setStatus(layer, virtualKeys, source);
        } else if (signalName === 'PausedChanged') {
          const [paused] = parameters.deep_unpack();
          this._setPaused(paused);
        }
      }
    );

    this._signalHandlerId = global.display.connect(
      'notify::focus-window',
      () => this._notifyFocus()
    );

    // Handle initial state at boot
    this._notifyFocus();
    this._refreshStatusFromDaemon();
    this._refreshPausedFromDaemon();
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
    if (this._settingsFocusOnlyChangedId) {
      this._settings.disconnect(this._settingsFocusOnlyChangedId);
      this._settingsFocusOnlyChangedId = null;
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
      this._pauseMenuItem = null;
    }

    this._daemonProxy = null;
    this._settings = null;
    this._paused = false;

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
      const [layer, virtualKeys, source] = result.deep_unpack();
      this._setStatus(layer, virtualKeys, source);
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
    this._pauseMenuItem = new PopupMenu.PopupSwitchMenuItem('Pause', false);
    this._pauseMenuItem.connect('toggled', (_item, state) => {
      if (this._isUpdatingPauseItem) {
        return;
      }
      if (state) {
        this._requestPause();
      } else {
        this._requestUnpause();
      }
    });
    this._indicator.menu.addMenuItem(this._pauseMenuItem);
    this._indicator.menu.addAction('Settings', () => this.openPreferences());
    this._indicator.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());
    this._indicator.menu.addAction('Restart', () => this._requestRestart());
    this._syncPauseMenuItem();
    this._applyStatusToIndicator();
  }

  _setStatus(layer, virtualKeys, source) {
    const nextStatus = {
      layer,
      virtualKeys,
      source
    };
    if (source === 'focus') {
      this._focusStatus = nextStatus;
    }
    this._status = nextStatus;
    this._lastStatus = nextStatus;
    this._applyStatusToIndicator();
  }

  _applyStatusToIndicator() {
    if (!this._indicator) {
      return;
    }

    const showFocusOnly = this._settings.get_boolean(SETTINGS_KEY_FOCUS_ONLY);
    const status = this._paused
      ? this._lastStatus
      : selectStatus(showFocusOnly, this._focusStatus, this._lastStatus);
    const layerText = formatLayerLetter(status.layer);
    const vkText = formatVirtualKeys(status.virtualKeys);

    this._layerLabel.set_text(layerText);
    this._vkLabel.set_text(vkText);
    this._vkLabel.visible = vkText.length > 0;
  }

  _setPaused(paused) {
    this._paused = paused;
    this._syncPauseMenuItem();
    this._applyStatusToIndicator();
  }

  _syncPauseMenuItem() {
    if (!this._pauseMenuItem) {
      return;
    }
    this._isUpdatingPauseItem = true;
    this._pauseMenuItem.setToggleState(this._paused);
    this._isUpdatingPauseItem = false;
  }

  _requestRestart() {
    Gio.DBus.session.call(
      DBUS_NAME,
      DBUS_PATH,
      DBUS_INTERFACE,
      'Restart',
      null,
      null,
      Gio.DBusCallFlags.NO_AUTO_START,
      -1,
      null,
      null
    );
  }

  _requestPause() {
    Gio.DBus.session.call(
      DBUS_NAME,
      DBUS_PATH,
      DBUS_INTERFACE,
      'Pause',
      null,
      null,
      Gio.DBusCallFlags.NO_AUTO_START,
      -1,
      null,
      null
    );
  }

  _requestUnpause() {
    Gio.DBus.session.call(
      DBUS_NAME,
      DBUS_PATH,
      DBUS_INTERFACE,
      'Unpause',
      null,
      null,
      Gio.DBusCallFlags.NO_AUTO_START,
      -1,
      null,
      null
    );
  }

  _refreshPausedFromDaemon() {
    if (!this._daemonProxy) {
      return;
    }

    try {
      const result = this._daemonProxy.call_sync(
        'GetPaused',
        null,
        Gio.DBusCallFlags.NO_AUTO_START,
        -1,
        null
      );
      const paused = result.deep_unpack();
      this._setPaused(paused);
    } catch (error) {
      console.error(`[KanataSwitcher] Failed to read pause state: ${error}`);
    }
  }
}

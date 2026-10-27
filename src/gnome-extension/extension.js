// Kanata Switcher - GNOME Shell Extension
//
// Multi-instance: enumerates daemons in the
// `com.github.kanata.Switcher.instances.*` subtree, shows one top-bar
// indicator per daemon, and emits `FocusChanged` signals (one emitter, N
// receivers) to all daemons that subscribe.

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import St from 'gi://St';
import Clutter from 'gi://Clutter';
import { Extension } from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';
import { formatLayerLetter, formatVirtualKeys, selectStatus } from './format.js';
import { unpackSingleBoolean } from './dbus.js';
import {
  disconnectedState,
  initialFocusStatusState,
  initialStatusState,
  isDaemonOwnerAvailable
} from './daemon-state.js';
import { extractFocus } from './focus.js';
import {
  composeTooltip,
  DAEMON_BUS_NAME_PREFIX,
  filterDaemonNames,
  parseKeyboardName
} from './extension-multiplex.js';

const DBUS_PATH = '/com/github/kanata/Switcher';
const DBUS_INTERFACE = 'com.github.kanata.Switcher';
const FOCUS_DBUS_PATH = '/com/github/kanata/Switcher/extensions/GNOME';
const FOCUS_DBUS_INTERFACE = 'com.github.kanata.Switcher.extensions.GNOME';
// arg0namespace bus-name filter passed to `signal_subscribe`. DBus matches any
// arg0 equal to this string or beginning with `<this>.` — exactly the daemon
// `instances.*` subtree.
const DAEMON_BUS_NAME_NAMESPACE = 'com.github.kanata.Switcher.instances';
const FOCUS_DBUS_XML = `
  <node>
    <interface name="${FOCUS_DBUS_INTERFACE}">
      <method name="GetFocus">
        <arg type="s" direction="out" name="class"/>
        <arg type="s" direction="out" name="title"/>
      </method>
      <signal name="FocusChanged">
        <arg type="s" name="class"/>
        <arg type="s" name="title"/>
      </signal>
    </interface>
  </node>
`;
const DAEMON_RECONNECT_POLL_INTERVAL_MS = 1000;

const SETTINGS_KEY_SHOW_ICON = 'show-top-bar-icon';
const SETTINGS_KEY_FOCUS_ONLY = 'show-focus-layer-only';

export default class KanataSwitcherExtension extends Extension {
  enable() {
    this._settings = this.getSettings();
    this._entries = new Map();

    this._settingsChangedId = this._settings.connect(
      `changed::${SETTINGS_KEY_SHOW_ICON}`,
      () => this._applySettingsToAllEntries()
    );
    this._settingsFocusOnlyChangedId = this._settings.connect(
      `changed::${SETTINGS_KEY_FOCUS_ONLY}`,
      () => this._applySettingsToAllEntries()
    );

    this._focusDbus = Gio.DBusExportedObject.wrapJSObject(FOCUS_DBUS_XML, this);
    this._focusDbus.export(Gio.DBus.session, FOCUS_DBUS_PATH);

    this._signalHandlerId = global.display.connect(
      'notify::focus-window',
      () => this._notifyFocus()
    );

    // Server-side arg0namespace filter — broker delivers only NameOwnerChanged
    // signals whose first arg equals `DAEMON_BUS_NAME_NAMESPACE` or starts
    // with `<namespace>.` (i.e., daemon `instances.*` bus names). Removes the
    // wakeup cost of unrelated name churn on busy session buses.
    this._nameOwnerChangedId = Gio.DBus.session.signal_subscribe(
      'org.freedesktop.DBus',
      'org.freedesktop.DBus',
      'NameOwnerChanged',
      '/org/freedesktop/DBus',
      DAEMON_BUS_NAME_NAMESPACE,
      Gio.DBusSignalFlags.MATCH_ARG0_NAMESPACE,
      (_connection, _sender, _path, _iface, _signal, parameters) => {
        const [name, oldOwner, newOwner] = parameters.deep_unpack();
        // arg0namespace also matches the bare `instances` namespace itself
        // (an unowned name our daemons never register); filterDaemonNames
        // rejects names without a trailing-component suffix.
        if (!name || !name.startsWith(DAEMON_BUS_NAME_PREFIX)) {
          return;
        }
        if (newOwner && newOwner.length > 0) {
          this._addOrRefreshEntry(name);
        } else if (oldOwner && oldOwner.length > 0) {
          this._removeEntry(name);
        }
      }
    );

    this._enumerateInitialDaemons();

    // Initial focus push to anyone listening
    this._notifyFocus();

    console.log('[KanataSwitcher] Extension enabled (multi-indicator)');
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

    if (this._nameOwnerChangedId) {
      Gio.DBus.session.signal_unsubscribe(this._nameOwnerChangedId);
      this._nameOwnerChangedId = 0;
    }

    if (this._focusDbus) {
      this._focusDbus.flush();
      this._focusDbus.unexport();
      this._focusDbus = null;
    }

    if (this._entries) {
      for (const entry of this._entries.values()) {
        this._teardownEntry(entry);
      }
      this._entries.clear();
      this._entries = null;
    }

    this._settings = null;

    console.log('[KanataSwitcher] Extension disabled');
  }

  _enumerateInitialDaemons() {
    try {
      const result = Gio.DBus.session.call_sync(
        'org.freedesktop.DBus',
        '/org/freedesktop/DBus',
        'org.freedesktop.DBus',
        'ListNames',
        null,
        new GLib.VariantType('(as)'),
        Gio.DBusCallFlags.NONE,
        -1,
        null
      );
      const [names] = result.deep_unpack();
      for (const name of filterDaemonNames(names)) {
        this._addOrRefreshEntry(name);
      }
    } catch (error) {
      console.error(`[KanataSwitcher] Failed to enumerate session-bus names: ${error}`);
    }
  }

  _addOrRefreshEntry(busName) {
    if (!this._entries) {
      return;
    }
    const existing = this._entries.get(busName);
    if (existing) {
      this._refreshStatusFromDaemon(existing);
      this._refreshPausedFromDaemon(existing);
      return;
    }
    const entry = this._buildEntry(busName);
    this._entries.set(busName, entry);
    this._applySettingsToEntry(entry);
    this._refreshStatusFromDaemon(entry);
    this._refreshPausedFromDaemon(entry);
  }

  _removeEntry(busName) {
    if (!this._entries) {
      return;
    }
    const entry = this._entries.get(busName);
    if (!entry) {
      return;
    }
    this._teardownEntry(entry);
    this._entries.delete(busName);
  }

  _buildEntry(busName) {
    const keyboard = parseKeyboardName(busName);
    const indicatorRoleName = `kanata-switcher::${keyboard || 'default'}`;
    const proxy = Gio.DBusProxy.new_for_bus_sync(
      Gio.BusType.SESSION,
      Gio.DBusProxyFlags.DO_NOT_AUTO_START,
      null,
      busName,
      DBUS_PATH,
      DBUS_INTERFACE,
      null
    );
    const entry = {
      busName,
      keyboard,
      proxy,
      status: initialStatusState(),
      focusStatus: initialFocusStatusState(),
      lastStatus: initialStatusState(),
      paused: false,
      isUpdatingPauseItem: false,
      indicator: null,
      layerLabel: null,
      vkLabel: null,
      pauseMenuItem: null,
      signalId: 0,
      ownerChangedId: 0,
      reconnectProbeId: 0,
      indicatorRoleName
    };
    entry.signalId = proxy.connect('g-signal', (_p, _sender, signalName, parameters) => {
      if (signalName === 'StatusChanged') {
        const [layer, virtualKeys, source] = parameters.deep_unpack();
        this._setStatus(entry, layer, virtualKeys, source);
      } else if (signalName === 'PausedChanged') {
        const [paused] = parameters.deep_unpack();
        this._setPaused(entry, paused);
      }
    });
    entry.ownerChangedId = proxy.connect('notify::g-name-owner', () =>
      this._onDaemonOwnerChanged(entry)
    );
    return entry;
  }

  _teardownEntry(entry) {
    if (entry.signalId && entry.proxy) {
      entry.proxy.disconnect(entry.signalId);
      entry.signalId = 0;
    }
    if (entry.ownerChangedId && entry.proxy) {
      entry.proxy.disconnect(entry.ownerChangedId);
      entry.ownerChangedId = 0;
    }
    this._clearReconnectProbe(entry);
    if (entry.indicator) {
      entry.indicator.destroy();
      entry.indicator = null;
      entry.layerLabel = null;
      entry.vkLabel = null;
      entry.pauseMenuItem = null;
    }
    entry.proxy = null;
  }

  _applySettingsToAllEntries() {
    if (!this._entries) {
      return;
    }
    for (const entry of this._entries.values()) {
      this._applySettingsToEntry(entry);
    }
  }

  _applySettingsToEntry(entry) {
    const shouldShow = this._settings.get_boolean(SETTINGS_KEY_SHOW_ICON);
    if (shouldShow && !entry.indicator) {
      this._createIndicator(entry);
    } else if (!shouldShow && entry.indicator) {
      entry.indicator.destroy();
      entry.indicator = null;
      entry.layerLabel = null;
      entry.vkLabel = null;
      entry.pauseMenuItem = null;
    } else {
      this._applyStatusToIndicator(entry);
    }
  }

  _createIndicator(entry) {
    entry.indicator = new PanelMenu.Button(0.0, entry.indicatorRoleName, false);
    const box = new St.BoxLayout({ style_class: 'panel-status-menu-box' });
    entry.layerLabel = new St.Label({
      text: '?',
      y_align: Clutter.ActorAlign.CENTER
    });
    entry.vkLabel = new St.Label({
      text: '',
      y_align: Clutter.ActorAlign.CENTER
    });
    entry.vkLabel.set_style('color: #00ffff; padding-left: 2px;');
    box.add_child(entry.layerLabel);
    box.add_child(entry.vkLabel);
    entry.indicator.add_child(box);
    Main.panel.addToStatusArea(entry.indicatorRoleName, entry.indicator, 0, 'right');
    entry.pauseMenuItem = new PopupMenu.PopupSwitchMenuItem('Pause', false);
    entry.pauseMenuItem.connect('toggled', (_item, state) => {
      if (entry.isUpdatingPauseItem) {
        return;
      }
      if (state) {
        this._requestPause(entry);
      } else {
        this._requestUnpause(entry);
      }
    });
    entry.indicator.menu.addMenuItem(entry.pauseMenuItem);
    entry.indicator.menu.addAction('Settings', () => this.openPreferences());
    entry.indicator.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());
    entry.indicator.menu.addAction('Restart', () => this._requestRestart(entry));
    this._syncPauseMenuItem(entry);
    this._applyStatusToIndicator(entry);
  }

  _setStatus(entry, layer, virtualKeys, source) {
    const nextStatus = { layer, virtualKeys, source };
    if (source === 'focus') {
      entry.focusStatus = nextStatus;
    }
    entry.status = nextStatus;
    entry.lastStatus = nextStatus;
    this._applyStatusToIndicator(entry);
  }

  _setPaused(entry, paused) {
    entry.paused = paused;
    this._syncPauseMenuItem(entry);
    this._applyStatusToIndicator(entry);
  }

  _syncPauseMenuItem(entry) {
    if (!entry.pauseMenuItem) {
      return;
    }
    entry.isUpdatingPauseItem = true;
    entry.pauseMenuItem.setToggleState(entry.paused);
    entry.isUpdatingPauseItem = false;
  }

  _applyStatusToIndicator(entry) {
    if (!entry.indicator) {
      return;
    }
    const showFocusOnly = this._settings.get_boolean(SETTINGS_KEY_FOCUS_ONLY);
    const status = entry.paused
      ? entry.lastStatus
      : selectStatus(showFocusOnly, entry.focusStatus, entry.lastStatus);
    const layerText = formatLayerLetter(status.layer);
    const vkText = formatVirtualKeys(status.virtualKeys);
    entry.layerLabel.set_text(layerText);
    entry.vkLabel.set_text(vkText);
    entry.vkLabel.visible = vkText.length > 0;
    const tooltip = composeTooltip({
      keyboard: entry.keyboard,
      layer: status.layer,
      virtualKeys: status.virtualKeys
    });
    if (typeof entry.indicator.set_accessible_name === 'function') {
      entry.indicator.set_accessible_name(tooltip);
    }
  }

  _onDaemonOwnerChanged(entry) {
    if (!entry || !entry.proxy) {
      return;
    }
    let owner = null;
    try {
      owner = entry.proxy.get_name_owner();
    } catch (_error) {
      owner = null;
    }
    if (!isDaemonOwnerAvailable(owner)) {
      this._setDisconnected(entry);
      return;
    }
    this._clearReconnectProbe(entry);
    this._refreshStatusFromDaemon(entry);
    this._refreshPausedFromDaemon(entry);
  }

  _setDisconnected(entry) {
    const state = disconnectedState(entry.lastStatus, entry.focusStatus);
    entry.status = state.status;
    entry.focusStatus = state.focusStatus;
    entry.lastStatus = state.lastStatus;
    entry.paused = state.paused;
    this._syncPauseMenuItem(entry);
    this._applyStatusToIndicator(entry);
    this._ensureReconnectProbe(entry);
  }

  _ensureReconnectProbe(entry) {
    if (entry.reconnectProbeId !== 0) {
      return;
    }
    entry.reconnectProbeId = GLib.timeout_add(
      GLib.PRIORITY_DEFAULT,
      DAEMON_RECONNECT_POLL_INTERVAL_MS,
      () => {
        if (!entry || !entry.proxy) {
          entry.reconnectProbeId = 0;
          return GLib.SOURCE_REMOVE;
        }
        let owner = null;
        try {
          owner = entry.proxy.get_name_owner();
        } catch (_error) {
          owner = null;
        }
        if (!isDaemonOwnerAvailable(owner)) {
          return GLib.SOURCE_CONTINUE;
        }
        entry.reconnectProbeId = 0;
        this._refreshStatusFromDaemon(entry);
        this._refreshPausedFromDaemon(entry);
        return GLib.SOURCE_REMOVE;
      }
    );
  }

  _clearReconnectProbe(entry) {
    if (!entry || entry.reconnectProbeId === 0) {
      return;
    }
    GLib.source_remove(entry.reconnectProbeId);
    entry.reconnectProbeId = 0;
  }

  _refreshStatusFromDaemon(entry) {
    if (!entry.proxy) {
      return;
    }
    try {
      const result = entry.proxy.call_sync(
        'GetStatus',
        null,
        Gio.DBusCallFlags.NO_AUTO_START,
        -1,
        null
      );
      const [layer, virtualKeys, source] = result.deep_unpack();
      this._setStatus(entry, layer, virtualKeys, source);
    } catch (error) {
      console.error(
        `[KanataSwitcher] (${entry.busName}) Failed to read status: ${error}`
      );
    }
  }

  _refreshPausedFromDaemon(entry) {
    if (!entry.proxy) {
      return;
    }
    try {
      const result = entry.proxy.call_sync(
        'GetPaused',
        null,
        Gio.DBusCallFlags.NO_AUTO_START,
        -1,
        null
      );
      const paused = unpackSingleBoolean(result);
      this._setPaused(entry, paused);
    } catch (error) {
      console.error(
        `[KanataSwitcher] (${entry.busName}) Failed to read pause state: ${error}`
      );
    }
  }

  _notifyFocus() {
    const { windowClass, windowTitle } = this._currentFocus();
    if (!this._focusDbus) {
      return;
    }
    this._focusDbus.emit_signal(
      'FocusChanged',
      new GLib.Variant('(ss)', [windowClass, windowTitle])
    );
  }

  _currentFocus() {
    return extractFocus(global.display.focus_window);
  }

  GetFocus() {
    const { windowClass, windowTitle } = this._currentFocus();
    return [windowClass, windowTitle];
  }

  _requestRestart(entry) {
    Gio.DBus.session.call(
      entry.busName,
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

  _requestPause(entry) {
    Gio.DBus.session.call(
      entry.busName,
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

  _requestUnpause(entry) {
    Gio.DBus.session.call(
      entry.busName,
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
}

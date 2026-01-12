// Kanata Layer Switcher - GNOME Shell Extension
// Minimal extension that exposes focused window info via DBus

import Gio from 'gi://Gio';
import Shell from 'gi://Shell';
import { Extension } from 'resource:///org/gnome/shell/extensions/extension.js';

export default class KanataSwitcherExtension extends Extension {
  enable() {
    const dbusXml = `
      <node>
        <interface name="com.github.kanata.Switcher">
          <method name="GetFocusedWindow">
            <arg type="s" direction="out" name="window"/>
          </method>
        </interface>
      </node>
    `;

    this._dbus = Gio.DBusExportedObject.wrapJSObject(dbusXml, this);
    this._dbus.export(Gio.DBus.session, '/com/github/kanata/Switcher');

    console.log('[KanataSwitcher] Extension enabled, DBus service exported');
  }

  disable() {
    if (this._dbus) {
      this._dbus.flush();
      this._dbus.unexport();
      this._dbus = null;
    }

    console.log('[KanataSwitcher] Extension disabled');
  }

  GetFocusedWindow() {
    const window = global.display.focus_window;
    if (window) {
      return JSON.stringify({
        class: window.get_wm_class() || '',
        title: window.get_title() || ''
      });
    }
    return JSON.stringify({ class: '', title: '' });
  }
}

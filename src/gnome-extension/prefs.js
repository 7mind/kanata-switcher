import Adw from 'gi://Adw';
import Gio from 'gi://Gio';
import Gtk from 'gi://Gtk';
import { ExtensionPreferences } from 'resource:///org/gnome/Shell/Extensions/js/extensions/prefs.js';

const SETTINGS_KEY_SHOW_ICON = 'show-top-bar-icon';
const SETTINGS_KEY_FOCUS_ONLY = 'show-focus-layer-only';

export default class KanataSwitcherPreferences extends ExtensionPreferences {
  fillPreferencesWindow(window) {
    const settings = this.getSettings();

    const page = new Adw.PreferencesPage({
      title: 'Kanata Switcher'
    });
    const group = new Adw.PreferencesGroup({
      title: 'Top Bar'
    });

    const row = new Adw.ActionRow({
      title: 'Show top bar icon',
      subtitle: 'Display the active layer and virtual key status'
    });

    const toggle = new Gtk.Switch({
      active: settings.get_boolean(SETTINGS_KEY_SHOW_ICON),
      valign: Gtk.Align.CENTER
    });

    settings.bind(
      SETTINGS_KEY_SHOW_ICON,
      toggle,
      'active',
      Gio.SettingsBindFlags.DEFAULT
    );

    row.add_suffix(toggle);
    row.activatable_widget = toggle;
    group.add(row);

    const focusRow = new Adw.ActionRow({
      title: 'Show app layer only',
      subtitle: 'Show the layer from the current app'
    });

    const focusToggle = new Gtk.Switch({
      active: settings.get_boolean(SETTINGS_KEY_FOCUS_ONLY),
      valign: Gtk.Align.CENTER
    });

    settings.bind(
      SETTINGS_KEY_FOCUS_ONLY,
      focusToggle,
      'active',
      Gio.SettingsBindFlags.DEFAULT
    );

    focusRow.add_suffix(focusToggle);
    focusRow.activatable_widget = focusToggle;
    group.add(focusRow);
    page.add(group);
    window.add(page);
  }
}

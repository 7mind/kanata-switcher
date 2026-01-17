import Adw from 'gi://Adw';
import Gio from 'gi://Gio';
import Gtk from 'gi://Gtk';
import { ExtensionPreferences } from 'resource:///org/gnome/shell/extensions/extension.js';

const SETTINGS_KEY_SHOW_ICON = 'show-top-bar-icon';

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
    page.add(group);
    window.add(page);
  }
}

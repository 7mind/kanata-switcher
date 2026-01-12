# GNOME Shell Focused Window Detection - Prior Art

## Summary

We chose **Option B (Intermediary Daemon)** - extension is minimal, daemon handles config/rules/kanata.

## Approaches Found

| Project | Model | IPC |
|---------|-------|-----|
| keymapper | Push | Extension → DBus → client |
| keyd | Push | Extension → FIFO, dynamic KWin injection |
| xremap | Pull | Client polls extension DBus |

## keyd KDE Approach (adopted)

keyd dynamically injects KWin script via DBus - no manual kpackagetool install:
```python
kwin = bus.get_object('org.kde.KWin', '/Scripting')
kwin.unloadScript(path)
num = kwin.loadScript(path)
script = bus.get_object('org.kde.KWin', f'/Scripting/Script{num}')
script.run()
```

## Key APIs

GNOME Shell (45+):
```javascript
global.display.focus_window
window.get_wm_class()
window.get_title()
Gio.DBusExportedObject.wrapJSObject(xml, this)
```

KDE KWin:
```javascript
workspace.windowActivated.connect(handler)  // KDE 6
workspace.clientActivated.connect(handler)  // KDE 5
callDBus(service, path, interface, method, ...args)
```

## Reference Repos

All cloned to `./local/` (gitignored):
- keymapper, keyd, xremap, xremap-gnome, hyprkan, kanata

## Licensing Analysis

| Project | License |
|---------|---------|
| keymapper | GPL-3.0 |
| keyd | MIT |
| xremap | MIT |
| xremap-gnome | GPL-2.0+ |
| kanata | LGPL-3.0 |
| hyprkan | MIT |

### MIT is Defensible

Despite referencing xremap-gnome (GPL-2.0+), this project can use MIT. Rationale:

1. **No code copied** - Our GNOME extension (46 lines) shares no code with xremap-gnome (278 lines)

2. **Copyright protects expression, not ideas** - Studying an approach and implementing independently is legal. GPL triggers only on distributing derivative works, which requires copying protected expression.

3. **API-dictated pattern** - The DBus service export pattern is dictated by GNOME Shell's extension API:
   ```javascript
   Gio.DBusExportedObject.wrapJSObject(xml, this)  // Only way to export DBus
   global.display.focus_window.get_wm_class()      // Only API for window info
   ```

4. **Merger doctrine** - When idea and expression merge (only one way to do something), expression isn't copyrightable

5. **Scènes à faire** - Standard solutions to common problems aren't protectable

### keyd KDE Pattern (MIT)

The KWin script injection approach was adopted from keyd. MIT is permissive - no obligations beyond attribution (optional but courteous).

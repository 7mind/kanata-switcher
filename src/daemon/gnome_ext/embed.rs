use std::fs;
use std::path::Path;
use std::process::Command;
use crate::constants::*;

macro_rules! gnome_ext_file {
    ($file:literal) => {
        concat!("../../../", "src/gnome-extension", "/", $file)
    };
}

pub(crate) const EMBEDDED_EXTENSION_JS: &str = include_str!(gnome_ext_file!("extension.js"));
pub(crate) const EMBEDDED_METADATA_JSON: &str = include_str!(gnome_ext_file!("metadata.json"));
pub(crate) const EMBEDDED_PREFS_JS: &str = include_str!(gnome_ext_file!("prefs.js"));
pub(crate) const EMBEDDED_FORMAT_JS: &str = include_str!(gnome_ext_file!("format.js"));
pub(crate) const EMBEDDED_DBUS_JS: &str = include_str!(gnome_ext_file!("dbus.js"));
pub(crate) const EMBEDDED_FOCUS_JS: &str = include_str!(gnome_ext_file!("focus.js"));
pub(crate) const EMBEDDED_DAEMON_STATE_JS: &str = include_str!(gnome_ext_file!("daemon-state.js"));
pub(crate) const EMBEDDED_MULTIPLEX_JS: &str = include_str!(gnome_ext_file!("extension-multiplex.js"));
pub(crate) const EMBEDDED_GSETTINGS_SCHEMA: &str = include_str!(gnome_ext_file!(
    "schemas/org.gnome.shell.extensions.kanata-switcher.gschema.xml"
));

pub(crate) fn compile_gnome_schemas(dir: &Path) -> std::io::Result<()> {
    let schema_dir = dir.join("schemas");
    let output = Command::new("glib-compile-schemas")
        .arg(&schema_dir)
        .output()?;
    if !output.status.success() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!(
                "glib-compile-schemas failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ),
        ));
    }
    Ok(())
}

pub(crate) fn write_embedded_extension_to_dir(dir: &Path) -> std::io::Result<()> {
    fs::write(dir.join("extension.js"), EMBEDDED_EXTENSION_JS)?;
    fs::write(dir.join("metadata.json"), EMBEDDED_METADATA_JSON)?;
    fs::write(dir.join("prefs.js"), EMBEDDED_PREFS_JS)?;
    fs::write(dir.join("format.js"), EMBEDDED_FORMAT_JS)?;
    fs::write(dir.join("dbus.js"), EMBEDDED_DBUS_JS)?;
    fs::write(dir.join("focus.js"), EMBEDDED_FOCUS_JS)?;
    fs::write(dir.join("daemon-state.js"), EMBEDDED_DAEMON_STATE_JS)?;
    fs::write(dir.join("extension-multiplex.js"), EMBEDDED_MULTIPLEX_JS)?;
    let schema_dir = dir.join("schemas");
    fs::create_dir_all(&schema_dir)?;
    fs::write(
        dir.join(GNOME_EXTENSION_SCHEMA_FILE),
        EMBEDDED_GSETTINGS_SCHEMA,
    )?;
    compile_gnome_schemas(dir)?;
    Ok(())
}

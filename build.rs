use std::env;
use std::fs;
use std::path::Path;

const GNOME_EXTENSION_SRC: &str = "src/gnome-extension";
const GNOME_EXTENSION_FILES: &[&str] = &["extension.js", "metadata.json", "prefs.js"];
const GNOME_EXTENSION_SCHEMA_FILES: &[&str] =
    &["schemas/org.gnome.shell.extensions.kanata-switcher.gschema.xml"];

fn main() {
    for file in GNOME_EXTENSION_FILES {
        println!("cargo:rerun-if-changed={}/{}", GNOME_EXTENSION_SRC, file);
    }
    for file in GNOME_EXTENSION_SCHEMA_FILES {
        println!("cargo:rerun-if-changed={}/{}", GNOME_EXTENSION_SRC, file);
    }

    let out_dir = env::var("OUT_DIR").unwrap();
    let out_path = Path::new(&out_dir);

    // Navigate from OUT_DIR (target/{profile}/build/kanata-switcher-xxx/out)
    // up to target/{profile}/gnome
    // ancestors: out -> kanata-switcher-xxx -> build -> {profile}
    let target_profile_dir = out_path
        .ancestors()
        .nth(3)
        .expect("Could not find target profile directory");
    let target_gnome_dir = target_profile_dir.join("gnome");

    fs::create_dir_all(&target_gnome_dir).expect("Failed to create gnome directory");

    let src_dir = Path::new(GNOME_EXTENSION_SRC);

    for file in GNOME_EXTENSION_FILES {
        let src = src_dir.join(file);
        let dst = target_gnome_dir.join(file);
        fs::copy(&src, &dst).unwrap_or_else(|e| {
            panic!("Failed to copy {} to {}: {}", src.display(), dst.display(), e)
        });
    }

    let target_schema_dir = target_gnome_dir.join("schemas");
    fs::create_dir_all(&target_schema_dir).expect("Failed to create gnome schema directory");
    for file in GNOME_EXTENSION_SCHEMA_FILES {
        let src = src_dir.join(file);
        let dst = target_gnome_dir.join(file);
        fs::copy(&src, &dst).unwrap_or_else(|e| {
            panic!("Failed to copy {} to {}: {}", src.display(), dst.display(), e)
        });
    }

    println!(
        "cargo:warning=GNOME extension files copied to {}",
        target_gnome_dir.display()
    );
}

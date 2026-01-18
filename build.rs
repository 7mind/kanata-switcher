use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

const GNOME_EXTENSION_SRC: &str = "src/gnome-extension";
const GNOME_EXTENSION_FILES: &[&str] = &["metadata.json"];
const GNOME_EXTENSION_SCHEMA_FILES: &[&str] =
    &["schemas/org.gnome.shell.extensions.kanata-switcher.gschema.xml"];

fn main() {
    for file in GNOME_EXTENSION_FILES {
        println!("cargo:rerun-if-changed={}/{}", GNOME_EXTENSION_SRC, file);
    }
    let src_dir = Path::new(GNOME_EXTENSION_SRC);
    let entries = fs::read_dir(src_dir).expect("Failed to read GNOME extension directory");
    for entry in entries {
        let entry = entry.expect("Failed to read GNOME extension directory entry");
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("js") {
            continue;
        }
        println!("cargo:rerun-if-changed={}", path.display());
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

    for file in GNOME_EXTENSION_FILES {
        let src = src_dir.join(file);
        let dst = target_gnome_dir.join(file);
        fs::copy(&src, &dst).unwrap_or_else(|e| {
            panic!("Failed to copy {} to {}: {}", src.display(), dst.display(), e)
        });
    }

    let entries = fs::read_dir(src_dir).expect("Failed to read GNOME extension directory");
    for entry in entries {
        let entry = entry.expect("Failed to read GNOME extension directory entry");
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("js") {
            continue;
        }
        let filename = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("GNOME extension JS filename must be valid UTF-8");
        let dst = target_gnome_dir.join(filename);
        fs::copy(&path, &dst).unwrap_or_else(|e| {
            panic!("Failed to copy {} to {}: {}", path.display(), dst.display(), e)
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

    let compile_result = Command::new("glib-compile-schemas")
        .arg(&target_schema_dir)
        .output()
        .unwrap_or_else(|e| panic!("Failed to run glib-compile-schemas: {}", e));
    if !compile_result.status.success() {
        panic!(
            "glib-compile-schemas failed: {}",
            String::from_utf8_lossy(&compile_result.stderr)
        );
    }

    println!(
        "cargo:warning=GNOME extension files copied to {}",
        target_gnome_dir.display()
    );
}

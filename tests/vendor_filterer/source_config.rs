use camino::{Utf8Path, Utf8PathBuf};

use super::common::{
    tempdir, vendor, verify_crate_is_no_stub, write_file_create_parents, VendorOptions,
};

/// The configuration `cargo vendor` suggests, redirecting crates.io to the
/// vendor directory
const VENDORED_SOURCES_CONFIG: &str = r#"
[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "vendor"
"#;

/// Create a project in `root/project` depending on hex and bitflags, with the
/// cargo configuration `config`.
fn write_project(root: &Utf8Path, config: &str) -> Utf8PathBuf {
    let project = root.join("project");
    write_file_create_parents(
        &project,
        "Cargo.toml",
        r#"
        [package]
        name = "vendored-sources-config-test"
        version = "0.1.0"
        edition = "2021"

        [dependencies]
        bitflags = "1.3"
        hex = "0.4"
    "#,
    )
    .unwrap();
    write_file_create_parents(&project, "src/lib.rs", "").unwrap();
    write_file_create_parents(&project, ".cargo/config.toml", config).unwrap();
    project
}

/// Vendoring must ignore a `.cargo/config.toml` replacing crates.io with the
/// vendor directory that is about to be created, like `cargo vendor` does.
#[test]
fn vendored_sources_config() {
    for (manifest_path, keep_dep_kinds) in [(None, None), (Some("Cargo.toml"), Some("normal"))] {
        let (_td, root) = tempdir().unwrap();
        let project = write_project(&root, VENDORED_SOURCES_CONFIG);
        let output = vendor(VendorOptions {
            current_dir: Some(&project),
            manifest_path: manifest_path.map(Utf8Path::new),
            keep_dep_kinds,
            ..Default::default()
        })
        .unwrap();
        assert!(
            output.status.success(),
            "vendor-filterer failed with manifest path {manifest_path:?}, dependency kinds {keep_dep_kinds:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        verify_crate_is_no_stub(&project.join("vendor"), "hex");
    }
}

/// The rest of such a configuration still applies, and its relative paths
/// still resolve against the project.
#[test]
fn vendored_sources_config_with_patch() {
    let (_td, root) = tempdir().unwrap();
    let project = write_project(
        &root,
        &format!(
            "{VENDORED_SOURCES_CONFIG}\n[patch.crates-io]\nhex = {{ path = \"../hexlocal\" }}\n"
        ),
    );
    let hexlocal = root.join("hexlocal");
    write_file_create_parents(
        &hexlocal,
        "Cargo.toml",
        r#"
        [package]
        name = "hex"
        version = "0.4.99"
        edition = "2021"
    "#,
    )
    .unwrap();
    write_file_create_parents(&hexlocal, "src/lib.rs", "").unwrap();

    let output = vendor(VendorOptions {
        current_dir: Some(&project),
        ..Default::default()
    })
    .unwrap();
    assert!(
        output.status.success(),
        "vendor-filterer failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let vendor_dir = project.join("vendor");
    verify_crate_is_no_stub(&vendor_dir, "bitflags");
    // The local patch isn't vendored
    assert!(!vendor_dir.join("hex").exists());
}

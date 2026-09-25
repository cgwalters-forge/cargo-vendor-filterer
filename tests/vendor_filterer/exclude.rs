use super::common::{targets_supported, tempdir, vendor, verify_no_windows, VendorOptions};

#[test]
#[serial_test::parallel]
fn linux_multiple_platforms() {
    const PLATFORMS: &[&str] = &["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"];
    if !targets_supported(PLATFORMS) {
        return;
    }
    let (_td, mut test_folder) = tempdir().unwrap();
    test_folder.push("vendor");
    let output = vendor(VendorOptions {
        output: Some(&test_folder),
        platforms: Some(PLATFORMS),
        exclude_crate_paths: Some(&["hex#benches", "*#tests"]),
        ..Default::default()
    })
    .unwrap();
    assert!(output.status.success());
    verify_no_windows(&test_folder);
    test_folder.push("hex/benches");
    assert!(!test_folder.exists());
    test_folder.push("../tests");
    assert!(!test_folder.exists());
}

#[test]
#[serial_test::parallel]
fn windows_with_dep_kind_filter_normal() {
    const PLATFORMS: &[&str] = &["x86_64-pc-windows-gnu"];
    if !targets_supported(PLATFORMS) {
        return;
    }
    let (_td, mut test_folder) = tempdir().unwrap();
    test_folder.push("vendor-test2");
    let output = vendor(VendorOptions {
        output: Some(&test_folder),
        platforms: Some(PLATFORMS),
        keep_dep_kinds: Some("normal"),
        ..Default::default()
    })
    .unwrap();
    assert!(output.status.success());
    test_folder.push("serial_test/tests"); // crate replaced with a stub, so tests folder is removed
    assert!(!test_folder.exists());
    test_folder.push("../openssl/examples"); // openssl removed because defined only for non-windows
    assert!(!test_folder.exists());
}

#[test]
#[serial_test::parallel]
fn exclude_with_glob_patterns() {
    const PLATFORMS: &[&str] = &["x86_64-unknown-linux-gnu"];
    if !targets_supported(PLATFORMS) {
        return;
    }
    let (_td, mut test_folder) = tempdir().unwrap();
    test_folder.push("vendor");
    let output = vendor(VendorOptions {
        output: Some(&test_folder),
        platforms: Some(PLATFORMS),
        exclude_crate_paths: Some(&["hex#*.md", "*#benches", "libz-sys#src/*.c"]),
        ..Default::default()
    })
    .unwrap();
    assert!(output.status.success());
    let hex_dir = test_folder.join("hex");
    assert!(hex_dir.exists());
    assert!(!hex_dir.join("README.md").exists());
    assert!(!hex_dir.join("CHANGELOG.md").exists());
    assert!(!hex_dir.join("benches").exists());
    // Check that .c files were removed from libz-sys if it exists
    let libz_sys_dir = test_folder.join("libz-sys");
    if libz_sys_dir.exists() {
        let src_dir = libz_sys_dir.join("src");
        if src_dir.exists() {
            for entry in src_dir.read_dir_utf8().unwrap() {
                let entry = entry.unwrap();
                assert!(entry.path().extension() != Some("c"));
            }
        }
    }
}

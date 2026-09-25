use cargo_vendor_filterer::tiers::Tier;

use super::common::{
    targets_supported, tempdir, vendor, verify_crate_is_no_stub, verify_no_windows, VendorOptions,
};

#[test]
fn linux() {
    const PLATFORMS: &[&str] = &["x86_64-unknown-linux-gnu"];
    if !targets_supported(PLATFORMS) {
        return;
    }
    let (_td, mut test_folder) = tempdir().unwrap();
    test_folder.push("vendor");
    let output = vendor(VendorOptions {
        output: Some(&test_folder),
        platforms: Some(PLATFORMS),
        ..Default::default()
    })
    .unwrap();
    assert!(output.status.success());
    verify_no_windows(&test_folder);
}

#[test]
fn linux_multiple() {
    const PLATFORMS: &[&str] = &["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"];
    if !targets_supported(PLATFORMS) {
        return;
    }
    let (_td, mut test_folder) = tempdir().unwrap();
    test_folder.push("vendor");
    let output = vendor(VendorOptions {
        output: Some(&test_folder),
        platforms: Some(PLATFORMS),
        ..Default::default()
    })
    .unwrap();
    assert!(output.status.success());
    verify_no_windows(&test_folder);
}

#[test]
fn linux_and_windows_with_dep_kind_filter() {
    const PLATFORMS: &[&str] = &["x86_64-unknown-linux-gnu", "x86_64-pc-windows-gnu"];
    if !targets_supported(PLATFORMS) {
        return;
    }
    let (_td, mut test_folder) = tempdir().unwrap();
    test_folder.push("vendor");
    let output = vendor(VendorOptions {
        output: Some(&test_folder),
        platforms: Some(PLATFORMS),
        keep_dep_kinds: Some("no-dev"),
        ..Default::default()
    })
    .unwrap();
    assert!(output.status.success());
    // A package needed by one platform only has to survive the dependency kind
    // filtering of the other platforms: anstyle-wincon is a normal dependency
    // on windows only, libc on linux only.
    verify_crate_is_no_stub(&test_folder, "anstyle-wincon");
    verify_crate_is_no_stub(&test_folder, "libc");
}

#[test]
fn linux_glob() {
    // The tier 2 targets the glob below expands to
    let targets: Vec<_> = Tier::Two
        .targets()
        .filter(|t| t.ends_with("-unknown-linux-gnu"))
        .collect();
    if !targets_supported(&targets) {
        return;
    }
    let (_td, mut test_folder) = tempdir().unwrap();
    test_folder.push("vendor");
    let output = vendor(VendorOptions {
        output: Some(&test_folder),
        platforms: Some(&["*-unknown-linux-gnu"]),
        tier: Some("2"),
        ..Default::default()
    })
    .unwrap();
    assert!(output.status.success());
    verify_no_windows(&test_folder);
}

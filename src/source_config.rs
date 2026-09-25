//! Resolving dependencies without cargo's `[source]` configuration, like
//! `cargo vendor` does.

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use cargo_metadata::MetadataCommand;
use std::process::Command;

/// The directory holding cargo's configuration files
const CARGO_CONFIG_DIR: &str = ".cargo";
/// The names of cargo's configuration file, in the order cargo prefers them
const CARGO_CONFIG_FILES: &[&str] = &["config", "config.toml"];
/// The table of cargo's configuration defining sources and their replacement
const SOURCE_KEY: &str = "source";
/// The table of cargo's configuration patching dependencies
const PATCH_KEY: &str = "patch";
/// The key of a patch or dependency naming a local path
const PATH_KEY: &str = "path";
/// The array of cargo's configuration overriding packages with local paths
const PATHS_KEY: &str = "paths";
/// The environment variable overriding cargo's home directory
const CARGO_HOME_ENV: &str = "CARGO_HOME";

/// Where and how to run the cargo commands resolving the dependencies.
///
/// `cargo vendor` ignores the `[source]` configuration unless it is run with
/// `--respect-source-config`, and so must the dependency resolution here: the
/// configuration `cargo vendor` suggests replaces crates.io with the vendor
/// directory, which is missing or outdated when vendoring again.
///
/// Cargo can't be told to ignore a part of its configuration, but it only
/// reads the configuration files in the directory it runs in and its parents,
/// and the one in `$CARGO_HOME`. So if one of the former has a `[source]`
/// table, cargo runs in an empty temporary directory instead, and gets them
/// back without `[source]` via `--config`. `[source]` in `$CARGO_HOME` still
/// applies. The relative paths in `[patch]` and `paths` of a rewritten file
/// are made absolute; others, which don't matter for resolving dependencies,
/// resolve against the parent of the temporary directory, as does a relative
/// `CARGO_TARGET_DIR`. And unless the rustup proxy picked the toolchain before
/// (as for `cargo vendor-filterer`), a `rust-toolchain.toml` of the project
/// doesn't apply either.
#[derive(Debug, Default)]
pub(crate) struct Cargo {
    /// The empty directory to run cargo in, which also holds the rewritten
    /// configuration files
    workdir: Option<tempfile::TempDir>,
    /// The configuration files to pass with `--config`, outermost first as
    /// later ones take precedence
    configs: Vec<Utf8PathBuf>,
}

impl Cargo {
    /// Prepare to run cargo as if in `cwd`, where `respect_source_config` is
    /// the `--respect-source-config` of `cargo vendor`.
    pub(crate) fn new(cwd: &Utf8Path, respect_source_config: bool) -> Result<Self> {
        if respect_source_config {
            return Ok(Self::default());
        }
        let temp_dir =
            Utf8PathBuf::try_from(std::env::temp_dir()).context("Non-UTF-8 temporary directory")?;
        Self::new_in(cwd, cargo_home(cwd).as_deref(), &temp_dir)
    }

    /// Like [`Self::new`] ignoring `[source]`, with cargo's home directory
    /// `cargo_home` and the temporary directory in `temp_dir`.
    fn new_in(cwd: &Utf8Path, cargo_home: Option<&Utf8Path>, temp_dir: &Utf8Path) -> Result<Self> {
        // Innermost first
        let configs = find_configs(cwd, cargo_home)
            .map(|path| {
                let contents =
                    std::fs::read_to_string(&path).with_context(|| format!("Reading {path}"))?;
                let table: toml::Table =
                    toml::from_str(&contents).with_context(|| format!("Parsing {path}"))?;
                Ok((path, table))
            })
            .collect::<Result<Vec<_>>>()?;
        if !configs
            .iter()
            .any(|(_, table)| table.contains_key(SOURCE_KEY))
        {
            return Ok(Self::default());
        }

        // Cargo would find the [source] again from inside the directory.
        let canonical_temp_dir = canonicalize(temp_dir);
        for (path, table) in &configs {
            let root = config_root(path);
            if table.contains_key(SOURCE_KEY) && canonical_temp_dir.starts_with(canonicalize(root))
            {
                anyhow::bail!(
                    "The temporary directory {temp_dir} is inside {root}, so cargo would \
                     use the [{SOURCE_KEY}] configuration of {path}; set TMPDIR elsewhere"
                );
            }
        }
        let workdir = tempfile::tempdir_in(temp_dir)?;
        let workdir_path = Utf8Path::from_path(workdir.path())
            .with_context(|| format!("Non-UTF-8 temporary directory {:?}", workdir.path()))?;
        let mut paths = Vec::new();
        for (i, (path, mut table)) in configs.into_iter().rev().enumerate() {
            if table.remove(SOURCE_KEY).is_none() {
                paths.push(path);
                continue;
            }
            eprintln!("Ignoring [{SOURCE_KEY}] in {path}, like cargo vendor");
            make_paths_absolute(&mut table, config_root(&path));
            let rewritten = workdir_path.join(format!("config-{i}.toml"));
            std::fs::write(&rewritten, toml::to_string(&table)?)
                .with_context(|| format!("Writing {rewritten}"))?;
            paths.push(rewritten);
        }
        Ok(Self {
            workdir: Some(workdir),
            configs: paths,
        })
    }

    /// Whether cargo runs in another directory than the current one, so
    /// paths passed to it must be absolute.
    pub(crate) fn relocated(&self) -> bool {
        self.workdir.is_some()
    }

    /// The options to pass to every cargo command
    pub(crate) fn options(&self) -> impl Iterator<Item = String> + '_ {
        self.configs
            .iter()
            .flat_map(|path| ["--config".to_owned(), path.to_string()])
    }

    /// Prepare a cargo command; add [`Self::options`] after the subcommand.
    pub(crate) fn command(&self) -> Command {
        let mut command = Command::new("cargo");
        if let Some(workdir) = &self.workdir {
            command.current_dir(workdir.path());
        }
        command
    }

    /// Prepare `cargo metadata`; add [`Self::options`] to its other options.
    pub(crate) fn metadata(&self) -> MetadataCommand {
        let mut command = MetadataCommand::new();
        if let Some(workdir) = &self.workdir {
            command.current_dir(workdir.path());
        }
        command
    }
}

/// Cargo's home directory, if known, as cargo determines it when run in `cwd`.
fn cargo_home(cwd: &Utf8Path) -> Option<Utf8PathBuf> {
    if let Some(home) = std::env::var_os(CARGO_HOME_ENV) {
        return Some(cwd.join(Utf8PathBuf::try_from(std::path::PathBuf::from(home)).ok()?));
    }
    // Not deprecated anymore as of Rust 1.87, but still with 1.86
    #[allow(deprecated)]
    let home = std::env::home_dir()?;
    Some(Utf8PathBuf::try_from(home).ok()?.join(CARGO_CONFIG_DIR))
}

/// The path with symbolic links resolved, if it exists.
fn canonicalize(path: &Utf8Path) -> Utf8PathBuf {
    path.canonicalize_utf8().unwrap_or_else(|_| path.to_owned())
}

/// The directory relative paths in the configuration file `path` are relative
/// to: the parent of its `.cargo` directory.
fn config_root(path: &Utf8Path) -> &Utf8Path {
    path.parent()
        .and_then(Utf8Path::parent)
        .expect("configuration files are found in a .cargo directory")
}

/// Make the paths that affect dependency resolution in a configuration table
/// absolute, resolving them in `root`.
fn make_paths_absolute(table: &mut toml::Table, root: &Utf8Path) {
    let absolute = |value: &mut toml::Value| {
        if let toml::Value::String(path) = value {
            *path = root.join(&*path).into_string();
        }
    };
    // [patch.<registry>.<crate>] path = "..."
    let patches = table
        .get_mut(PATCH_KEY)
        .and_then(toml::Value::as_table_mut)
        .into_iter()
        .flat_map(|registries| registries.iter_mut().map(|(_, crates)| crates))
        .filter_map(toml::Value::as_table_mut)
        .flat_map(|crates| crates.iter_mut().map(|(_, patch)| patch))
        .filter_map(toml::Value::as_table_mut)
        .filter_map(|patch| patch.get_mut(PATH_KEY));
    patches.for_each(absolute);
    // paths = ["..."]
    if let Some(paths) = table.get_mut(PATHS_KEY).and_then(toml::Value::as_array_mut) {
        paths.iter_mut().for_each(absolute);
    }
}

/// The configuration files cargo reads from `cwd` and its parents, innermost
/// first, except the one in `cargo_home`, which cargo always reads.
fn find_configs<'a>(
    cwd: &'a Utf8Path,
    cargo_home: Option<&Utf8Path>,
) -> impl Iterator<Item = Utf8PathBuf> + 'a {
    let cargo_home = cargo_home.map(canonicalize);
    cwd.ancestors().filter_map(move |dir| {
        let dir = dir.join(CARGO_CONFIG_DIR);
        if cargo_home.as_deref() == Some(&*canonicalize(&dir)) {
            return None;
        }
        CARGO_CONFIG_FILES
            .iter()
            .map(|name| dir.join(name))
            .find(|path| path.is_file())
    })
}

/// The manifest cargo uses when run in `cwd` without `--manifest-path`.
pub(crate) fn find_manifest(cwd: &Utf8Path) -> Result<Utf8PathBuf> {
    cwd.ancestors()
        .map(|dir| dir.join(crate::CARGO_TOML))
        .find(|path| path.is_file())
        .with_context(|| {
            format!(
                "Could not find {} in {cwd} or its parents",
                crate::CARGO_TOML
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir() -> (tempfile::TempDir, Utf8PathBuf) {
        let td = tempfile::tempdir().unwrap();
        let path = Utf8Path::from_path(td.path()).unwrap().to_owned();
        (td, path)
    }

    fn write_config(dir: &Utf8Path, name: &str, contents: &str) -> Utf8PathBuf {
        let path = dir.join(CARGO_CONFIG_DIR).join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
        path
    }

    const VENDORED: &str = r#"
[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "vendor"
"#;
    const NET: &str = "[net]\nretry = 5\n";

    #[test]
    fn test_no_source_config() {
        let (_td, root) = tempdir();
        let cwd = root.join("project");
        write_config(&root, "config.toml", NET);
        write_config(&cwd, "config.toml", NET);
        let (_temp, temp_dir) = tempdir();
        let cargo = Cargo::new_in(&cwd, None, &temp_dir).unwrap();
        assert!(!cargo.relocated());
        assert_eq!(cargo.options().count(), 0);
    }

    #[test]
    fn test_respect_source_config() {
        let (_td, cwd) = tempdir();
        write_config(&cwd, "config.toml", VENDORED);
        let cargo = Cargo::new(&cwd, true).unwrap();
        assert!(!cargo.relocated());
        assert_eq!(cargo.options().count(), 0);
    }

    #[test]
    fn test_ignore_source_config() {
        let (_td, root) = tempdir();
        let cwd = root.join("project");
        let outer = write_config(&root, "config.toml", NET);
        // `config` takes precedence over `config.toml` in the same directory
        write_config(&cwd, "config.toml", NET);
        write_config(&cwd, "config", &format!("{NET}{VENDORED}"));

        let (_temp, temp_dir) = tempdir();
        let cargo = Cargo::new_in(&cwd, None, &temp_dir).unwrap();
        assert!(cargo.relocated());
        let options: Vec<_> = cargo.options().collect();
        let [flag1, config1, flag2, config2] = options.as_slice() else {
            panic!("expected two configuration files: {options:?}");
        };
        assert_eq!((flag1.as_str(), flag2.as_str()), ("--config", "--config"));
        // The outer one is passed unchanged, the inner one without [source]
        assert_eq!(config1, &outer);
        let inner: toml::Table =
            toml::from_str(&std::fs::read_to_string(config2).unwrap()).unwrap();
        let expected: toml::Table = toml::from_str(NET).unwrap();
        assert_eq!(inner, expected);
    }

    /// The configuration files passed to cargo
    fn configs(cargo: &Cargo) -> Vec<Utf8PathBuf> {
        let options: Vec<_> = cargo.options().collect();
        options
            .chunks(2)
            .map(|option| {
                assert_eq!(option[0], "--config");
                option[1].as_str().into()
            })
            .collect()
    }

    #[test]
    fn test_rewritten_paths_are_absolute() {
        let (_td, root) = tempdir();
        let cwd = root.join("project");
        write_config(
            &cwd,
            "config.toml",
            &format!(
                r#"paths = ["../local", "/abs/local"]
{VENDORED}

[patch.crates-io]
hex = {{ path = "../hexlocal" }}
bitflags = {{ git = "https://example.com/bitflags" }}

[patch."https://example.com/repo"]
foo = {{ path = "/abs/foo" }}
"#
            ),
        );
        let (_temp, temp_dir) = tempdir();
        let cargo = Cargo::new_in(&cwd, None, &temp_dir).unwrap();
        let [config] = configs(&cargo).try_into().unwrap();
        let config: toml::Table =
            toml::from_str(&std::fs::read_to_string(config).unwrap()).unwrap();
        let expected: toml::Table = toml::from_str(&format!(
            r#"
paths = ["{cwd}/../local", "/abs/local"]

[patch.crates-io]
hex = {{ path = "{cwd}/../hexlocal" }}
bitflags = {{ git = "https://example.com/bitflags" }}

[patch."https://example.com/repo"]
foo = {{ path = "/abs/foo" }}
"#
        ))
        .unwrap();
        assert_eq!(config, expected);
    }

    #[test]
    fn test_temp_dir_inside_project() {
        let (_td, root) = tempdir();
        let cwd = root.join("project");
        write_config(&cwd, "config.toml", VENDORED);
        let temp_dir = cwd.join("tmp");
        std::fs::create_dir_all(&temp_dir).unwrap();
        let err = Cargo::new_in(&cwd, None, &temp_dir).unwrap_err();
        assert!(err.to_string().contains("set TMPDIR elsewhere"), "{err}");

        // Outside of the directory with [source] is fine, even if inside one
        // with other configuration.
        write_config(&root, "config.toml", NET);
        let temp_dir = root.join("tmp");
        std::fs::create_dir_all(&temp_dir).unwrap();
        assert!(Cargo::new_in(&cwd, None, &temp_dir).unwrap().relocated());
    }

    #[test]
    fn test_cargo_home_is_skipped() {
        let (_td, root) = tempdir();
        let cwd = root.join("project");
        let cargo_home = root.join(CARGO_CONFIG_DIR);
        write_config(&root, "config.toml", &format!("{NET}{VENDORED}"));
        let (_temp, temp_dir) = tempdir();
        // Cargo reads the configuration in its home anyway, [source] included.
        let cargo = Cargo::new_in(&cwd, Some(&cargo_home), &temp_dir).unwrap();
        assert!(!cargo.relocated());

        let project_config = write_config(&cwd, "config.toml", NET);
        write_config(&cwd, "config.toml", &format!("{NET}{VENDORED}"));
        let cargo = Cargo::new_in(&cwd, Some(&cargo_home), &temp_dir).unwrap();
        let [config] = configs(&cargo).try_into().unwrap();
        assert_ne!(config, project_config);
        assert!(config.starts_with(&temp_dir));
    }

    #[test]
    fn test_find_manifest() {
        let (_td, root) = tempdir();
        let manifest = root.join(crate::CARGO_TOML);
        std::fs::write(&manifest, "").unwrap();
        let cwd = root.join("src/nested");
        std::fs::create_dir_all(&cwd).unwrap();
        assert_eq!(find_manifest(&cwd).unwrap(), manifest);
        assert_eq!(find_manifest(&root).unwrap(), manifest);
    }
}

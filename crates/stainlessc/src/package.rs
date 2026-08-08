// Recursive Stainless source-package resolution for the standalone compiler.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

pub(crate) const PACKAGE_MANIFEST_FILENAME: &str = "stainless-package.toml";

#[derive(Debug)]
pub(crate) struct ResolvedPackage {
    pub(crate) sources: Vec<PathBuf>,
    pub(crate) package_roots: Vec<PathBuf>,
    pub(crate) native_dependencies: Vec<(String, PathBuf)>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PackageManifest {
    schema: u32,
    name: String,
    sources: Vec<PathBuf>,
    #[serde(default)]
    main: Option<PathBuf>,
    #[serde(default)]
    dependencies: BTreeMap<String, PathBuf>,
    #[serde(default)]
    native_dependencies: BTreeMap<String, PathBuf>,
}

#[derive(Default)]
struct Resolver {
    visiting: BTreeSet<PathBuf>,
    visited: BTreeSet<PathBuf>,
    package_names: BTreeMap<String, PathBuf>,
    source_paths: BTreeSet<PathBuf>,
    sources: Vec<PathBuf>,
    package_roots: Vec<PathBuf>,
    native_dependencies: BTreeMap<String, PathBuf>,
}

/// Resolves one package and all transitive Stainless dependencies in
/// dependency-first order.
pub(crate) fn resolve(root: &Path) -> Result<ResolvedPackage, String> {
    let root = canonical_directory(root, "package root")?;
    let mut resolver = Resolver::default();
    resolver.visit(&root, None, true)?;
    Ok(ResolvedPackage {
        sources: resolver.sources,
        package_roots: resolver.package_roots,
        native_dependencies: resolver.native_dependencies.into_iter().collect(),
    })
}

impl Resolver {
    #[allow(clippy::too_many_lines)]
    fn visit(
        &mut self,
        root: &Path,
        expected_name: Option<&str>,
        include_main: bool,
    ) -> Result<(), String> {
        if self.visited.contains(root) {
            if let Some(expected_name) = expected_name {
                self.validate_resolved_name(root, expected_name)?;
            }
            return Ok(());
        }
        if !self.visiting.insert(root.to_owned()) {
            return Err(format!(
                "cyclic Stainless package dependency at `{}`",
                root.display()
            ));
        }

        let manifest_path = root.join(PACKAGE_MANIFEST_FILENAME);
        let source = fs::read_to_string(&manifest_path).map_err(|error| {
            format!(
                "failed to read Stainless package manifest `{}`: {error}",
                manifest_path.display()
            )
        })?;
        let manifest = toml::from_str::<PackageManifest>(&source).map_err(|error| {
            format!(
                "invalid Stainless package manifest `{}`: {error}",
                manifest_path.display()
            )
        })?;
        if manifest.schema != 1 {
            return Err(format!(
                "unsupported Stainless package schema {} in `{}`; expected 1",
                manifest.schema,
                manifest_path.display()
            ));
        }
        validate_name(&manifest.name, "package name")?;
        if let Some(expected_name) = expected_name
            && expected_name != manifest.name
        {
            return Err(format!(
                "dependency `{expected_name}` resolves to package `{}` at `{}`",
                manifest.name,
                root.display()
            ));
        }
        if let Some(previous) = self.package_names.get(&manifest.name)
            && previous != root
        {
            return Err(format!(
                "Stainless package `{}` resolves to both `{}` and `{}`",
                manifest.name,
                previous.display(),
                root.display()
            ));
        }
        self.package_names
            .insert(manifest.name.clone(), root.to_owned());

        for (name, dependency) in manifest.dependencies {
            validate_name(&name, "dependency name")?;
            let dependency_root = canonical_directory(&root.join(dependency), "dependency root")?;
            self.visit(&dependency_root, Some(&name), false)?;
        }
        for (name, dependency) in manifest.native_dependencies {
            validate_name(&name, "native dependency name")?;
            let dependency_root =
                canonical_directory(&root.join(dependency), "native dependency root")?;
            if !dependency_root.join("Cargo.toml").is_file() {
                return Err(format!(
                    "native dependency `{name}` has no Cargo.toml at `{}`",
                    dependency_root.display()
                ));
            }
            if let Some(previous) = self.native_dependencies.get(&name)
                && previous != &dependency_root
            {
                return Err(format!(
                    "native dependency `{name}` resolves to both `{}` and `{}`",
                    previous.display(),
                    dependency_root.display()
                ));
            }
            self.native_dependencies.insert(name, dependency_root);
        }
        for source in manifest.sources {
            self.add_source(root, &source)?;
        }
        if include_main && let Some(main) = manifest.main {
            self.add_source(root, &main)?;
        }

        self.visiting.remove(root);
        self.visited.insert(root.to_owned());
        self.package_roots.push(root.to_owned());
        Ok(())
    }

    fn validate_resolved_name(&self, root: &Path, expected_name: &str) -> Result<(), String> {
        match self.package_names.get(expected_name) {
            Some(resolved) if resolved == root => Ok(()),
            Some(resolved) => Err(format!(
                "dependency `{expected_name}` resolves to both `{}` and `{}`",
                resolved.display(),
                root.display()
            )),
            None => Err(format!(
                "dependency `{expected_name}` does not match the package at `{}`",
                root.display()
            )),
        }
    }

    fn add_source(&mut self, root: &Path, relative: &Path) -> Result<(), String> {
        validate_source_path(relative)?;
        let source = root.join(relative);
        let source = fs::canonicalize(&source).map_err(|error| {
            format!(
                "failed to resolve Stainless source `{}`: {error}",
                source.display()
            )
        })?;
        if !source.is_file() {
            return Err(format!(
                "Stainless source `{}` is not a file",
                source.display()
            ));
        }
        if self.source_paths.insert(source.clone()) {
            self.sources.push(source);
        }
        Ok(())
    }
}

fn canonical_directory(path: &Path, description: &str) -> Result<PathBuf, String> {
    let resolved = fs::canonicalize(path).map_err(|error| {
        format!(
            "failed to resolve {description} `{}`: {error}",
            path.display()
        )
    })?;
    if !resolved.is_dir() {
        return Err(format!(
            "{description} `{}` is not a directory",
            resolved.display()
        ));
    }
    Ok(resolved)
}

fn validate_name(name: &str, description: &str) -> Result<(), String> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(format!("invalid {description} `{name}`"));
    }
    Ok(())
}

fn validate_source_path(path: &Path) -> Result<(), String> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(format!(
            "Stainless source path `{}` must stay inside its package",
            path.display()
        ));
    }
    if path.extension().and_then(|extension| extension.to_str()) != Some("stl") {
        return Err(format!(
            "Stainless package source `{}` must end in .stl",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::resolve;
    use std::path::Path;

    #[test]
    fn resolves_the_poker_package_transitively() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let package = resolve(&workspace.join("apps/poker")).expect("poker package");
        assert!(
            package
                .sources
                .iter()
                .any(|source| source.ends_with("src/crypto.stl"))
        );
        assert!(
            package
                .sources
                .iter()
                .any(|source| source.ends_with("src/main.stl"))
        );
        assert!(package.native_dependencies.iter().any(|(name, path)| {
            name == "stainless-http" && path.ends_with("crates/stainless-http")
        }));

        let tests = resolve(&workspace.join("apps/poker/test")).expect("poker test package");
        assert!(
            tests
                .sources
                .iter()
                .any(|source| source.ends_with("test/poker_test.stl"))
        );
        assert!(
            !tests
                .sources
                .iter()
                .any(|source| source.ends_with("poker/src/main.stl"))
        );
    }
}

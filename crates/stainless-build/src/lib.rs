//! Cargo build-script integration for Stainless source files.

use std::env;
use std::error::Error;
use std::fmt;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Configures Stainless source files compiled as one translation unit by a
/// Cargo build script.
#[derive(Clone, Debug)]
pub struct Builder {
    sources: Vec<PathBuf>,
    output_name: Option<String>,
    exports: Vec<Export>,
}

#[derive(Clone, Debug)]
struct Export {
    stainless_path: String,
    rust_name: String,
}

impl Builder {
    /// Creates a build for `source`, relative to the consuming package root.
    #[must_use]
    pub fn new(source: impl Into<PathBuf>) -> Self {
        Self {
            sources: vec![source.into()],
            output_name: None,
            exports: Vec::new(),
        }
    }

    /// Adds another source fragment to the same Stainless translation unit.
    ///
    /// Sources are concatenated in call order with a newline boundary. This is
    /// useful for separating tests or generated declarations without creating
    /// a Stainless module boundary.
    #[must_use]
    pub fn add_source(mut self, source: impl Into<PathBuf>) -> Self {
        self.sources.push(source.into());
        self
    }

    /// Overrides the generated filename written beneath Cargo's `OUT_DIR`.
    #[must_use]
    pub fn output_name(mut self, output_name: impl Into<String>) -> Self {
        self.output_name = Some(output_name.into());
        self
    }

    /// Re-exports one exact, non-overloaded Stainless free function under a
    /// stable Rust name.
    #[must_use]
    pub fn export(
        mut self,
        stainless_path: impl Into<String>,
        rust_name: impl Into<String>,
    ) -> Self {
        self.exports.push(Export {
            stainless_path: stainless_path.into(),
            rust_name: rust_name.into(),
        });
        self
    }

    /// Transpiles the source and writes generated Rust beneath `OUT_DIR`.
    ///
    /// The returned path is suitable for `include!(concat!(env!("OUT_DIR"),
    /// "/name.stainless.rs"))` when paired with a fixed [`Self::output_name`].
    ///
    /// # Errors
    ///
    /// Returns an error for I/O failures, Stainless diagnostics, invalid or
    /// ambiguous exports, an invalid Rust export name, or a missing `OUT_DIR`.
    pub fn compile(&self) -> Result<PathBuf, BuildError> {
        verify_rustc_version()?;
        let package_root = package_root()?;
        let (source_paths, source) = load_sources(&package_root, &self.sources)?;
        let bindings_path =
            package_root.join(stainless_compiler::interop::BINDINGS_MANIFEST_FILENAME);
        println!("cargo:rerun-if-changed={}", bindings_path.display());
        let bindings =
            stainless_compiler::interop::load_package_bindings(&package_root).map_err(|error| {
                BuildError::new(format!(
                    "failed to load Stainless bindings for `{}`: {error}",
                    package_root.display()
                ))
            })?;
        let result = stainless_compiler::transpile_with_bindings(&source, &bindings);
        for warning in result
            .analysis
            .diagnostics
            .iter()
            .filter(|diagnostic| !diagnostic.is_error())
        {
            println!(
                "cargo:warning={} {:?} at {}..{}: {}",
                warning.code, warning.phase, warning.span.start, warning.span.end, warning.message
            );
        }
        let errors = result
            .analysis
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.is_error())
            .collect::<Vec<_>>();
        if !errors.is_empty() {
            let diagnostics = result
                .analysis
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.is_error())
                .map(|diagnostic| {
                    format!(
                        "{} {:?} at {}..{}: {}",
                        diagnostic.code,
                        diagnostic.phase,
                        diagnostic.span.start,
                        diagnostic.span.end,
                        diagnostic.message
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            return Err(BuildError::new(format!(
                "Stainless compilation failed for {}:\n{diagnostics}",
                display_source_paths(&source_paths)
            )));
        }

        let mut generated = result
            .rust
            .ok_or_else(|| BuildError::new("Stainless produced no generated Rust"))?;
        for export in &self.exports {
            append_export(&mut generated, &result.analysis.semantics, export)?;
        }

        let output_name = self
            .output_name
            .clone()
            .unwrap_or_else(|| default_output_name(&self.sources[0]));
        if Path::new(&output_name)
            .file_name()
            .and_then(|name| name.to_str())
            != Some(output_name.as_str())
        {
            return Err(BuildError::new(
                "the Stainless output name must be one plain UTF-8 filename",
            ));
        }
        let output = PathBuf::from(
            env::var_os("OUT_DIR").ok_or_else(|| BuildError::new("Cargo did not set OUT_DIR"))?,
        )
        .join(output_name);
        fs::write(&output, generated).map_err(|error| {
            BuildError::new(format!(
                "failed to write generated Rust `{}`: {error}",
                output.display()
            ))
        })?;
        Ok(output)
    }
}

fn package_root() -> Result<PathBuf, BuildError> {
    Ok(PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR")
            .ok_or_else(|| BuildError::new("Cargo did not set CARGO_MANIFEST_DIR"))?,
    ))
}

fn load_sources(
    package_root: &Path,
    sources: &[PathBuf],
) -> Result<(Vec<PathBuf>, String), BuildError> {
    let source_paths = sources
        .iter()
        .map(|source| {
            if source.is_absolute() {
                source.clone()
            } else {
                package_root.join(source)
            }
        })
        .collect::<Vec<_>>();
    let mut combined = String::new();
    for source_path in &source_paths {
        println!("cargo:rerun-if-changed={}", source_path.display());
        let fragment = fs::read_to_string(source_path).map_err(|error| {
            BuildError::new(format!(
                "failed to read Stainless source `{}`: {error}",
                source_path.display()
            ))
        })?;
        if !combined.is_empty() {
            combined.push('\n');
        }
        combined.push_str(&fragment);
    }
    Ok((source_paths, combined))
}

fn display_source_paths(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| format!("`{}`", path.display()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn verify_rustc_version() -> Result<(), BuildError> {
    let rustc = env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let output = Command::new(&rustc)
        .arg("-Vv")
        .output()
        .map_err(|error| BuildError::new(format!("failed to query rustc with `-Vv`: {error}")))?;
    if !output.status.success() {
        return Err(BuildError::new(format!(
            "rustc `-Vv` exited with {} while checking Stainless compatibility",
            output.status
        )));
    }
    let version = String::from_utf8(output.stdout)
        .map_err(|error| BuildError::new(format!("rustc emitted a non-UTF-8 version: {error}")))?;
    let release = version
        .lines()
        .find_map(|line| line.strip_prefix("release: "))
        .ok_or_else(|| BuildError::new("rustc `-Vv` did not report a release"))?;
    let actual_minor = release.split('.').take(2).collect::<Vec<_>>().join(".");
    if actual_minor != stainless_compiler::SUPPORTED_RUST_MINOR {
        return Err(BuildError::new(format!(
            "Stainless's native bindings target Rust {}, but the active rustc reports {release}; select a matching Rust toolchain or Stainless compiler release",
            stainless_compiler::SUPPORTED_RUST_MINOR
        )));
    }
    Ok(())
}

fn append_export(
    generated: &mut String,
    semantics: &stainless_compiler::resolution::SemanticModel,
    export: &Export,
) -> Result<(), BuildError> {
    syn::parse_str::<syn::Ident>(&export.rust_name).map_err(|error| {
        BuildError::new(format!(
            "invalid Rust export name `{}`: {error}",
            export.rust_name
        ))
    })?;
    let source_path = export
        .stainless_path
        .split("::")
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if source_path.is_empty() || source_path.join("::") != export.stainless_path {
        return Err(BuildError::new(format!(
            "invalid Stainless export path `{}`",
            export.stainless_path
        )));
    }
    let candidates = semantics
        .functions
        .iter()
        .filter(|function| {
            function.receiver.is_none()
                && function.has_definition
                && function
                    .path
                    .iter()
                    .map(String::as_str)
                    .eq(source_path.iter().copied())
        })
        .collect::<Vec<_>>();
    let function = match candidates.as_slice() {
        [function] => *function,
        [] => {
            return Err(BuildError::new(format!(
                "cannot export missing Stainless free function `{}`",
                export.stainless_path
            )));
        }
        _ => {
            return Err(BuildError::new(format!(
                "cannot export overloaded Stainless function `{}` without a signature",
                export.stainless_path
            )));
        }
    };
    let mut target = String::from("crate");
    for namespace in &function.path[..function.path.len().saturating_sub(1)] {
        write!(target, "::__stainless_namespace_{namespace}")
            .expect("writing a generated path to a String cannot fail");
    }
    write!(target, "::{}", function.mangled_name)
        .expect("writing a generated path to a String cannot fail");
    write!(
        generated,
        "\n#[allow(unused_imports)]\npub use {target} as {};\n",
        export.rust_name
    )
    .expect("writing generated Rust to a String cannot fail");
    Ok(())
}

fn default_output_name(source: &Path) -> String {
    let stem = source
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("stainless");
    format!("{stem}.stainless.rs")
}

/// Failure produced while integrating Stainless into a Cargo build.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuildError {
    message: String,
}

impl BuildError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for BuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for BuildError {}

#[cfg(test)]
mod tests {
    use super::{Builder, default_output_name, display_source_paths};
    use std::path::{Path, PathBuf};

    #[test]
    fn derives_a_generated_output_name() {
        assert_eq!(
            default_output_name(Path::new("src/application.stl")),
            "application.stainless.rs"
        );
    }

    #[test]
    fn preserves_additional_source_order() {
        let builder = Builder::new("src/library.stl")
            .add_source("src/generated.stl")
            .add_source("tests/library_test.stl");

        assert_eq!(
            builder.sources,
            [
                PathBuf::from("src/library.stl"),
                PathBuf::from("src/generated.stl"),
                PathBuf::from("tests/library_test.stl"),
            ]
        );
        assert_eq!(
            display_source_paths(&builder.sources),
            "`src/library.stl`, `src/generated.stl`, `tests/library_test.stl`"
        );
    }
}

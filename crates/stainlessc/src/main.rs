use std::env;
use std::ffi::{OsStr, OsString};
use std::fmt::Write as _;
use std::fs;
use std::io::{self, Write as _};
use std::path::PathBuf;
use std::process::{Command as ProcessCommand, ExitCode};
use std::sync::atomic::{AtomicU64, Ordering};

mod package;

const USAGE: &str = "\
Usage: stainlessc [OPTIONS] [INPUT.stl]...

Transpile Stainless source to Rust.

Options:
    --check              Validate without emitting Rust
    --build              Compile a root `i32 main()` into an executable
    --run                Compile and run a root `i32 main()` function
    --package <DIR>      Build DIR/stainless-package.toml and its dependencies
    --package-root <DIR> Load DIR/stainless-bindings.toml (repeatable)
    --dependency <N=P>  Add native Cargo dependency N from path P
    -o, --output <PATH>  Write emitted Rust or the built executable to PATH
    -h, --help           Print help
    -V, --version        Print version
";

#[derive(Clone, Debug, Eq, PartialEq)]
struct Options {
    inputs: Vec<PathBuf>,
    package: Option<PathBuf>,
    package_roots: Vec<PathBuf>,
    dependencies: Vec<(String, PathBuf)>,
    output: Option<PathBuf>,
    check: bool,
    build: bool,
    run: bool,
}

enum Command {
    Compile(Options),
    Help,
    Version,
}

fn main() -> ExitCode {
    match parse_options(env::args_os().skip(1)) {
        Ok(Command::Help) => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Ok(Command::Version) => {
            println!("stainlessc {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Ok(Command::Compile(options)) => compile(&options),
        Err(message) => {
            eprintln!("error: {message}\n\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn parse_options(arguments: impl IntoIterator<Item = OsString>) -> Result<Command, String> {
    let mut inputs = Vec::new();
    let mut package = None;
    let mut package_roots = Vec::new();
    let mut dependencies = Vec::new();
    let mut output = None;
    let mut check = false;
    let mut build = false;
    let mut run = false;
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        match argument.to_str() {
            Some("-h" | "--help") => return Ok(Command::Help),
            Some("-V" | "--version") => return Ok(Command::Version),
            Some("--check") => check = true,
            Some("--build") => build = true,
            Some("--run") => run = true,
            Some("--package") => {
                if package.is_some() {
                    return Err("the package option may be provided only once".to_owned());
                }
                package = Some(PathBuf::from(arguments.next().ok_or_else(|| {
                    "expected a directory after the package option".to_owned()
                })?));
            }
            Some("-o" | "--output") => {
                if output.is_some() {
                    return Err("the output option may be provided only once".to_owned());
                }
                output = Some(PathBuf::from(arguments.next().ok_or_else(|| {
                    "expected a path after the output option".to_owned()
                })?));
            }
            Some("--package-root") => {
                let package_root = PathBuf::from(arguments.next().ok_or_else(|| {
                    "expected a directory after the package-root option".to_owned()
                })?);
                if package_roots.contains(&package_root) {
                    return Err(format!(
                        "package root `{}` was provided more than once",
                        package_root.display()
                    ));
                }
                package_roots.push(package_root);
            }
            Some("--dependency") => {
                let dependency = arguments
                    .next()
                    .ok_or_else(|| "expected NAME=PATH after the dependency option".to_owned())?;
                let dependency = dependency
                    .to_str()
                    .ok_or_else(|| "dependency specifications must be UTF-8".to_owned())?;
                let (name, path) = dependency
                    .split_once('=')
                    .ok_or_else(|| "dependency specifications must use NAME=PATH".to_owned())?;
                if name.is_empty()
                    || !name
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
                    || path.is_empty()
                {
                    return Err("dependency specifications must use a valid NAME=PATH".to_owned());
                }
                if dependencies.iter().any(|(existing, _)| existing == name) {
                    return Err(format!("dependency `{name}` was provided more than once"));
                }
                dependencies.push((name.to_owned(), PathBuf::from(path)));
            }
            Some(option) if option.starts_with('-') && option != "-" => {
                return Err(format!("unknown option `{option}`"));
            }
            _ => {
                inputs.push(PathBuf::from(argument));
            }
        }
    }
    if inputs.is_empty() && package.is_none() {
        return Err("missing a Stainless input file or --package <DIR>".to_owned());
    }
    if usize::from(check) + usize::from(build) + usize::from(run) > 1 {
        return Err("--check, --build, and --run are mutually exclusive".to_owned());
    }
    if run && output.is_some() {
        return Err("--run cannot be combined with --output".to_owned());
    }
    if check && output.is_some() {
        return Err("--check cannot be combined with --output".to_owned());
    }
    if build && output.is_none() {
        return Err("--build requires --output <PROGRAM>".to_owned());
    }
    if build
        && output
            .as_ref()
            .is_some_and(|output| inputs.contains(output))
    {
        return Err("the executable output cannot overwrite the Stainless input".to_owned());
    }
    Ok(Command::Compile(Options {
        inputs,
        package,
        package_roots,
        dependencies,
        output,
        check,
        build,
        run,
    }))
}

fn compile(options: &Options) -> ExitCode {
    let compilation = match prepare_compilation(options) {
        Ok(compilation) => compilation,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
    };
    let mut source = String::new();
    for input in &compilation.inputs {
        let fragment = match fs::read_to_string(input) {
            Ok(fragment) => fragment,
            Err(error) => {
                eprintln!("error: failed to read `{}`: {error}", input.display());
                return ExitCode::FAILURE;
            }
        };
        if !source.is_empty() {
            source.push('\n');
        }
        source.push_str(&fragment);
    }
    let (result, registry_dependencies) =
        match transpile_sources(&source, &compilation.package_roots) {
            Ok(output) => output,
            Err(error) => {
                eprintln!("error: {error}");
                return ExitCode::FAILURE;
            }
        };
    if let Some(dependency) = registry_dependencies.iter().find(|dependency| {
        compilation
            .dependencies
            .iter()
            .any(|(name, _)| name == &dependency.name)
    }) {
        eprintln!(
            "error: dependency `{}` is declared by both the package and --dependency",
            dependency.name
        );
        return ExitCode::FAILURE;
    }
    let input_label = compilation
        .inputs
        .iter()
        .map(|input| input.display().to_string())
        .collect::<Vec<_>>()
        .join(",");
    if !result.analysis.diagnostics.is_empty() {
        for diagnostic in &result.analysis.diagnostics {
            eprintln!(
                "{}:{}..{}: {:?} {} {:?}: {}",
                input_label,
                diagnostic.span.start,
                diagnostic.span.end,
                diagnostic.severity,
                diagnostic.code,
                diagnostic.phase,
                diagnostic.message
            );
        }
    }
    if result
        .analysis
        .diagnostics
        .iter()
        .any(stainless_compiler::Diagnostic::is_error)
    {
        return ExitCode::FAILURE;
    }
    if options.check {
        return ExitCode::SUCCESS;
    }
    if options.run {
        return run_program(&result, &compilation.dependencies, &registry_dependencies);
    }
    if options.build {
        return build_program(
            &result,
            &compilation.dependencies,
            &registry_dependencies,
            options
                .output
                .as_deref()
                .expect("--build was validated to have an output"),
        );
    }
    let Some(rust) = result.rust else {
        eprintln!("error: Stainless produced no Rust without a diagnostic");
        return ExitCode::FAILURE;
    };
    if options
        .output
        .as_deref()
        .is_none_or(|path| path == OsStr::new("-"))
    {
        if let Err(error) = io::stdout().lock().write_all(rust.as_bytes()) {
            eprintln!("error: failed to write generated Rust to stdout: {error}");
            return ExitCode::FAILURE;
        }
    } else if let Some(output) = &options.output
        && let Err(error) = fs::write(output, rust)
    {
        eprintln!("error: failed to write `{}`: {error}", output.display());
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

struct Compilation {
    inputs: Vec<PathBuf>,
    package_roots: Vec<PathBuf>,
    dependencies: Vec<(String, PathBuf)>,
}

fn prepare_compilation(options: &Options) -> Result<Compilation, String> {
    let mut inputs = Vec::new();
    let mut package_roots = Vec::new();
    let mut dependencies = Vec::new();
    if let Some(package_root) = &options.package {
        let package = package::resolve(package_root)?;
        inputs.extend(package.sources);
        package_roots.extend(package.package_roots);
        dependencies.extend(package.native_dependencies);
    }
    inputs.extend(options.inputs.iter().cloned());
    for package_root in &options.package_roots {
        if !package_roots.contains(package_root) {
            package_roots.push(package_root.clone());
        }
    }
    for (name, path) in &options.dependencies {
        if dependencies.iter().any(|(existing, _)| existing == name) {
            return Err(format!(
                "native dependency `{name}` is declared by both the package and --dependency"
            ));
        }
        dependencies.push((name.clone(), path.clone()));
    }
    if let Some(output) = &options.output
        && inputs.contains(output)
    {
        return Err("the output cannot overwrite a Stainless package source".to_owned());
    }
    Ok(Compilation {
        inputs,
        package_roots,
        dependencies,
    })
}

fn transpile_sources(
    source: &str,
    package_roots: &[PathBuf],
) -> Result<
    (
        stainless_compiler::TranspileResult,
        Vec<stainless_compiler::interop::CargoDependency>,
    ),
    String,
> {
    if package_roots.is_empty() {
        return Ok((stainless_compiler::transpile(source), Vec::new()));
    }
    let mut bindings = stainless_compiler::interop::standard_bindings()
        .map_err(|error| format!("failed to load compiler bindings: {error}"))?;
    let mut dependencies: Vec<stainless_compiler::interop::CargoDependency> = Vec::new();
    for package_root in package_roots {
        let external = stainless_compiler::interop::load_package_external_bindings(package_root)
            .map_err(|error| {
                format!(
                    "failed to load Stainless bindings from `{}`: {error}",
                    package_root.display()
                )
            })?;
        bindings = bindings.merge(external).map_err(|error| {
            format!(
                "conflicting Stainless bindings from `{}`: {error}",
                package_root.display()
            )
        })?;
        let package_dependencies = stainless_compiler::interop::load_package_dependencies(
            package_root,
        )
        .map_err(|error| {
            format!(
                "failed to load Stainless dependencies from `{}`: {error}",
                package_root.display()
            )
        })?;
        for dependency in package_dependencies {
            if let Some(existing) = dependencies
                .iter()
                .find(|existing| existing.name == dependency.name)
            {
                if existing != &dependency {
                    return Err(format!(
                        "conflicting package dependency `{}`",
                        dependency.name
                    ));
                }
            } else {
                dependencies.push(dependency);
            }
        }
    }
    Ok((
        stainless_compiler::transpile_with_bindings(source, &bindings),
        dependencies,
    ))
}

fn run_program(
    result: &stainless_compiler::TranspileResult,
    dependencies: &[(String, PathBuf)],
    registry_dependencies: &[stainless_compiler::interop::CargoDependency],
) -> ExitCode {
    let rust = match executable_source(result) {
        Ok(rust) => rust,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::FAILURE;
        }
    };
    let directory = match temporary_directory() {
        Ok(directory) => directory,
        Err(error) => {
            eprintln!("error: failed to create a temporary build directory: {error}");
            return ExitCode::FAILURE;
        }
    };
    let source = directory.join("generated.rs");
    let executable = directory.join(format!("stainless-program{}", env::consts::EXE_SUFFIX));
    let outcome = compile_executable(
        &rust,
        &source,
        &executable,
        dependencies,
        registry_dependencies,
    )
    .and_then(|()| {
        ProcessCommand::new(&executable)
            .status()
            .map_err(|error| format!("failed to run `{}`: {error}", executable.display()))
    });
    if let Err(error) = fs::remove_dir_all(&directory) {
        eprintln!(
            "warning: failed to remove temporary directory `{}`: {error}",
            directory.display()
        );
    }
    match outcome {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(status) => status
            .code()
            .and_then(|code| u8::try_from(code).ok())
            .map_or(ExitCode::FAILURE, ExitCode::from),
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn build_program(
    result: &stainless_compiler::TranspileResult,
    dependencies: &[(String, PathBuf)],
    registry_dependencies: &[stainless_compiler::interop::CargoDependency],
    output: &std::path::Path,
) -> ExitCode {
    let rust = match executable_source(result) {
        Ok(rust) => rust,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::FAILURE;
        }
    };
    let directory = match temporary_directory() {
        Ok(directory) => directory,
        Err(error) => {
            eprintln!("error: failed to create a temporary build directory: {error}");
            return ExitCode::FAILURE;
        }
    };
    let source = directory.join("generated.rs");
    let outcome = compile_executable(&rust, &source, output, dependencies, registry_dependencies);
    if let Err(error) = fs::remove_dir_all(&directory) {
        eprintln!(
            "warning: failed to remove temporary directory `{}`: {error}",
            directory.display()
        );
    }
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn compile_executable(
    rust: &str,
    source: &std::path::Path,
    output: &std::path::Path,
    dependencies: &[(String, PathBuf)],
    registry_dependencies: &[stainless_compiler::interop::CargoDependency],
) -> Result<(), String> {
    if rust.contains("::stainless_runtime::")
        || !dependencies.is_empty()
        || !registry_dependencies.is_empty()
    {
        return compile_executable_with_runtime(
            rust,
            source,
            output,
            dependencies,
            registry_dependencies,
        );
    }
    fs::write(source, rust)
        .map_err(|error| format!("failed to write `{}`: {error}", source.display()))?;
    let rustc = env::var_os("RUSTC").unwrap_or_else(|| OsString::from("rustc"));
    let status = ProcessCommand::new(rustc)
        .arg("--edition=2024")
        .arg("--crate-name")
        .arg("stainless_program")
        .arg(source)
        .arg("-o")
        .arg(output)
        .status()
        .map_err(|error| format!("failed to invoke rustc: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("rustc rejected the generated program".to_owned())
    }
}

fn compile_executable_with_runtime(
    rust: &str,
    source: &std::path::Path,
    output: &std::path::Path,
    dependencies: &[(String, PathBuf)],
    registry_dependencies: &[stainless_compiler::interop::CargoDependency],
) -> Result<(), String> {
    let directory = source
        .parent()
        .ok_or_else(|| "temporary generated source has no parent directory".to_owned())?;
    let source_directory = directory.join("src");
    fs::create_dir(&source_directory).map_err(|error| {
        format!(
            "failed to create hidden Cargo source directory `{}`: {error}",
            source_directory.display()
        )
    })?;
    let main_source = source_directory.join("main.rs");
    fs::write(&main_source, rust)
        .map_err(|error| format!("failed to write `{}`: {error}", main_source.display()))?;

    let runtime_source = PathBuf::from(stainless_runtime::CRATE_SOURCE_DIR);
    let dependency = if runtime_source.join("Cargo.toml").is_file() {
        format!(
            "stainless-runtime = {{ path = \"{}\" }}",
            toml_escape_path(&runtime_source)
        )
    } else {
        format!("stainless-runtime = \"={}\"", env!("CARGO_PKG_VERSION"))
    };
    let mut manifest = format!(
        "[package]\nname = \"stainless-program\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\n\n[dependencies]\n{dependency}\n"
    );
    for (name, path) in dependencies {
        let path = if path.is_absolute() {
            path.clone()
        } else {
            env::current_dir()
                .map_err(|error| format!("failed to resolve dependency `{name}`: {error}"))?
                .join(path)
        };
        writeln!(
            manifest,
            "{name} = {{ path = \"{}\" }}",
            toml_escape_path(&path)
        )
        .expect("writing a dependency to a String cannot fail");
    }
    for dependency in registry_dependencies {
        let features = dependency
            .features
            .iter()
            .map(|feature| format!("\"{}\"", toml_escape_string(feature)))
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(
            manifest,
            "{} = {{ version = \"{}\", default-features = {}, features = [{}] }}",
            dependency.name,
            toml_escape_string(&dependency.version),
            dependency.default_features,
            features
        )
        .expect("writing a dependency to a String cannot fail");
    }
    let manifest_path = directory.join("Cargo.toml");
    fs::write(&manifest_path, manifest).map_err(|error| {
        format!(
            "failed to write hidden Cargo manifest `{}`: {error}",
            manifest_path.display()
        )
    })?;

    let target_directory = directory.join("target");
    let cargo = env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let status = ProcessCommand::new(cargo)
        .arg("build")
        .arg("--quiet")
        .current_dir(directory)
        .env("CARGO_TARGET_DIR", &target_directory)
        .status()
        .map_err(|error| format!("failed to invoke Cargo for the Stainless runtime: {error}"))?;
    if !status.success() {
        return Err("Cargo rejected the generated program or its Stainless runtime".to_owned());
    }

    let built = target_directory
        .join("debug")
        .join(format!("stainless-program{}", env::consts::EXE_SUFFIX));
    fs::copy(&built, output).map_err(|error| {
        format!(
            "failed to copy built executable `{}` to `{}`: {error}",
            built.display(),
            output.display()
        )
    })?;
    Ok(())
}

fn toml_escape_path(path: &std::path::Path) -> String {
    toml_escape_string(&path.to_string_lossy())
}

fn toml_escape_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn executable_source(result: &stainless_compiler::TranspileResult) -> Result<String, String> {
    let program = result
        .hir
        .as_ref()
        .ok_or_else(|| "Stainless produced no executable HIR".to_owned())?;
    let entries = program
        .functions
        .iter()
        .filter(|function| function.source_path == ["main"])
        .collect::<Vec<_>>();
    let entry = match entries.as_slice() {
        [entry] => *entry,
        [] => return Err("a runnable program must define root `i32 main()`".to_owned()),
        _ => return Err("the root `main` function cannot be overloaded".to_owned()),
    };
    if !entry.parameters.is_empty()
        || entry.return_type != stainless_compiler::hir::Type::Primitive("i32")
    {
        return Err("the program entry point must have the signature `i32 main()`".to_owned());
    }
    let mut rust = result
        .rust
        .clone()
        .ok_or_else(|| "Stainless produced no generated Rust".to_owned())?;
    if entry.throws {
        write!(
            rust,
            "\nfn main() {{\n    let code = match {}() {{\n        Ok(code) => code,\n        Err(error) => {{\n            ::std::eprintln!(\"Unhandled Stainless exception: {{error}}\");\n            1\n        }}\n    }};\n    ::std::process::exit(code);\n}}\n",
            entry.rust_name
        )
        .expect("writing generated Rust to a String cannot fail");
    } else {
        write!(
            rust,
            "\nfn main() {{\n    ::std::process::exit({}());\n}}\n",
            entry.rust_name
        )
        .expect("writing generated Rust to a String cannot fail");
    }
    Ok(rust)
}

fn temporary_directory() -> io::Result<PathBuf> {
    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);
    for _ in 0..100 {
        let index = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let directory = env::temp_dir().join(format!("stainlessc-{}-{index}", std::process::id()));
        match fs::create_dir(&directory) {
            Ok(()) => return Ok(directory),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique temporary directory",
    ))
}

#[cfg(test)]
mod tests {
    use super::{Command, Options, executable_source, parse_options};
    use std::ffi::OsString;
    use std::path::PathBuf;

    #[test]
    fn parses_check_and_output_modes() {
        let Command::Compile(check) =
            parse_options(["--check", "input.stl"].map(OsString::from)).expect("check arguments")
        else {
            panic!("expected compile command");
        };
        assert_eq!(
            check,
            Options {
                inputs: vec![PathBuf::from("input.stl")],
                package: None,
                package_roots: Vec::new(),
                dependencies: Vec::new(),
                output: None,
                check: true,
                build: false,
                run: false,
            }
        );

        let Command::Compile(output) =
            parse_options(["input.stl", "-o", "output.rs"].map(OsString::from))
                .expect("output arguments")
        else {
            panic!("expected compile command");
        };
        assert_eq!(output.output, Some(PathBuf::from("output.rs")));
    }

    #[test]
    fn parses_a_source_package_without_explicit_inputs() {
        let Command::Compile(options) = parse_options(
            ["--build", "--package", "apps/poker", "-o", "poker-dealer"].map(OsString::from),
        )
        .expect("package arguments") else {
            panic!("expected compile command");
        };
        assert_eq!(options.package, Some(PathBuf::from("apps/poker")));
        assert!(options.inputs.is_empty());
    }

    #[test]
    fn parses_multiple_sources_and_package_roots() {
        let Command::Compile(options) = parse_options(
            [
                "--check",
                "--package-root",
                "app",
                "--package-root",
                "library",
                "src/first.stl",
                "src/second.stl",
            ]
            .map(OsString::from),
        )
        .expect("multi-source arguments") else {
            panic!("expected compile command");
        };
        assert_eq!(
            options.inputs,
            vec![
                PathBuf::from("src/first.stl"),
                PathBuf::from("src/second.stl")
            ]
        );
        assert_eq!(
            options.package_roots,
            vec![PathBuf::from("app"), PathBuf::from("library")]
        );
    }

    #[test]
    fn parses_native_dependency_paths() {
        let Command::Compile(options) = parse_options(
            [
                "--check",
                "--dependency",
                "native-helper=../helper",
                "src/main.stl",
            ]
            .map(OsString::from),
        )
        .expect("dependency arguments") else {
            panic!("expected compile command");
        };
        assert_eq!(
            options.dependencies,
            vec![("native-helper".to_owned(), PathBuf::from("../helper"))]
        );
    }

    #[test]
    fn rejects_conflicting_or_unknown_options() {
        assert!(
            parse_options(["--check", "-o", "out.rs", "input.stl"].map(OsString::from)).is_err()
        );
        assert!(parse_options(["--wat"].map(OsString::from)).is_err());
        assert!(parse_options(std::iter::empty()).is_err());
    }

    #[test]
    fn parses_run_and_rejects_conflicting_output_modes() {
        let Command::Compile(run) =
            parse_options(["--run", "main.stl"].map(OsString::from)).expect("run arguments")
        else {
            panic!("expected compile command");
        };
        assert!(run.run);
        assert!(parse_options(["--run", "--check", "main.stl"].map(OsString::from)).is_err());
        assert!(parse_options(["--run", "-o", "program", "main.stl"].map(OsString::from)).is_err());
    }

    #[test]
    fn build_requires_a_distinct_executable_output() {
        let Command::Compile(build) =
            parse_options(["--build", "-o", "hello", "main.stl"].map(OsString::from))
                .expect("build arguments")
        else {
            panic!("expected compile command");
        };
        assert!(build.build);
        assert!(parse_options(["--build", "main.stl"].map(OsString::from)).is_err());
        assert!(
            parse_options(["--build", "-o", "main.stl", "main.stl"].map(OsString::from)).is_err()
        );
    }

    #[test]
    fn creates_a_rust_entry_point_for_stainless_main() {
        let result = stainless_compiler::transpile("i32 main() { return 0; }");
        let rust = executable_source(&result).expect("valid executable source");

        assert!(rust.contains("fn main()"));
        assert!(rust.contains("::std::process::exit("));

        let missing = stainless_compiler::transpile("i32 helper() { return 0; }");
        assert!(executable_source(&missing).is_err());
    }
}

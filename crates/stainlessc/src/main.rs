use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Write as _};
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "\
Usage: stainlessc [OPTIONS] <INPUT.stl>

Transpile Stainless source to Rust.

Options:
    --check              Validate without emitting Rust
    -o, --output <PATH>  Write Rust to PATH instead of stdout; use - for stdout
    -h, --help           Print help
    -V, --version        Print version
";

#[derive(Clone, Debug, Eq, PartialEq)]
struct Options {
    input: PathBuf,
    output: Option<PathBuf>,
    check: bool,
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
    let mut input = None;
    let mut output = None;
    let mut check = false;
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        match argument.to_str() {
            Some("-h" | "--help") => return Ok(Command::Help),
            Some("-V" | "--version") => return Ok(Command::Version),
            Some("--check") => check = true,
            Some("-o" | "--output") => {
                if output.is_some() {
                    return Err("the output option may be provided only once".to_owned());
                }
                output = Some(PathBuf::from(arguments.next().ok_or_else(|| {
                    "expected a path after the output option".to_owned()
                })?));
            }
            Some(option) if option.starts_with('-') && option != "-" => {
                return Err(format!("unknown option `{option}`"));
            }
            _ => {
                if input.replace(PathBuf::from(argument)).is_some() {
                    return Err("expected exactly one Stainless input file".to_owned());
                }
            }
        }
    }
    let input = input.ok_or_else(|| "missing Stainless input file".to_owned())?;
    if check && output.is_some() {
        return Err("--check cannot be combined with --output".to_owned());
    }
    Ok(Command::Compile(Options {
        input,
        output,
        check,
    }))
}

fn compile(options: &Options) -> ExitCode {
    let source = match fs::read_to_string(&options.input) {
        Ok(source) => source,
        Err(error) => {
            eprintln!(
                "error: failed to read `{}`: {error}",
                options.input.display()
            );
            return ExitCode::FAILURE;
        }
    };
    let result = stainless_compiler::transpile(&source);
    if !result.analysis.diagnostics.is_empty() {
        for diagnostic in &result.analysis.diagnostics {
            eprintln!(
                "{}:{}..{}: {} {:?}: {}",
                options.input.display(),
                diagnostic.span.start,
                diagnostic.span.end,
                diagnostic.code,
                diagnostic.phase,
                diagnostic.message
            );
        }
        return ExitCode::FAILURE;
    }
    if options.check {
        return ExitCode::SUCCESS;
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

#[cfg(test)]
mod tests {
    use super::{Command, Options, parse_options};
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
                input: PathBuf::from("input.stl"),
                output: None,
                check: true,
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
    fn rejects_conflicting_or_unknown_options() {
        assert!(
            parse_options(["--check", "-o", "out.rs", "input.stl"].map(OsString::from)).is_err()
        );
        assert!(parse_options(["--wat"].map(OsString::from)).is_err());
        assert!(parse_options(std::iter::empty()).is_err());
    }
}

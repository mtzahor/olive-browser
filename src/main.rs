use olive_html::{ParseOptions, parse_reader};
use std::{
    env,
    ffi::OsStr,
    fs::File,
    io::{self, Write},
    process::ExitCode,
};

const HELP: &str = "Olive Browser — HTML parser\n\nUsage: olive [FILE|URL|-]\n\nParse an HTML document and print its DOM tree.\nOmit FILE or use - to read stdin. Input is limited to 1 MiB.\nHTTP(S) and file URLs require a build with --features net (included in gui).\nHTML recovery diagnostics go to stderr. Scripts are never executed.\n\nOptions:\n  -h, --help       Show help\n  -V, --version    Show version\n";

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args_os().skip(1);
    let source = args.next();
    if args.next().is_some() {
        return Err("expected a single file path or URL; use --help for usage".into());
    }
    match source.as_deref() {
        Some(arg) if arg == OsStr::new("--help") || arg == OsStr::new("-h") => {
            io::stdout().lock().write_all(HELP.as_bytes())?;
            return Ok(());
        }
        Some(arg) if arg == OsStr::new("--version") || arg == OsStr::new("-V") => {
            writeln!(io::stdout().lock(), "olive {}", env!("CARGO_PKG_VERSION"))?;
            return Ok(());
        }
        Some(arg) if arg != OsStr::new("-") && arg.to_string_lossy().starts_with('-') => {
            return Err(
                "unknown option; use --help for usage (prefix dash-leading file paths with ./)"
                    .into(),
            );
        }
        _ => {}
    }
    let options = ParseOptions::default();
    let parsed = match source.as_deref() {
        None => parse_reader(io::stdin().lock(), options)?,
        Some(arg) if arg == OsStr::new("-") => parse_reader(io::stdin().lock(), options)?,
        Some(path) => {
            let input = path.to_string_lossy();
            if input.contains("://") {
                #[cfg(feature = "net")]
                {
                    use olive_html::net::{DocumentLoader, Location};
                    DocumentLoader::new()?
                        .load(Location::from_input(&input)?)?
                        .parse(false)?
                }
                #[cfg(not(feature = "net"))]
                {
                    return Err("URL loading requires --features net; for example: cargo run --features net --bin olive -- https://example.com".into());
                }
            } else {
                parse_reader(File::open(path)?, options)?
            }
        }
    };
    let mut output = io::BufWriter::new(io::stdout().lock());
    parsed.document.write_tree(&mut output)?;
    output.flush()?;
    let mut errors = io::stderr().lock();
    for diagnostic in parsed.diagnostics {
        writeln!(
            errors,
            "HTML recovery near line {}: {}",
            diagnostic.line,
            diagnostic.message.escape_debug()
        )?;
    }
    if parsed.omitted_diagnostics > 0 {
        writeln!(
            errors,
            "{} additional diagnostics omitted",
            parsed.omitted_diagnostics
        )?;
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error)
            if error
                .downcast_ref::<io::Error>()
                .is_some_and(|e| e.kind() == io::ErrorKind::BrokenPipe) =>
        {
            ExitCode::SUCCESS
        }
        Err(error) => {
            let _ = writeln!(io::stderr().lock(), "olive: {error}");
            ExitCode::FAILURE
        }
    }
}

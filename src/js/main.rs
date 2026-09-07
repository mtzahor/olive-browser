use olive_html::js::{Runtime, ScriptOptions};
use std::{
    env,
    ffi::OsStr,
    fs::File,
    io::{self, Write},
    process::ExitCode,
};

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut check = false;
    let mut source = None;
    for arg in env::args_os().skip(1) {
        if arg == "--help" || arg == "-h" {
            println!(
                "Olive JavaScript\n\nUsage: olive-js [--check] [FILE|-]\n\nParse and execute UTF-8 JavaScript, or validate syntax with --check.\nOmit FILE or use - for stdin. Source limit: 256 KiB.\nConsole messages go to stderr; the result preview goes to stdout.\nNo DOM, files, network, timers, modules or promise jobs are provided."
            );
            return Ok(());
        } else if arg == "--check" && !check {
            check = true;
        } else if arg != "-" && arg.to_string_lossy().starts_with('-') {
            return Err("unknown option; use --help for usage".into());
        } else if source.replace(arg).is_some() {
            return Err("expected a single JavaScript file".into());
        }
    }
    let mut runtime = Runtime::new(ScriptOptions::default())?;
    let program = match source.as_deref() {
        None => runtime.parse_reader(io::stdin().lock())?,
        Some(path) if path == OsStr::new("-") => runtime.parse_reader(io::stdin().lock())?,
        Some(path) => runtime.parse_reader(File::open(path)?)?,
    };
    if check {
        return Ok(());
    }
    let result = runtime.execute(&program);
    for message in runtime.console() {
        writeln!(io::stderr().lock(), "{}", message.escape_debug())?;
    }
    if runtime.omitted_console_messages() > 0 {
        writeln!(
            io::stderr().lock(),
            "{} console messages omitted",
            runtime.omitted_console_messages()
        )?;
    }
    writeln!(io::stdout().lock(), "{}", result?.escape_debug())?;
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
            let _ = writeln!(
                io::stderr().lock(),
                "olive-js: {}",
                error.to_string().escape_debug()
            );
            ExitCode::FAILURE
        }
    }
}

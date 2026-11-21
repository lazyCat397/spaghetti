//! `spg`: the Spaghetti command line. the main event.
//!
//!   spg file.spg            run a program (shorthand for `spg run`)
//!   spg run file.spg        run a program
//!   spg check file.spg      type-check only, don't run
//!   spg build file.spg      make a standalone executable (-o NAME for its name)
//!   spg lsp                 start the language server on stdin/stdout
//!   spg --version | --help
//!
//! Exit codes: 0 ok · 1 the program/source has an error · 2 bad usage.

mod bundle;
mod winio;

use spg_vm::chunk::Program;
use spg_vm::Vm;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const BUILD_ID: &str = env!("SPG_BUILD_ID");

const HELP: &str = "\
spg — the Spaghetti language

USAGE:
    spg <file.spg> [OPTIONS] [-- ARGS...]   run a program; ARGS reach it as @sys.args()
    spg run <file.spg> [OPTIONS] [-- ARGS...]   same as above
    spg check <file.spg>            type-check without running
    spg build <file.spg> [-o NAME]  make a standalone executable that runs the program
                                    --lean: a smaller runtime (about 3 MB), no JIT
                                    --enc: renames the program's names and removes its
                                    comments (the standard library stays readable)
    spg fmt <file|folder>... [--check | --stdout] [--min | --min-nc]
                                    format .spg files (rewrites them; --check lists the
                                    ones that would change; --stdout prints the result)
                                    --min: compact layout, comments kept (no indentation,
                                    no blank lines, no unneeded spaces, statements joined by ';')
                                    --min-nc: the same, comments removed too
    spg lsp                         start the language server (used by editors)

OPTIONS:
    --time              print the VM execution time to stderr
    --dump-bytecode     print the compiled bytecode before running
    --no-opt            skip the superinstruction optimizer (slower; for debugging)
    --no-jit            run everything in the VM (native code for numeric, string and array functions is on by default)
    --safe-party        run with the safe-party limits on, even if the file does not say #> safe-party
    --jit-report        after the run, print what native code made of each function and loop, and why the rest stays in the VM
    -V, --version       print the version
    -h, --help          print this help

EXIT CODES:
    0  success      1  error in the program or its source      2  bad usage
    (a program may also end with any code of its own: @sys.exit(code))
";

/// runs the `spg` command line. `gui` is set by `spgw`, a window build: its output goes to the
/// terminal that started it, or to a log file when there is none, and it never opens a console.
pub fn run_cli(gui: bool) {
    // `println!` panics when stdout is a closed pipe (`spg prog.spg | head`).
    // that's not a bug in the user's program. leave quietly. any other panic
    // is ours. say so plainly, instead of dumping a Rust backtrace at someone.
    std::panic::set_hook(Box::new(|info| {
        if info.to_string().contains("Broken pipe") {
            std::process::exit(1);
        }
        spg_vm::flush_stdout_in_panic(); // whatever the program printed before the bug still counts
        eprintln!("spg: internal error: {}", info);
        eprintln!("this is a bug in spg, not in your program");
    }));

    // a window build has no console of its own: attach to the one that started it, or log
    let console = !gui || winio::attach_or_log();

    let args: Vec<String> = std::env::args().skip(1).collect();
    // an executable made by `spg build` carries its program. it runs that, and
    // every arg goes to the program itself
    let bundle = bundle::read_own();
    // the VM recurses natively on every spg call, so the whole CLI runs on a
    // thread with a big stack instead of the OS default (8 MB on Linux, only
    // 1 MB on Windows). the memory is reserved lazily, not committed. cheap.
    let worker = std::thread::Builder::new()
        .stack_size(VM_STACK_BYTES)
        .spawn(move || match bundle {
            Some(bundle) => run_bundle(bundle, &args),
            None => real_main(&args),
        })
        .expect("failed to spawn the main worker thread");
    // a panic was already reported by the hook above. once is enough.
    let code = worker.join().unwrap_or(101);
    if gui && !console && code != 0 {
        winio::report_failure(code);
    }
    std::process::exit(code);
}

fn usage_error(msg: &str) -> i32 {
    eprintln!("error: {}", msg);
    eprintln!("try 'spg --help'");
    2
}

fn real_main(args: &[String]) -> i32 {
    let Some(first) = args.first() else {
        eprint!("{}", HELP);
        return 2;
    };

    match first.as_str() {
        "-h" | "--help" | "help" => {
            print!("{}", HELP);
            0
        }
        "-V" | "--version" | "version" => {
            println!("spg {} ({})", VERSION, BUILD_ID);
            0
        }
        #[cfg(feature = "lsp")]
        "lsp" => spg_lsp::run_stdio(),
        // the native libraries this build has, one per line. `spg build --lean` asks the lean
        // runtime which libraries it can run
        "--libs" => {
            for m in spg_libs::modules() {
                println!("{}", m.name);
            }
            0
        }
        "run" => run_cmd(&args[1..], true),
        "check" => run_cmd(&args[1..], false),
        "build" => build_cmd(&args[1..]),
        "fmt" => fmt_cmd(&args[1..]),
        _ => run_cmd(args, true),
    }
}

/// shared by `run` and `check`: parse options, find the one file arg
fn run_cmd(args: &[String], execute: bool) -> i32 {
    let mut file: Option<&str> = None;
    let mut dump_bytecode = false;
    let mut show_time = false;
    let mut optimize = true;
    // native code is on by default. --no-jit turns it off
    let mut jit = true;
    // --jit-report: after the run, print what native code made of each function and loop
    let mut jit_report = false;
    // --safe-party: the same limits as `#> safe-party`, set by whoever runs the file
    let mut safe_party = false;
    let mut program_args: Vec<String> = Vec::new();

    for (i, a) in args.iter().enumerate() {
        match a.as_str() {
            // everything after `--` belongs to the program (@sys.args()), not to spg
            "--" if execute => {
                program_args = args[i + 1..].to_vec();
                break;
            }
            "--dump-bytecode" if execute => dump_bytecode = true,
            "--time" if execute => show_time = true,
            "--no-opt" if execute => optimize = false,
            // native code is the default (see spg-jit). --jit stays as a no-op for old scripts
            "--jit" if execute => jit = true,
            "--no-jit" if execute => jit = false,
            "--jit-report" if execute => jit_report = true,
            "--safe-party" if execute => safe_party = true,
            s if s.starts_with('-') && s.len() > 1 => {
                return usage_error(&format!("unknown option '{}'", s));
            }
            s => {
                if file.is_some() {
                    return usage_error(&format!("unexpected extra argument '{}'", s));
                }
                file = Some(s);
            }
        }
    }
    let Some(file) = file else {
        return usage_error("missing input file");
    };

    // the entry file's own header picks the tone of every error it produces.
    // it's read on its own, so a file broken further down still gets it
    let mut spec = std::fs::read_to_string(file)
        .ok()
        .and_then(|src| spg_lexer::read_file_spec(&src).ok())
        .unwrap_or_default();
    // the flag only tightens: a file cannot switch it off with its own header
    if safe_party {
        spec.safe_party = true;
    }
    // `#> no-jit` in the file: the VM only, same as --no-jit
    if spec.no_jit {
        jit = false;
    }
    let friendly = spec.friendly_terminal;
    let strict = spec.strict;

    let bytecode = match build(std::path::Path::new(file), optimize, friendly, strict) {
        Ok(b) => b,
        Err(code) => return code,
    };
    // native code is generated from the plain (unfused) bytecode, which is what spg-jit understands.
    // the VM keeps running the optimized program
    let plain = match build(std::path::Path::new(file), false, friendly, strict) {
        Ok(b) => b,
        Err(code) => return code,
    };
    let native_src = if jit && cfg!(feature = "jit") { Some(plain) } else { None };

    if !execute {
        println!("ok: {} ({} function(s))", file, bytecode.functions.len());
        return 0;
    }
    execute_program(&bytecode, &spec, program_args, show_time, dump_bytecode, native_src.as_ref(), jit_report)
}

/// runs a compiled program: the `--dump-bytecode` listing, the program's
/// args, the `#>` timeout, and the error report with its stack trace
fn execute_program(bytecode: &Program, spec: &spg_lexer::FileSpec, program_args: Vec<String>, show_time: bool, dump_bytecode: bool, native_src: Option<&Program>, jit_report: bool) -> i32 {
    let friendly = spec.friendly_terminal;
    let strict = spec.strict;

    if dump_bytecode {
        println!("compile OK — {} function(s), main has {} instruction(s)", bytecode.functions.len(), bytecode.main.code.len());
        println!();
        println!("=== main ===");
        print_chunk(&bytecode.main);
        for (name, chunk) in bytecode.function_names.iter().zip(bytecode.functions.iter()) {
            println!();
            println!("=== fn {} ===", name);
            print_chunk(chunk);
        }
        println!();
        println!("--- running ---");
    }

    spg_vm::set_program_args(program_args);
    spg_vm::set_strict(strict);
    spg_vm::set_safe_party(spec.safe_party);
    // these must be set before the native code is generated below: spg-jit reads the depth limit
    if let Some(limit) = spec.stack_limit {
        spg_vm::set_call_depth_limit(limit);
    }
    if let Some(seed) = spec.seed {
        spg_vm::seed_random(seed);
    }
    if let Some(ms) = spec.timeout_ms {
        start_watchdog(ms, friendly);
    }
    let mut vm = Vm::new(bytecode);
    // why native code is missing, when it is: shown by --jit-report
    #[cfg_attr(not(feature = "jit"), allow(unused_mut))]
    let mut native_note: Option<String> = None;
    if let Some(src) = native_src {
        #[cfg(not(feature = "jit"))]
        {
            let _ = src;
        }
        #[cfg(feature = "jit")]
        {
        // native code is an optimization. if generating it panics, the VM runs everything.
        // the panic hook is silenced for this step only, so the user sees nothing.
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| spg_jit::build(src, &bytecode.struct_defs)));
        std::panic::set_hook(hook);
        let built = match built {
            Ok(r) => r,
            Err(_) => Err("internal error while generating native code".to_string()),
        };
        match built {
            Ok(table) => {
                if show_time {
                    eprintln!("--- native: {} function(s) can be compiled on demand ---", table.compiled);
                }
                vm.set_native(Box::new(table));
            }
            // not an error. the VM runs everything. shown only with --time, so normal runs stay quiet.
            Err(e) => {
                if show_time {
                    eprintln!("--- native code disabled, running in the VM: {e} ---");
                }
                native_note = Some(e);
            }
        }
        }
    }
    let start = std::time::Instant::now();
    let result = vm.run();
    let elapsed = start.elapsed();
    if jit_report {
        let lines = vm.native_report();
        if !cfg!(feature = "jit") {
            eprintln!("native code report: off (this runtime was built without native code, see --lean)");
        } else if native_src.is_none() {
            eprintln!("native code report: off (--no-jit)");
        } else if lines.is_empty() {
            eprintln!("native code report: none ({})", native_note.unwrap_or_else(|| "no native backend".to_string()));
        } else {
            for line in lines {
                eprintln!("{line}");
            }
        }
    }
    // a failed task nobody waited on: its error would vanish, so say so. the exit code doesn't change
    for (e, trace) in vm.take_unwaited_failures() {
        eprintln!("warning: a task failed and nobody called .wait() on it: {}", e);
        print_trace(trace);
    }
    // the checks of `attach #test`: listed with their place, and any failure makes the run fail
    let tests = spg_vm::take_test_report();
    print_test_report(&tests);
    let tests_failed = !tests.failures.is_empty();
    if let Err(e) = result {
        // `@sys.exit(code)` is the program picking its exit code, not a failure
        if let spg_vm::RuntimeError::Exit { code } = e {
            return if code == 0 && tests_failed { 1 } else { code };
        }
        if friendly {
            eprintln!("error: {}", e.friendly());
            print_trace(vm.take_trace());
        } else {
            let trace = vm.take_trace();
            match trace.0.first().and_then(|frame| runtime_diagnostic(&e, frame)) {
                Some(diagnostic) => eprintln!("{}\n", diagnostic),
                None => eprintln!("runtime error: {}", e),
            }
            print_trace(trace);
        }
        return 1;
    }
    if show_time {
        eprintln!(
            "--- VM execution time: {:.6}s ({} ms, {} µs) ---",
            elapsed.as_secs_f64(),
            elapsed.as_millis(),
            elapsed.as_micros()
        );
    }
    if tests_failed {
        return 1;
    }
    0
}

/// every failed `test.expect`, with its file and line, then the count
fn print_test_report(report: &spg_vm::TestReport) {
    if report.passed == 0 && report.failures.is_empty() {
        return;
    }
    for f in &report.failures {
        let place = match &f.file {
            Some(file) => format!("{}:{}", file, f.line),
            None => format!("line {}", f.line),
        };
        println!("FAIL {}: {}", place, f.message);
    }
    println!("test: {} passed, {} failed", report.passed, report.failures.len());
}


/// `spg build file.spg [-o NAME]`: validates the program like `check`, then
/// writes a standalone executable that runs it
/// a program that opens a window (`attach #ui` in any of its sources) needs the window build
fn window_program(sources: &[(std::path::PathBuf, String)]) -> bool {
    sources.iter().any(|(_, src)| {
        src.lines().any(|line| {
            let line = line.trim();
            line.starts_with("attach") && line.split_whitespace().any(|word| word == "#ui")
        })
    })
}

/// the native libraries a program attaches or installs (`attach #csv`, `install #ps`)
fn native_libs_used(sources: &[(std::path::PathBuf, String)]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for (_, src) in sources {
        for line in src.lines() {
            let line = line.trim();
            if !(line.starts_with("attach") || line.starts_with("install")) {
                continue;
            }
            for word in line.split_whitespace() {
                if let Some(rest) = word.strip_prefix('#') {
                    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                    if !name.is_empty() && !names.contains(&name) {
                        names.push(name);
                    }
                }
            }
        }
    }
    names
}

/// the lean runtime a program is made from: `spgw-lean` for a window program, `spg-lean` otherwise
fn lean_runtime_name(window: bool) -> &'static str {
    if window {
        "spgw-lean"
    } else {
        "spg-lean"
    }
}

/// the native libraries the lean runtime has, asked from the binary itself (`<runtime> --libs`)
fn lean_runtime_libs(window: bool) -> Result<Vec<String>, String> {
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = me.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let name = lean_runtime_name(window);
    let small = dir.join(format!("{}{}", name, std::env::consts::EXE_SUFFIX));
    if !small.is_file() {
        return Err(format!("'{}' was not found next to 'spg'", name));
    }
    let out = std::process::Command::new(&small)
        .arg("--libs")
        .output()
        .map_err(|e| format!("could not run '{}': {}", name, e))?;
    if !out.status.success() {
        return Err(format!("'{} --libs' failed", name));
    }
    Ok(String::from_utf8_lossy(&out.stdout).lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect())
}

/// the runtime a built program is made from: `spgw` for a window program, `spg` otherwise.
/// both sit next to each other, so the one running `spg build` can find the other.
fn runtime_bytes(window: bool, lean: bool) -> std::io::Result<Vec<u8>> {
    let me = std::env::current_exe()?;
    let dir = me.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let suffix = std::env::consts::EXE_SUFFIX;
    if lean {
        // `spg-lean` / `spgw-lean` sit next to `spg`, like `spgw`. they have no native code
        let small = dir.join(format!("{}{}", lean_runtime_name(window), suffix));
        if !small.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("'{}' was not found next to 'spg'", lean_runtime_name(window)),
            ));
        }
        return std::fs::read(small);
    }
    let console = dir.join(format!("spg{}", suffix));
    let windowed = dir.join(format!("spgw{}", suffix));
    if window && windowed.is_file() {
        return std::fs::read(windowed);
    }
    if window {
        eprintln!("warning: 'spgw' was not found next to 'spg': the program will open a console window");
    }
    if !window && console.is_file() {
        return std::fs::read(console);
    }
    if !window && me.file_stem().map(|s| s == "spgw").unwrap_or(false) {
        eprintln!("warning: 'spg' was not found next to 'spgw': the program will not print anything");
    }
    std::fs::read(me)
}

fn build_cmd(args: &[String]) -> i32 {
    let mut file: Option<&str> = None;
    let mut out: Option<&str> = None;
    let mut lean = false;
    let mut enc = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--lean" => lean = true,
            "--enc" => enc = true,
            "-o" | "--out" => {
                let Some(name) = args.get(i + 1) else {
                    return usage_error("'-o' needs a name for the executable");
                };
                out = Some(name);
                i += 1;
            }
            s if s.starts_with('-') && s.len() > 1 => {
                return usage_error(&format!("unknown option '{}'", s));
            }
            s => {
                if file.is_some() {
                    return usage_error(&format!("unexpected extra argument '{}'", s));
                }
                file = Some(s);
            }
        }
        i += 1;
    }
    let Some(file) = file else {
        return usage_error("missing input file");
    };
    let path = std::path::Path::new(file);

    // same checks as `spg check`. a program that doesn't compile doesn't get built
    let spec = std::fs::read_to_string(file)
        .ok()
        .and_then(|src| spg_lexer::read_file_spec(&src).ok())
        .unwrap_or_default();
    if let Err(code) = build(path, true, spec.friendly_terminal, spec.strict) {
        return code;
    }

    // every source file the program was built from, entry first
    let (_, sources) = match spg_modules::resolve_collect(path, &std::collections::HashMap::new()) {
        Ok(found) => found,
        Err(e) => {
            eprintln!("error: {}", e);
            return 1;
        }
    };

    // a standard module (`attach #cmp`) is written into the build under a folder of its own:
    // the program runs on another machine, where the stdlib folder isn't
    let std_here = spg_modules::std_dir().and_then(|d| d.canonicalize().ok());
    let sources: Vec<(std::path::PathBuf, String)> = sources
        .into_iter()
        .map(|(path, text)| {
            let moved = std_here
                .as_ref()
                .and_then(|std| path.strip_prefix(std).ok().map(|rel| bundled_std_dir().join(rel)));
            (moved.unwrap_or(path), text)
        })
        .collect();

    let sources = if enc {
        match encode_build_sources(sources) {
            Ok(sources) => sources,
            Err(e) => {
                eprintln!("error: {}", e);
                return 1;
            }
        }
    } else {
        sources
    };

    let out_path = match out {
        Some(name) => std::path::PathBuf::from(name),
        None => {
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "program".into());
            std::path::PathBuf::from(if cfg!(windows) { format!("{}.exe", stem) } else { stem })
        }
    };
    let window = window_program(&sources);
    if lean {
        // a native library the lean runtime lacks would fail only when the program runs
        let have = match lean_runtime_libs(window) {
            Ok(have) => have,
            Err(e) => {
                eprintln!("error: {}", e);
                return 1;
            }
        };
        let known = |name: &str| spg_libs::module(name).is_some();
        if let Some(missing) = native_libs_used(&sources).into_iter().find(|n| known(n) && !have.contains(n)) {
            eprintln!("error: '--lean' cannot build this program: it uses '#{}', which the lean runtime does not have. build without --lean", missing);
            return 1;
        }
    }
    let exe = match runtime_bytes(window, lean) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("error: could not read the spg executable to copy it: {}", e);
            return 1;
        }
    };
    if let Err(e) = bundle::write(&out_path, &exe, &sources) {
        eprintln!("error: could not write '{}': {}", out_path.display(), e);
        return 1;
    }
    let size = std::fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0);
    println!("built {} ({} source file(s), {} bytes)", out_path.display(), sources.len(), size);
    0
}

/// `--enc`: the program's own files, renamed and compact. the standard library's files stay
/// as they are, since the program calls them by their names
fn encode_build_sources(sources: Vec<(std::path::PathBuf, String)>) -> Result<Vec<(std::path::PathBuf, String)>, String> {
    let std = bundled_std_dir();
    let (user, library): (Vec<_>, Vec<_>) = sources.into_iter().partition(|(path, _)| !path.starts_with(&std));
    let user_texts: Vec<String> = user.iter().map(|(_, text)| text.clone()).collect();
    let library_texts: Vec<String> = library.iter().map(|(_, text)| text.clone()).collect();
    // what must keep its name: the native libraries, and the builtin functions (a program
    // that declares `len` still calls the builtin `len` from another file)
    let mut native: Vec<String> = spg_libs::modules().iter().map(|m| m.name.to_string()).collect();
    native.extend(spg_typecheck::builtins::builtin_signatures().into_keys());
    native.extend(spg_typecheck::builtins::checker_builtins().into_iter().map(|(name, _)| name.to_string()));
    let encoded = spg_fmt::encode_sources(&user_texts, &library_texts, &native).map_err(|e| e.to_string())?;
    let mut encoded = encoded.into_iter();
    let mut out: Vec<(std::path::PathBuf, String)> = Vec::new();
    for (path, _) in user {
        out.push((path, encoded.next().unwrap_or_default()));
    }
    out.extend(library);
    Ok(out)
}

/// the folder that stands for the standard library inside a built program. it is a path
/// that is not on the disk: the embedded files are found through it, like the overlay
fn bundled_std_dir() -> std::path::PathBuf {
    if cfg!(windows) {
        std::path::PathBuf::from(r"C:\spg-bundled-std")
    } else {
        std::path::PathBuf::from("/spg-bundled-std")
    }
}

/// an executable made by `spg build`: compile the embedded sources and run them
fn run_bundle(bundle: bundle::Bundle, args: &[String]) -> i32 {
    spg_modules::set_bundled_std(bundled_std_dir());
    let spec = spg_lexer::read_file_spec(&bundle.files[0].1).unwrap_or_default();
    // the same two copies `spg run` makes: the optimized one the VM runs, and the
    // plain one native code gets generated from (spg-jit only understands the plain one)
    let compile = |optimize: bool| -> Result<Program, i32> {
        let program = match spg_modules::resolve_with(&bundle.entry(), &bundle.overlay()) {
            Ok(program) => program,
            Err(e) => {
                if spec.friendly_terminal {
                    eprintln!("error: {}", e.friendly());
                } else {
                    eprintln!("error: {}", e);
                }
                return Err(1);
            }
        };
        compile_resolved(program, optimize, spec.friendly_terminal, spec.strict)
    };
    let bytecode = match compile(true) {
        Ok(bytecode) => bytecode,
        Err(code) => return code,
    };
    // native code is an optimization. without its plain copy the VM just runs everything
    let native_src = compile(false).ok();
    execute_program(&bytecode, &spec, args.to_vec(), false, false, native_src.as_ref(), false)
}

/// native stack reserved for the worker thread that runs the whole CLI
const VM_STACK_BYTES: usize = 1 << 30;

/// resolve imports -> typecheck -> compile. on failure the diagnostics get
/// printed and the exit code is returned.
/// `#> timeout N`: a thread that kills the whole process once the program has
/// run for `ms` ms. it's a thread, not a check inside the VM, so it
/// also cuts a `time.sleep` or a blocked `readLine`. exit code 124, like
/// the Unix `timeout` command. stolen with love.
fn start_watchdog(ms: u64, friendly: bool) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(ms));
        // never blocks, even if the VM is holding the output lock
        spg_vm::flush_stdout_in_panic();
        if friendly {
            let fact = format!("the program ran for more than {} ms and was stopped", ms);
            eprintln!("error: {}", spg_lexer::friendly_sentence(&fact, spg_lexer::HINT_TIMEOUT));
        } else {
            eprintln!("runtime error: timeout: the program ran for more than {} ms", ms);
        }
        std::process::exit(124);
    });
}

fn build(path: &std::path::Path, optimize: bool, friendly: bool, strict: bool) -> Result<Program, i32> {
    // resolves the entry file AND everything it imports into a single
    // merged program. lexing/parsing of every file happens in there
    let program = spg_modules::resolve(path).map_err(|e| {
        if friendly {
            eprintln!("error: {}", e.friendly());
        } else if let Some(diagnostic) = show_syntax_error(&e) {
            eprintln!("{}", diagnostic);
        } else {
            eprintln!("error: {}", e);
        }
        1
    })?;
    compile_resolved(program, optimize, friendly, strict)
}

fn print_chunk(chunk: &spg_vm::Chunk) {
    println!("locals: {}, arity: {}", chunk.num_locals, chunk.arity);
    for (i, op) in chunk.code.iter().enumerate() {
        println!("  {:>4}  {:?}", i, op);
    }
    if !chunk.constants.is_empty() {
        println!("  constants: {:?}", chunk.constants);
    }
}

/// a runtime error as a diagnostic: the statement where it died, underlined from its start to
/// the end of its line, with why and how to fix it. none when the file can't be read: the caller
/// then prints the one-line message.
fn runtime_diagnostic(e: &spg_vm::RuntimeError, frame: &spg_vm::TraceFrame) -> Option<String> {
    use spg_vm::RuntimeError;
    let file = frame.file.as_deref()?;
    let source = std::fs::read_to_string(file).ok()?;
    let text = source.lines().nth((frame.line as usize).checked_sub(1)?)?;
    let col = (frame.col as usize).max(1);
    let len = text.trim_end().chars().count().saturating_sub(col - 1).max(1);
    let (code, why, help): (&str, Option<String>, Vec<String>) = match e {
        RuntimeError::DivisionByZero => (
            "E0301",
            Some("the divisor is 0 at this point".to_string()),
            vec!["check the divisor first, for example: if (b != 0) { ... }".to_string()],
        ),
        RuntimeError::IndexOutOfBounds { index, len: 0 } => (
            "E0302",
            Some(format!("index {} is used on an empty array", index)),
            vec!["check the array's length before indexing it".to_string()],
        ),
        RuntimeError::IndexOutOfBounds { index, len } => (
            "E0302",
            Some(format!("index {} is outside 0..{}", index, len)),
            vec![format!("use an index between 0 and {}", len - 1)],
        ),
        _ => ("E0300", None, vec![]),
    };
    let fact = e.to_string();
    Some(spg_lexer::render_diagnostic(&spg_lexer::Diagnostic {
        code,
        fact: &fact,
        file,
        source: &source,
        line: frame.line as usize,
        col,
        len,
        focus: 0,
        why: why.as_deref(),
        help: &help,
    }))
}

/// the stack trace of a runtime error, innermost call first. one line per
/// frame, `at <function> (<file>:<line>)`. a run of identical frames (a
/// recursion stuck on one line) prints once, then a count. frames the VM
/// dropped at its limit get counted at the end.
fn print_trace((frames, omitted): (Vec<spg_vm::TraceFrame>, usize)) {
    let mut i = 0;
    while i < frames.len() {
        let frame = &frames[i];
        let mut j = i + 1;
        while j < frames.len() && frames[j] == *frame {
            j += 1;
        }
        match &frame.file {
            Some(file) if frame.col > 0 => eprintln!("  at {} ({}:{}:{})", frame.function, file, frame.line, frame.col),
            Some(file) => eprintln!("  at {} ({}:{})", frame.function, file, frame.line),
            None => eprintln!("  at {} (line {})", frame.function, frame.line),
        }
        if j - i > 1 {
            eprintln!("  ... same line repeated {} more times", j - i - 1);
        }
        i = j;
    }
    if omitted > 0 {
        eprintln!("  ... {} more frames", omitted);
    }
}

/// one type error as a full diagnostic: the source line with the underline, why, and how to fix it.
/// falls back to the one-line message when the source file can't be read.
fn show_type_error(e: &spg_typecheck::error::TypeError) -> String {
    let pos = e.pos();
    let Some(file) = pos.file.as_deref() else {
        return format!("  {}", e);
    };
    let Ok(source) = std::fs::read_to_string(file) else {
        return format!("  {}", e);
    };
    let fact = e.fact();
    let why = e.why();
    let help = e.help();
    spg_lexer::render_diagnostic(&spg_lexer::Diagnostic {
        code: e.code(),
        fact: &fact,
        file,
        source: &source,
        line: pos.line,
        col: pos.col,
        len: pos.len,
        focus: pos.focus,
        why: why.as_deref(),
        help: &help,
    })
}

/// a syntax error (lexing or parsing) of one file as a full diagnostic: the source line with
/// the underline, why, and how to fix it. none when the error has no place to point at, or the
/// file can't be read: the caller then prints the one-line message.
fn show_syntax_error(e: &spg_modules::ModuleError) -> Option<String> {
    use spg_modules::ModuleError;
    let (path, code, fact, why, help, spot) = match e {
        ModuleError::Parse { path, error, .. } => {
            let spot = error.spot();
            let is_eof = matches!(error, spg_parser::ParseError::UnexpectedEof { .. });
            (path, error.code(), error.fact(), error.why(), error.help(), if is_eof { None } else { spot })
        }
        ModuleError::Lex { path, error, .. } => (path, error.code(), error.fact(), error.why(), error.help(), Some(error.spot())),
        _ => return None,
    };
    let source = std::fs::read_to_string(path).ok()?;
    let file = path.display().to_string();
    // a file that ends too early points at its last line: the error is where reading stopped
    // a spot past the last line (the file ended while something was still open) points at the end
    let lines = source.lines().count().max(1);
    let (line, col) = match spot {
        Some((line, col)) if line <= lines => (line, col),
        _ => {
            let width = source.lines().last().map_or(0, |l| l.chars().count());
            (lines, width + 1)
        }
    };
    let text = source.lines().nth(line.saturating_sub(1)).unwrap_or("");
    let len = spg_lexer::token::token_extent(text, col);
    Some(spg_lexer::render_diagnostic(&spg_lexer::Diagnostic {
        code,
        fact: &fact,
        file: &file,
        source: &source,
        line,
        col,
        len,
        focus: 0,
        why: why.as_deref(),
        help: &help,
    }))
}

/// typecheck and compile a program whose files are already resolved
fn compile_resolved(program: spg_parser::ast::Program, optimize: bool, friendly: bool, strict: bool) -> Result<Program, i32> {
    let typed = spg_typecheck::type_check_with(program, strict).map_err(|errors| {
        if friendly {
            eprintln!("Oops! I found {} problem(s) with the types:", errors.len());
            for e in &errors {
                eprintln!("  {}", e.friendly_at());
            }
        } else {
            eprintln!("{} type error(s) found:", errors.len());
            for e in &errors {
                eprintln!("{}\n", show_type_error(e));
            }
        }
        1
    })?;

    spg_compiler::compile_with(typed, optimize).map_err(|e| {
        if friendly {
            eprintln!("error: {}", e.friendly());
        } else {
            eprintln!("compile error: {}", e);
        }
        1
    })
}

/// `spg fmt`: the canonical layout of one or more files, or of every `.spg`
/// file under a folder. a file gets written only when `spg_fmt::check` proves the
/// result has the same tokens, comments and directives, and is a fixed point.
fn fmt_cmd(args: &[String]) -> i32 {
    let mut check = false;
    let mut to_stdout = false;
    let mut layout = spg_fmt::Layout::Canonical;
    let mut paths: Vec<&str> = Vec::new();
    for a in args {
        match a.as_str() {
            "--check" => check = true,
            "--stdout" => to_stdout = true,
            "--min" | "--min-nc" => {
                if layout != spg_fmt::Layout::Canonical {
                    return usage_error("'--min' and '--min-nc' cannot be used together");
                }
                layout = spg_fmt::Layout::Compact { keep_comments: a == "--min" };
            }
            s if s.starts_with('-') && s.len() > 1 => return usage_error(&format!("unknown option '{}'", s)),
            s => paths.push(s),
        }
    }
    if check && to_stdout {
        return usage_error("'--check' and '--stdout' cannot be used together");
    }
    if paths.is_empty() {
        return usage_error("missing file or folder to format");
    }

    let mut files = Vec::new();
    for p in paths {
        if let Err(e) = collect_spg_files(std::path::Path::new(p), &mut files) {
            eprintln!("error: could not read '{}': {}", p, e);
            return 1;
        }
    }
    if files.is_empty() {
        eprintln!("error: no .spg files found");
        return 1;
    }

    let (mut changed, mut unchanged, mut failed) = (0usize, 0usize, 0usize);
    for file in &files {
        let src = match std::fs::read_to_string(file) {
            Ok(src) => src,
            Err(e) => {
                eprintln!("error: could not read '{}': {}", file.display(), e);
                failed += 1;
                continue;
            }
        };
        match spg_fmt::check_layout(&src, layout) {
            Err(e) => {
                eprintln!("error: {}: {}", file.display(), e);
                failed += 1;
            }
            Ok(out) if out == src => {
                unchanged += 1;
                // --stdout always prints the result, even when it is already formatted
                if to_stdout {
                    print!("{}", out);
                }
            }
            Ok(out) => {
                changed += 1;
                if to_stdout {
                    print!("{}", out);
                } else if check {
                    println!("would format: {}", file.display());
                } else if let Err(e) = std::fs::write(file, &out) {
                    eprintln!("error: could not write '{}': {}", file.display(), e);
                    failed += 1;
                } else {
                    println!("formatted: {}", file.display());
                }
            }
        }
    }
    if !to_stdout {
        eprintln!("{} file(s) checked: {} changed, {} already formatted, {} failed", files.len(), changed, unchanged, failed);
    }
    if failed > 0 || (check && changed > 0) {
        1
    } else {
        0
    }
}

/// a file as given, or every `.spg` under a folder (sorted, so output is stable)
fn collect_spg_files(path: &std::path::Path, out: &mut Vec<std::path::PathBuf>) -> std::io::Result<()> {
    if path.is_dir() {
        let mut entries: Vec<std::path::PathBuf> = std::fs::read_dir(path)?.filter_map(|e| e.ok().map(|e| e.path())).collect();
        entries.sort();
        for entry in entries {
            if entry.is_dir() {
                collect_spg_files(&entry, out)?;
            } else if entry.extension().is_some_and(|x| x == "spg") {
                out.push(entry);
            }
        }
    } else {
        out.push(path.to_path_buf());
    }
    Ok(())
}

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
    spg fmt <file|folder>... [--check | --stdout]
                                    format .spg files (rewrites them; --check lists the
                                    ones that would change; --stdout prints the result)
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

fn main() {
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
    std::process::exit(worker.join().unwrap_or(101));
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
        "lsp" => spg_lsp::run_stdio(),
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
    // native code is generated from the plain (unfused) bytecode, which is what
    // spg-jit understands. the VM keeps running the optimized program
    // the plain (unfused) program: native code is generated from it, and the static check of
    // `compile JIT` reads it. the check runs even with --no-jit, since the contract is the program's
    let plain = match build(std::path::Path::new(file), false, friendly, strict) {
        Ok(b) => b,
        Err(code) => return code,
    };
    if let Err(msg) = spg_jit::check_blocks(&plain) {
        eprintln!("error: {}: {}", file, msg);
        return 1;
    }
    let native_src = if jit { Some(plain) } else { None };

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
    let mut native_note: Option<String> = None;
    if let Some(src) = native_src {
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
    let start = std::time::Instant::now();
    let result = vm.run();
    let elapsed = start.elapsed();
    if jit_report {
        let lines = vm.native_report();
        if native_src.is_none() {
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
    // the checks of `install #test`: listed with their place, and any failure makes the run fail
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
        } else {
            eprintln!("runtime error: {}", e);
        }
        print_trace(vm.take_trace());
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
fn build_cmd(args: &[String]) -> i32 {
    let mut file: Option<&str> = None;
    let mut out: Option<&str> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
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

    let out_path = match out {
        Some(name) => std::path::PathBuf::from(name),
        None => {
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "program".into());
            std::path::PathBuf::from(if cfg!(windows) { format!("{}.exe", stem) } else { stem })
        }
    };
    let exe = match std::env::current_exe().and_then(std::fs::read) {
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

/// an executable made by `spg build`: compile the embedded sources and run them
fn run_bundle(bundle: bundle::Bundle, args: &[String]) -> i32 {
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
    if let Some(plain) = &native_src {
        if let Err(msg) = spg_jit::check_blocks(plain) {
            eprintln!("error: {}", msg);
            return 1;
        }
    }
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
                eprintln!("  {}", e);
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
    let mut paths: Vec<&str> = Vec::new();
    for a in args {
        match a.as_str() {
            "--check" => check = true,
            "--stdout" => to_stdout = true,
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
        match spg_fmt::check(&src) {
            Err(e) => {
                eprintln!("error: {}: {}", file.display(), e);
                failed += 1;
            }
            Ok(out) if out == src => unchanged += 1,
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

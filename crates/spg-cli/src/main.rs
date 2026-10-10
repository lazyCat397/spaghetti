//! `spg` — the Spaghetti command line.
//!
//!   spg file.spg            run a program (shorthand for `spg run`)
//!   spg run file.spg        run a program
//!   spg check file.spg      type-check only, don't run
//!   spg lsp                 start the language server on stdin/stdout
//!   spg --version | --help
//!
//! Exit codes: 0 ok · 1 the program/source has an error · 2 bad usage.

use spg_compiler::compile_with;
use spg_typecheck::type_check_with;
use spg_vm::chunk::Program;
use spg_vm::Vm;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const HELP: &str = "\
spg — the Spaghetti language

USAGE:
    spg <file.spg> [OPTIONS] [-- ARGS...]   run a program; ARGS reach it as @sys.args()
    spg run <file.spg> [OPTIONS] [-- ARGS...]   same as above
    spg check <file.spg>            type-check without running
    spg lsp                         start the language server (used by editors)

OPTIONS:
    --time              print the VM execution time to stderr
    --dump-bytecode     print the compiled bytecode before running
    --no-opt            skip the superinstruction optimizer (slower; for debugging)
    -V, --version       print the version
    -h, --help          print this help

EXIT CODES:
    0  success      1  error in the program or its source      2  bad usage
    (a program may also end with any code of its own: @sys.exit(code))
";

fn main() {
    // `println!` panics when stdout is a closed pipe (`spg prog.spg | head`).
    // That's not a bug in the user's program: leave quietly. Any other panic
    // is ours — say so plainly instead of dumping a raw Rust backtrace hint.
    std::panic::set_hook(Box::new(|info| {
        if info.to_string().contains("Broken pipe") {
            std::process::exit(1);
        }
        spg_vm::flush_stdout_in_panic(); // whatever the program printed before the bug still counts
        eprintln!("spg: internal error: {}", info);
        eprintln!("this is a bug in spg, not in your program");
    }));

    let args: Vec<String> = std::env::args().skip(1).collect();
    // The VM recurses natively on every spg call, so the whole CLI runs on a
    // thread with a big stack instead of the OS default (8 MB on Linux, only
    // 1 MB on Windows). The memory is reserved lazily, not committed.
    let worker = std::thread::Builder::new()
        .stack_size(VM_STACK_BYTES)
        .spawn(move || real_main(&args))
        .expect("failed to spawn the main worker thread");
    // a panic was already reported by the hook above
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
            println!("spg {}", VERSION);
            0
        }
        "lsp" => spg_lsp::run_stdio(),
        "run" => run_cmd(&args[1..], true),
        "check" => run_cmd(&args[1..], false),
        _ => run_cmd(args, true),
    }
}

/// shared by `run` and `check`: parse options, find the one file argument
fn run_cmd(args: &[String], execute: bool) -> i32 {
    let mut file: Option<&str> = None;
    let mut dump_bytecode = false;
    let mut show_time = false;
    let mut optimize = true;
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
    // It is read on its own, so a file broken further down still gets it
    let spec = std::fs::read_to_string(file)
        .ok()
        .and_then(|src| spg_lexer::read_file_spec(&src).ok())
        .unwrap_or_default();
    let friendly = spec.friendly_terminal;
    let strict = spec.strict;

    let bytecode = match build(std::path::Path::new(file), optimize, friendly, strict) {
        Ok(b) => b,
        Err(code) => return code,
    };

    if !execute {
        println!("ok: {} ({} function(s))", file, bytecode.functions.len());
        return 0;
    }

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
    if let Some(ms) = spec.timeout_ms {
        start_watchdog(ms, friendly);
    }
    let mut vm = Vm::new(&bytecode);
    let start = std::time::Instant::now();
    let result = vm.run();
    let elapsed = start.elapsed();
    if let Err(e) = result {
        // `@sys.exit(code)` is the program choosing its exit code, not a failure
        if let spg_vm::RuntimeError::Exit { code } = e {
            return code;
        }
        if friendly {
            eprintln!("error: {}", e.friendly());
        } else {
            eprintln!("runtime error: {}", e);
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
    0
}

/// native stack reserved for the worker thread that runs the whole CLI
const VM_STACK_BYTES: usize = 1 << 30;

/// resolve imports -> typecheck -> compile. On failure the diagnostics are
/// already printed and the exit code is returned.
/// `#> timeout N`: a thread that ends the whole process once the program has
/// run for `ms` milliseconds. It is a thread, not a check inside the VM, so it
/// also cuts a `time.sleep` or a `readLine` that is blocked. Exit code 124, like
/// the Unix `timeout` command
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
    // merged program — lexing/parsing of every file happens in there
    let program = spg_modules::resolve(path).map_err(|e| {
        if friendly {
            eprintln!("error: {}", e.friendly());
        } else {
            eprintln!("error: {}", e);
        }
        1
    })?;

    let typed = type_check_with(program, strict).map_err(|errors| {
        if friendly {
            eprintln!("Oops! I found {} problem(s) with the types:", errors.len());
            for e in &errors {
                eprintln!("  {}", e.friendly().trim_start_matches("Oops! "));
            }
        } else {
            eprintln!("{} type error(s) found:", errors.len());
            for e in &errors {
                eprintln!("  {}", e);
            }
        }
        1
    })?;

    compile_with(typed, optimize).map_err(|e| {
        if friendly {
            eprintln!("error: {}", e.friendly());
        } else {
            eprintln!("compile error: {}", e);
        }
        1
    })
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

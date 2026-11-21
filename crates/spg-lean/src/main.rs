//! `spg-lean`: the `spg` command line without native code. `spg build --lean` copies it
//! into a program, so the program carries no Cranelift.
fn main() {
    spg_cli::run_cli(false)
}

//! `spgw-lean`: the window build of `spg-lean`. the Windows subsystem keeps it from opening
//! a console, like `spgw`.
#![windows_subsystem = "windows"]

fn main() {
    spg_cli::run_cli(true)
}

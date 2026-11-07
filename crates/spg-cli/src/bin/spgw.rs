//! `spgw`: the window build of the same runtime. the Windows subsystem keeps it from
//! opening a console, so a program that opens a window (`install #ui`) doesn't bring a
//! terminal with it. `spg build` picks it for those programs.
#![windows_subsystem = "windows"]

fn main() {
    spg_cli::run_cli(true)
}

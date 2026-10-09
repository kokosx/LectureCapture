// Prevents an additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--selftest") {
        lecturecapture_lib::selftest::run(&args[i + 1..]);
        return;
    }
    lecturecapture_lib::run()
}

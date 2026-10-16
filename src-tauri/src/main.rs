// Prevents an additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Windows: the WebView2 renderer can crash on machines without a (working) GPU or with
    // broken drivers / in VMs. Software rendering is plenty for this UI. Users can override
    // by setting WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS themselves.
    #[cfg(windows)]
    if std::env::var_os("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS").is_none() {
        std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", "--disable-gpu --disable-gpu-compositing");
    }
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--selftest") {
        lecturecapture_lib::selftest::run(&args[i + 1..]);
        return;
    }
    lecturecapture_lib::run()
}

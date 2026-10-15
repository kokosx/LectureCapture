fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        // ScreenCaptureKit bridge uses the system Swift runtime
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        // Load vulkan-1.dll lazily: on PCs without a Vulkan driver the app must still start
        // (and transcribe on the CPU) instead of failing with "vulkan-1.dll not found".
        println!("cargo:rustc-link-arg-bins=delayimp.lib");
        println!("cargo:rustc-link-arg-bins=/DELAYLOAD:vulkan-1.dll");
    }
    tauri_build::build()
}

fn main() {
    // Single build call: manifest (asInvoker, so logon autostart works) + icons in one go.
    // Calling tauri_build twice would emit duplicate resources (CVT1100).
    let mut windows = tauri_build::WindowsAttributes::new();
    windows = windows.app_manifest(include_str!("windows-manifest.xml"));
    let attrs = tauri_build::Attributes::new().windows_attributes(windows);
    tauri_build::try_build(attrs).expect("failed to run build script")
}

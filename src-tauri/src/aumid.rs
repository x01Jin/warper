#[cfg(windows)]
pub fn ensure_aumid(app: &tauri::AppHandle) {
    let _ = run(&app.config().identifier);
}

#[cfg(not(windows))]
pub fn ensure_aumid(_app: &tauri::AppHandle) {}

#[cfg(windows)]
fn run(aumid: &str) -> Option<()> {
    advertise(aumid)?;
    register_key(aumid)?;
    install_shortcut(aumid)?;
    Some(())
}

#[cfg(windows)]
fn advertise(aumid: &str) -> Option<()> {
    use windows::core::HSTRING;
    use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
    unsafe { SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(aumid)) }.ok()
}

#[cfg(windows)]
fn register_key(aumid: &str) -> Option<()> {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE,
        REG_OPTION_NON_VOLATILE, REG_SZ,
    };

    fn wide_z(s: &str) -> Vec<u8> {
        s.encode_utf16()
            .chain(std::iter::once(0))
            .flat_map(u16::to_le_bytes)
            .collect()
    }

    let subkey = HSTRING::from(format!("Software\\Classes\\AppUserModelId\\{aumid}"));
    let mut key = HKEY::default();
    if unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            &subkey,
            None,
            windows::core::PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        )
    } != ERROR_SUCCESS
    {
        return None;
    }
    let display = wide_z("Warper");
    let icon = std::env::current_exe()
        .ok()
        .map(|exe| wide_z(&exe.to_string_lossy()));
    let ok = unsafe {
        RegSetValueExW(
            key,
            &HSTRING::from("DisplayName"),
            None,
            REG_SZ,
            Some(display.as_slice()),
        ) == ERROR_SUCCESS
            && icon.as_ref().is_none_or(|bytes| {
                RegSetValueExW(
                    key,
                    &HSTRING::from("IconUri"),
                    None,
                    REG_SZ,
                    Some(bytes.as_slice()),
                ) == ERROR_SUCCESS
            })
    };
    unsafe {
        let _ = RegCloseKey(key);
    }
    ok.then_some(())
}

#[cfg(windows)]
fn install_shortcut(aumid: &str) -> Option<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::{Interface, PCWSTR};
    use windows::Win32::Foundation::PROPERTYKEY;
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

    const PKEY_APP_USER_MODEL_ID: PROPERTYKEY = PROPERTYKEY {
        fmtid: windows::core::GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
        pid: 5,
    };

    let exe = std::env::current_exe().ok()?;
    let mut lnk = std::path::PathBuf::from(std::env::var_os("APPDATA")?);
    lnk.push(r"Microsoft\Windows\Start Menu\Programs");
    std::fs::create_dir_all(&lnk).ok()?;
    lnk.push("Warper.lnk");

    let exe_wide: Vec<u16> = exe
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let lnk_wide: Vec<u16> = lnk
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let aumid_value = PROPVARIANT::from(aumid);

    let com_ok = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
        .ok()
        .is_ok();
    let result = (|| {
        let link: IShellLinkW =
            unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }.ok()?;
        unsafe { link.SetPath(PCWSTR(exe_wide.as_ptr())) }.ok()?;
        unsafe { link.SetIconLocation(PCWSTR(exe_wide.as_ptr()), 0) }.ok()?;
        let store: IPropertyStore = link.cast().ok()?;
        let pkey = PKEY_APP_USER_MODEL_ID;
        unsafe { store.SetValue(std::ptr::addr_of!(pkey), std::ptr::addr_of!(aumid_value)) }
            .ok()?;
        unsafe { store.Commit() }.ok()?;
        let persist: IPersistFile = link.cast().ok()?;
        unsafe { persist.Save(PCWSTR(lnk_wide.as_ptr()), true) }.ok()?;
        Some(())
    })();
    if com_ok {
        unsafe { CoUninitialize() };
    }
    result
}

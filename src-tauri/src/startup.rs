use windows::core::{HSTRING, PWSTR};
use windows::Win32::Foundation::{ERROR_NO_MORE_ITEMS, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegEnumValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SAM_FLAGS, REG_SZ,
};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_NAME: &str = "Warper";
const LAYERS_KEY: &str = r"Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers";
const ELEVATION_FLAG: &str = "RUNASADMIN";

fn open(path: &str, access: REG_SAM_FLAGS) -> Option<HKEY> {
    let mut key = HKEY::default();
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            &HSTRING::from(path),
            None,
            access,
            &mut key,
        )
    };
    (status == ERROR_SUCCESS).then_some(key)
}

fn read_string(key: HKEY, name: &HSTRING) -> Option<String> {
    let mut size = 0u32;
    if unsafe { RegQueryValueExW(key, name, None, None, None, Some(&mut size)) } != ERROR_SUCCESS
        || size < 2
    {
        return None;
    }
    let mut buf = vec![0u8; size as usize];
    if unsafe {
        RegQueryValueExW(
            key,
            name,
            None,
            None,
            Some(buf.as_mut_ptr()),
            Some(&mut size),
        )
    } != ERROR_SUCCESS
    {
        return None;
    }
    let wide: Vec<u16> = buf
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .take(size as usize / 2)
        .collect();
    Some(
        String::from_utf16_lossy(&wide)
            .trim_end_matches('\0')
            .to_string(),
    )
}

fn write_string(key: HKEY, name: &HSTRING, value: &str) {
    let bytes: Vec<u8> = value
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect();
    unsafe {
        let _ = RegSetValueExW(key, name, None, REG_SZ, Some(bytes.as_slice()));
    }
}

fn value_names(key: HKEY) -> Vec<String> {
    let mut names = Vec::new();
    let mut index = 0u32;
    loop {
        let mut buf = [0u16; 2048];
        let mut len = buf.len() as u32;
        let status = unsafe {
            RegEnumValueW(
                key,
                index,
                Some(PWSTR(buf.as_mut_ptr())),
                &mut len,
                None,
                None,
                None,
                None,
            )
        };
        if status == ERROR_NO_MORE_ITEMS || status != ERROR_SUCCESS {
            break;
        }
        names.push(String::from_utf16_lossy(&buf[..len as usize]));
        index += 1;
    }
    names
}

pub fn run_entry() -> Option<String> {
    let key = open(RUN_KEY, KEY_QUERY_VALUE)?;
    let value = read_string(key, &HSTRING::from(RUN_NAME));
    unsafe {
        let _ = RegCloseKey(key);
    }
    value
}

pub fn clear_elevation_layer() -> Option<String> {
    let exe = std::env::current_exe().ok()?.to_string_lossy().into_owned();
    let key = open(LAYERS_KEY, KEY_QUERY_VALUE | KEY_SET_VALUE)?;
    let mut removed = None;
    for name in value_names(key) {
        if !name.trim_matches('"').eq_ignore_ascii_case(&exe) {
            continue;
        }
        let Some(stored) = read_string(key, &HSTRING::from(name.as_str())) else {
            continue;
        };
        let tokens: Vec<&str> = stored.split_whitespace().collect();
        let kept: Vec<&str> = tokens
            .iter()
            .copied()
            .filter(|token| !token.eq_ignore_ascii_case(ELEVATION_FLAG))
            .collect();
        if kept.len() == tokens.len() {
            continue;
        }
        let entry = &HSTRING::from(name.as_str());
        let next = kept.join(" ");
        if next.is_empty() || next == "~" {
            unsafe {
                let _ = RegDeleteValueW(key, entry);
            }
        } else {
            write_string(key, entry, &next);
        }
        removed = Some(stored);
    }
    unsafe {
        let _ = RegCloseKey(key);
    }
    removed
}

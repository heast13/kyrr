//! Small HKLM helpers: read DWORD/string values, write status values.

use windows::core::PCWSTR;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegGetValueW, RegSetValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_SET_VALUE,
    REG_OPTION_NON_VOLATILE, REG_QWORD, REG_SZ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
};

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// Reads a DWORD below `key` (an open key or HKLM) and `subkey`.
pub fn read_dword(key: HKEY, subkey: &str, name: &str) -> Option<u32> {
    let (subkey, name) = (wide(subkey), wide(name));
    let mut value = 0u32;
    let mut size = 4u32;
    let err = unsafe {
        RegGetValueW(
            key,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(name.as_ptr()),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    (err == ERROR_SUCCESS).then_some(value)
}

/// Reads a string value from HKLM.
pub fn read_string(subkey: &str, name: &str) -> Option<String> {
    let (subkey, name) = (wide(subkey), wide(name));
    let mut buf = vec![0u16; 512];
    let mut size = (buf.len() * 2) as u32;
    let err = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(name.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut size),
        )
    };
    if err != ERROR_SUCCESS {
        return None;
    }
    let len = (size as usize / 2).saturating_sub(1).min(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

/// A value to write with [`write_values`].
pub enum Value<'a> {
    Str(&'a str),
    Qword(u64),
}

/// Writes values below HKLM\`subkey`, creating the key. Best effort: the
/// status is informational, so failures are ignored.
pub fn write_values(subkey: &str, values: &[(&str, Value)]) {
    let sub = wide(subkey);
    let mut key = HKEY::default();
    let err = unsafe {
        RegCreateKeyExW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(sub.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        )
    };
    if err != ERROR_SUCCESS {
        return;
    }
    for (name, value) in values {
        let name = wide(name);
        let (kind, bytes): (_, Vec<u8>) = match value {
            Value::Str(s) => (REG_SZ, wide(s).iter().flat_map(|c| c.to_le_bytes()).collect()),
            Value::Qword(q) => (REG_QWORD, q.to_le_bytes().to_vec()),
        };
        unsafe {
            let _ = RegSetValueExW(key, PCWSTR(name.as_ptr()), None, kind, Some(&bytes));
        }
    }
    unsafe {
        let _ = RegCloseKey(key);
    }
}

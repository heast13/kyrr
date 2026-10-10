//! Registry keys with automatic closing and typed values.

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{LocalFree, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, HLOCAL, WIN32_ERROR};
use windows::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegDeleteValueW, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW,
    RegSetValueExW, HKEY, KEY_ALL_ACCESS, KEY_READ, KEY_SET_VALUE, REG_DWORD, REG_OPTION_BACKUP_RESTORE,
    REG_OPTION_NON_VOLATILE, REG_SAM_FLAGS, REG_SZ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
};

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn check(err: WIN32_ERROR, what: &str) -> Result<(), String> {
    if err == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!(
            "{what}: {}",
            windows::core::Error::from(err.to_hresult()).message()
        ))
    }
}

pub struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

impl Key {
    fn open_with(root: HKEY, path: &str, access: REG_SAM_FLAGS) -> Option<Key> {
        let p = wide(path);
        let mut key = HKEY::default();
        let err = unsafe { RegOpenKeyExW(root, PCWSTR(p.as_ptr()), None, access, &mut key) };
        (err == ERROR_SUCCESS).then_some(Key(key))
    }

    /// Opens an existing key for reading.
    pub fn open(root: HKEY, path: &str) -> Option<Key> {
        Key::open_with(root, path, KEY_READ)
    }

    /// Opens an existing key for reading and writing values.
    pub fn open_writable(root: HKEY, path: &str) -> Option<Key> {
        Key::open_with(root, path, KEY_READ | KEY_SET_VALUE)
    }

    /// Opens or creates a key for writing. `sddl` sets the security of a newly
    /// created key (an existing key keeps its own).
    pub fn create(root: HKEY, path: &str, sddl: Option<&str>) -> Result<Key, String> {
        let p = wide(path);
        let mut key = HKEY::default();
        let mut sd = PSECURITY_DESCRIPTOR::default();
        if let Some(sddl) = sddl {
            let s = wide(sddl);
            unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(PCWSTR(s.as_ptr()), SDDL_REVISION_1, &mut sd, None)
            }
            .map_err(|e| format!("security descriptor: {e}"))?;
        }
        let attrs = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd.0,
            bInheritHandle: false.into(),
        };
        let err = unsafe {
            RegCreateKeyExW(
                root,
                PCWSTR(p.as_ptr()),
                None,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_ALL_ACCESS,
                sddl.map(|_| &attrs as *const _),
                &mut key,
                None,
            )
        };
        if !sd.0.is_null() {
            unsafe {
                let _ = LocalFree(Some(HLOCAL(sd.0)));
            }
        }
        check(err, path)?;
        Ok(Key(key))
    }

    /// Opens or creates a key regardless of its security, for keys owned by
    /// TrustedInstaller such as the audio endpoints. Needs the backup and
    /// restore privileges ([`crate::privilege::enable_backup_restore`]).
    pub fn create_privileged(root: HKEY, path: &str) -> Result<Key, String> {
        let p = wide(path);
        let mut key = HKEY::default();
        let err = unsafe {
            RegCreateKeyExW(
                root,
                PCWSTR(p.as_ptr()),
                None,
                PCWSTR::null(),
                REG_OPTION_BACKUP_RESTORE,
                KEY_ALL_ACCESS,
                None,
                &mut key,
                None,
            )
        };
        check(err, path)?;
        Ok(Key(key))
    }

    pub fn get_string(&self, name: &str) -> Option<String> {
        let n = wide(name);
        let mut size = 0u32;
        let err = unsafe {
            RegGetValueW(
                self.0,
                PCWSTR::null(),
                PCWSTR(n.as_ptr()),
                RRF_RT_REG_SZ,
                None,
                None,
                Some(&mut size),
            )
        };
        if err != ERROR_SUCCESS {
            return None;
        }
        let mut buf = vec![0u16; size as usize / 2 + 1];
        let mut size = (buf.len() * 2) as u32;
        let err = unsafe {
            RegGetValueW(
                self.0,
                PCWSTR::null(),
                PCWSTR(n.as_ptr()),
                RRF_RT_REG_SZ,
                None,
                Some(buf.as_mut_ptr().cast()),
                Some(&mut size),
            )
        };
        if err != ERROR_SUCCESS {
            return None;
        }
        let len = (size as usize / 2).saturating_sub(1).min(buf.len());
        Some(String::from_utf16_lossy(&buf[..len]))
    }

    pub fn get_dword(&self, name: &str) -> Option<u32> {
        let n = wide(name);
        let mut value = 0u32;
        let mut size = 4u32;
        let err = unsafe {
            RegGetValueW(
                self.0,
                PCWSTR::null(),
                PCWSTR(n.as_ptr()),
                RRF_RT_REG_DWORD,
                None,
                Some((&mut value as *mut u32).cast()),
                Some(&mut size),
            )
        };
        (err == ERROR_SUCCESS).then_some(value)
    }

    pub fn set_string(&self, name: &str, value: &str) -> Result<(), String> {
        let n = wide(name);
        let bytes: Vec<u8> = wide(value).iter().flat_map(|c| c.to_le_bytes()).collect();
        check(
            unsafe { RegSetValueExW(self.0, PCWSTR(n.as_ptr()), None, REG_SZ, Some(&bytes)) },
            name,
        )
    }

    pub fn set_dword(&self, name: &str, value: u32) -> Result<(), String> {
        let n = wide(name);
        check(
            unsafe { RegSetValueExW(self.0, PCWSTR(n.as_ptr()), None, REG_DWORD, Some(&value.to_le_bytes())) },
            name,
        )
    }

    /// Deletes a value; a missing value is fine.
    pub fn delete_value(&self, name: &str) -> Result<(), String> {
        let n = wide(name);
        let err = unsafe { RegDeleteValueW(self.0, PCWSTR(n.as_ptr())) };
        if err == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        check(err, name)
    }

    pub fn subkeys(&self) -> Vec<String> {
        let mut names = Vec::new();
        for i in 0.. {
            let mut buf = [0u16; 256];
            let mut len = buf.len() as u32;
            let err = unsafe {
                RegEnumKeyExW(
                    self.0,
                    i,
                    Some(PWSTR(buf.as_mut_ptr())),
                    &mut len,
                    None,
                    None,
                    None,
                    None,
                )
            };
            if err != ERROR_SUCCESS {
                break;
            }
            names.push(String::from_utf16_lossy(&buf[..len as usize]));
        }
        names
    }
}

/// Deletes a key with everything below it; a missing key is fine.
pub fn delete_tree(root: HKEY, path: &str) -> Result<(), String> {
    let p = wide(path);
    let err = unsafe { RegDeleteTreeW(root, PCWSTR(p.as_ptr())) };
    if err == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    check(err, path)
}

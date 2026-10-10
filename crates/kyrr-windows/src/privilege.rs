//! Token privileges for writing the audio endpoint keys, which belong to
//! TrustedInstaller. With the backup and restore privileges enabled (an
//! elevated administrator has both, switched off), registry calls with
//! `REG_OPTION_BACKUP_RESTORE` bypass the keys' security without changing it.

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_NOT_ALL_ASSIGNED, HANDLE, LUID};
use windows::Win32::Security::{
    AdjustTokenPrivileges, LookupPrivilegeValueW, LUID_AND_ATTRIBUTES, SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES,
    TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use crate::reg::wide;

fn enable(name: &str) -> Result<(), String> {
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &mut token) }
        .map_err(|e| format!("process token: {e}"))?;
    let result = (|| {
        let n = wide(name);
        let mut luid = LUID::default();
        unsafe { LookupPrivilegeValueW(PCWSTR::null(), PCWSTR(n.as_ptr()), &mut luid) }
            .map_err(|e| format!("{name}: {e}"))?;
        let tp = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };
        unsafe { AdjustTokenPrivileges(token, false, Some(&tp), 0, None, None) }.map_err(|e| format!("{name}: {e}"))?;
        // AdjustTokenPrivileges succeeds even when the token lacks the privilege.
        if unsafe { GetLastError() } == ERROR_NOT_ALL_ASSIGNED {
            return Err(format!("{name}: not held (run as administrator)"));
        }
        Ok(())
    })();
    unsafe {
        let _ = CloseHandle(token);
    }
    result
}

pub fn enable_backup_restore() -> Result<(), String> {
    enable("SeBackupPrivilege")?;
    enable("SeRestorePrivilege")
}

//! Running kyrr's own executable elevated (Windows asks the user) for setup
//! commands, and waiting for it; starting an update's installer.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED};
use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
use windows::Win32::UI::WindowsAndMessaging::{SW_HIDE, SW_SHOWNORMAL};

use crate::reg::wide;

#[derive(Debug)]
pub enum ElevateError {
    /// The user said no to the permission prompt.
    Declined,
    Failed(String),
}

/// Quotes one command-line argument the way the C runtime parses it.
fn quote(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.extend(std::iter::repeat_n('\\', backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
    out
}

/// Starts this executable elevated with `args`, waits, returns its exit code.
pub fn run_self_elevated(args: &[&str]) -> Result<u32, ElevateError> {
    let exe = std::env::current_exe().map_err(|e| ElevateError::Failed(e.to_string()))?;
    let file = wide(&exe.to_string_lossy());
    let params = wide(&args.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" "));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    if let Err(e) = unsafe { ShellExecuteExW(&mut info) } {
        return Err(if e.code() == ERROR_CANCELLED.to_hresult() {
            ElevateError::Declined
        } else {
            ElevateError::Failed(e.message())
        });
    }
    if info.hProcess.is_invalid() {
        return Err(ElevateError::Failed("no process handle".into()));
    }
    let mut code = 1u32;
    unsafe {
        WaitForSingleObject(info.hProcess, INFINITE);
        let _ = GetExitCodeProcess(info.hProcess, &mut code);
        let _ = CloseHandle(info.hProcess);
    }
    Ok(code)
}

/// Runs `file` with `args` the way a double click would and waits for it.
/// A program that needs administrator rights (kyrr's installer) asks for
/// them itself, so it keeps a link to the signed-in user and can start kyrr
/// again as that user, not as administrator.
pub fn run(file: &std::path::Path, args: &[&str]) -> Result<u32, ElevateError> {
    let file = wide(&file.to_string_lossy());
    let params = wide(&args.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" "));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    if let Err(e) = unsafe { ShellExecuteExW(&mut info) } {
        return Err(if e.code() == ERROR_CANCELLED.to_hresult() {
            ElevateError::Declined
        } else {
            ElevateError::Failed(e.message())
        });
    }
    let mut code = 1u32;
    if !info.hProcess.is_invalid() {
        unsafe {
            WaitForSingleObject(info.hProcess, INFINITE);
            let _ = GetExitCodeProcess(info.hProcess, &mut code);
            let _ = CloseHandle(info.hProcess);
        }
    }
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::quote;

    #[test]
    fn quoting() {
        assert_eq!(quote("attach"), "attach");
        assert_eq!(
            quote(r"C:\Program Files\kyrr\kyrr_apo.dll"),
            r#""C:\Program Files\kyrr\kyrr_apo.dll""#
        );
        assert_eq!(quote(r#"a "b""#), r#""a \"b\"""#);
        assert_eq!(quote(r"C:\dir\ "), r#""C:\dir\ ""#);
        assert_eq!(quote(r"x y\"), r#""x y\\""#);
    }
}

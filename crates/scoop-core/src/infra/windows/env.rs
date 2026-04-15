#![allow(unsafe_code)]

#[cfg(windows)]
use windows::{
    Win32::{
        Foundation::{ERROR_SUCCESS, GetLastError, LPARAM, WPARAM},
        UI::WindowsAndMessaging::{
            HWND_BROADCAST, SEND_MESSAGE_TIMEOUT_FLAGS, SMTO_ABORTIFHUNG, SendMessageTimeoutW,
            WM_SETTINGCHANGE,
        },
    },
    core::{Error, HRESULT, h},
};

#[cfg(windows)]
pub fn broadcast_environment_change() -> anyhow::Result<()> {
    let mut result = 0usize;

    // Soundness: the string pointer is static, null-terminated UTF-16 and remains valid for the
    // duration of the call; the result pointer is valid for writes until the call returns.
    let status = unsafe {
        windows::Win32::Foundation::SetLastError(ERROR_SUCCESS);
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            WPARAM(0),
            LPARAM(h!("Environment").as_ptr() as isize),
            SEND_MESSAGE_TIMEOUT_FLAGS(SMTO_ABORTIFHUNG.0),
            5_000,
            Some(&mut result as *mut usize),
        )
    };

    if status.0 != 0 {
        return Ok(());
    }

    // Soundness: GetLastError is thread-local and safe to query immediately after the failed call.
    let error = unsafe { GetLastError() };
    if error == ERROR_SUCCESS {
        return Ok(());
    }

    Err(Error::from_hresult(HRESULT::from_win32(error.0)).into())
}

#[cfg(not(windows))]
pub fn broadcast_environment_change() -> anyhow::Result<()> {
    Ok(())
}

#![allow(unsafe_code)]

use std::path::Path;

#[cfg(windows)]
use anyhow::Context;
#[cfg(windows)]
use camino::Utf8Path;
#[cfg(windows)]
use windows::{
    Win32::{
        Foundation::RPC_E_CHANGED_MODE,
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize, IPersistFile,
        },
        UI::Shell::{IShellLinkW, ShellLink},
    },
    core::{Error, HSTRING, Interface},
};

#[cfg(windows)]
struct ComGuard {
    should_uninitialize: bool,
}

#[cfg(windows)]
impl ComGuard {
    fn new() -> anyhow::Result<Self> {
        // Soundness: COM initialization is process-global but thread-affine. This call only
        // touches the current thread, and the matching uninitialize happens in Drop when we own
        // the initialization result.
        let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if result.is_ok() {
            return Ok(Self {
                should_uninitialize: true,
            });
        }
        if result == RPC_E_CHANGED_MODE {
            return Ok(Self {
                should_uninitialize: false,
            });
        }
        Err(anyhow::Error::from(Error::from_hresult(result)))
            .context("failed to initialize COM for shortcut creation")
    }
}

#[cfg(windows)]
impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.should_uninitialize {
            // Soundness: we only call CoUninitialize when this guard observed a successful
            // CoInitializeEx on the current thread.
            unsafe {
                CoUninitialize();
            }
        }
    }
}

#[cfg(windows)]
pub fn create_shortcut_file(
    target: &str,
    shortcut: &Utf8Path,
    arguments: &str,
    icon: Option<&str>,
) -> anyhow::Result<()> {
    let _com = ComGuard::new()?;

    // Soundness: the ShellLink class ID is valid, no aggregation is requested, and the returned
    // COM interface is immediately wrapped in safe interface types.
    let link: IShellLinkW = unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }
        .map_err(anyhow::Error::from)
        .map_err(|error| error.context("failed to create ShellLink COM object"))?;

    let target_value = HSTRING::from(target);
    let working_directory = working_directory(target);
    let (icon_path, icon_index) = split_icon_location(icon);
    let shortcut_value = HSTRING::from(shortcut.as_str());

    // Soundness: all HSTRING-backed pointers remain alive for the duration of these COM calls, and
    // the interface values stay valid while the method invocations execute.
    unsafe {
        link.SetPath(&target_value)
            .map_err(anyhow::Error::from)
            .map_err(|error| error.context("failed to set shortcut target path"))?;
        if !arguments.is_empty() {
            let arguments_value = HSTRING::from(arguments);
            link.SetArguments(&arguments_value)
                .map_err(anyhow::Error::from)
                .map_err(|error| error.context("failed to set shortcut arguments"))?;
        }
        if let Some(working_directory) = working_directory.as_ref() {
            link.SetWorkingDirectory(working_directory)
                .map_err(anyhow::Error::from)
                .map_err(|error| error.context("failed to set shortcut working directory"))?;
        }
        if let Some(icon_path) = icon_path.as_ref() {
            link.SetIconLocation(icon_path, icon_index)
                .map_err(anyhow::Error::from)
                .map_err(|error| error.context("failed to set shortcut icon"))?;
        }
        let persist: IPersistFile = link
            .cast()
            .map_err(anyhow::Error::from)
            .map_err(|error| error.context("failed to access shortcut persistence interface"))?;
        persist
            .Save(&shortcut_value, true)
            .map_err(anyhow::Error::from)
            .map_err(|error| error.context("failed to save shortcut file"))?;
    }

    Ok(())
}

#[cfg(not(windows))]
pub fn create_shortcut_file(
    _target: &str,
    _shortcut: &camino::Utf8Path,
    _arguments: &str,
    _icon: Option<&str>,
) -> anyhow::Result<()> {
    anyhow::bail!("start menu shortcuts are only supported on Windows")
}

#[cfg(windows)]
fn working_directory(target: &str) -> Option<HSTRING> {
    Path::new(target)
        .parent()
        .and_then(|parent| parent.to_str())
        .map(HSTRING::from)
}

#[cfg(windows)]
fn split_icon_location(icon: Option<&str>) -> (Option<HSTRING>, i32) {
    let Some(icon) = icon.map(str::trim).filter(|value| !value.is_empty()) else {
        return (None, 0);
    };

    let Some((path, index)) = icon.rsplit_once(',') else {
        return (Some(HSTRING::from(icon)), 0);
    };
    let Some(parsed_index) = index.trim().parse::<i32>().ok() else {
        return (Some(HSTRING::from(icon)), 0);
    };
    let path = path.trim();
    if path.is_empty() {
        return (Some(HSTRING::from(icon)), 0);
    }
    (Some(HSTRING::from(path)), parsed_index)
}

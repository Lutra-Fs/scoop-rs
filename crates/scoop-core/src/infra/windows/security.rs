#![allow(unsafe_code)]

#[cfg(windows)]
use windows::{
    Win32::{
        Security::{
            AllocateAndInitializeSid, CheckTokenMembership, FreeSid, PSID, SECURITY_NT_AUTHORITY,
        },
        System::SystemServices::{DOMAIN_ALIAS_RID_ADMINS, SECURITY_BUILTIN_DOMAIN_RID},
    },
    core::{BOOL, Result as WindowsResult},
};

#[cfg(windows)]
struct AllocatedSid(PSID);

#[cfg(windows)]
impl AllocatedSid {
    fn administrators() -> WindowsResult<Self> {
        let mut sid = PSID::default();

        // Soundness: Windows writes a newly allocated SID into `sid`; the returned pointer is
        // owned by this wrapper and freed exactly once in Drop.
        unsafe {
            AllocateAndInitializeSid(
                &SECURITY_NT_AUTHORITY,
                2,
                SECURITY_BUILTIN_DOMAIN_RID as u32,
                DOMAIN_ALIAS_RID_ADMINS as u32,
                0,
                0,
                0,
                0,
                0,
                0,
                &mut sid,
            )?;
        }

        Ok(Self(sid))
    }

    fn as_psid(&self) -> PSID {
        self.0
    }
}

#[cfg(windows)]
impl Drop for AllocatedSid {
    fn drop(&mut self) {
        if !self.0.0.is_null() {
            // Soundness: `self.0` was allocated by AllocateAndInitializeSid and is freed exactly
            // once when this wrapper drops.
            unsafe {
                let _ = FreeSid(self.0);
            }
        }
    }
}

#[cfg(windows)]
pub fn is_elevated() -> anyhow::Result<bool> {
    let administrators = AllocatedSid::administrators()
        .map_err(anyhow::Error::from)
        .map_err(|error| error.context("failed to construct administrators SID"))?;
    let mut is_member = BOOL::from(false);

    // Soundness: a null token handle asks Windows to inspect the current effective token; the SID
    // pointer comes from `AllocatedSid` and remains valid for the duration of the call.
    unsafe {
        CheckTokenMembership(None, administrators.as_psid(), &mut is_member)
            .map_err(anyhow::Error::from)
            .map_err(|error| error.context("failed to test administrator membership"))?;
    }

    Ok(is_member.as_bool())
}

#[cfg(not(windows))]
pub fn is_elevated() -> anyhow::Result<bool> {
    Ok(true)
}

use std::{
    ffi::{OsStr, c_void},
    io,
    mem::{self, size_of},
    os::windows::ffi::OsStrExt,
    path::Path,
    ptr,
};

use anyhow::{Context, anyhow};
use windows::{
    Win32::{
        Foundation::{CloseHandle, ERROR_SUCCESS, HANDLE, HLOCAL, LocalFree, WIN32_ERROR},
        Security::{
            Authorization::{
                ConvertSecurityDescriptorToStringSecurityDescriptorW, ConvertSidToStringSidW,
                ConvertStringSecurityDescriptorToSecurityDescriptorW, GetNamedSecurityInfoW,
                SE_FILE_OBJECT, SetSecurityInfo,
            },
            DACL_SECURITY_INFORMATION, GetSecurityDescriptorDacl, GetTokenInformation,
            OBJECT_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
            TOKEN_QUERY, TOKEN_USER, TokenUser,
        },
        Storage::FileSystem::{
            CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_DELETE, FILE_SHARE_READ,
            FILE_SHARE_WRITE, OPEN_EXISTING, READ_CONTROL, WRITE_DAC,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    },
    core::{PCWSTR, PWSTR},
};

use super::AgreementAclReadback;

pub(super) fn apply_agreement_acl(path: &Path) -> anyhow::Result<()> {
    let user_sid = current_user_sid_string().context("read current Windows user SID")?;
    let sddl = expected_sddl(&user_sid);
    let sddl_wide = to_wide_null(OsStr::new(&sddl));
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl_wide.as_ptr()),
            1,
            &raw mut descriptor,
            None,
        )
        .context("convert agreement ACL SDDL")?;
    }
    let descriptor_guard = LocalAllocGuard(HLOCAL(descriptor.0));

    let mut dacl_present = windows::core::BOOL::default();
    let mut dacl = ptr::null_mut();
    let mut dacl_defaulted = windows::core::BOOL::default();
    unsafe {
        GetSecurityDescriptorDacl(
            descriptor,
            &raw mut dacl_present,
            &raw mut dacl,
            &raw mut dacl_defaulted,
        )
        .context("extract agreement DACL from security descriptor")?;
    }
    if !dacl_present.as_bool() || dacl.is_null() {
        return Err(anyhow!(
            "converted agreement security descriptor did not contain a DACL"
        ));
    }

    let path_wide = to_wide_null(path.as_os_str());
    let handle = unsafe {
        CreateFileW(
            PCWSTR(path_wide.as_ptr()),
            (WRITE_DAC | READ_CONTROL).0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
        .with_context(|| format!("open {} for WRITE_DAC", path.display()))?
    };
    let handle_guard = HandleGuard(handle);
    let security_info = DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION;
    let code = unsafe {
        SetSecurityInfo(
            handle_guard.0,
            SE_FILE_OBJECT,
            security_info,
            None,
            None,
            Some(dacl),
            None,
        )
    };
    check_win32(code, "SetSecurityInfo agreement DACL")?;
    drop(handle_guard);
    drop(descriptor_guard);
    Ok(())
}

pub(super) fn prepare_agreement_for_reset(path: &Path) -> anyhow::Result<()> {
    let user_sid = current_user_sid_string().context("read current Windows user SID")?;
    let sddl = reset_sddl(&user_sid);
    set_acl_from_sddl(path, &sddl)
}

pub(super) fn read_agreement_acl(path: &Path) -> anyhow::Result<AgreementAclReadback> {
    let user_sid = current_user_sid_string().context("read current Windows user SID")?;
    let expected = expected_sddl(&user_sid);
    let path_wide = to_wide_null(path.as_os_str());
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    let security_info = DACL_SECURITY_INFORMATION;
    let code = unsafe {
        GetNamedSecurityInfoW(
            PCWSTR(path_wide.as_ptr()),
            SE_FILE_OBJECT,
            security_info,
            None,
            None,
            None,
            None,
            &raw mut descriptor,
        )
    };
    check_win32(code, "GetNamedSecurityInfoW agreement DACL")?;
    let descriptor_guard = LocalAllocGuard(HLOCAL(descriptor.0));
    let sddl = security_descriptor_to_sddl(descriptor, security_info)
        .context("convert agreement security descriptor to SDDL")?;
    drop(descriptor_guard);
    let matches_expected_contract = sddl == expected;
    Ok(AgreementAclReadback {
        sddl,
        expected_sddl: expected,
        matches_expected_contract,
    })
}

#[cfg(test)]
pub(super) fn restore_current_user_full_control_for_test(path: &Path) -> anyhow::Result<()> {
    let user_sid = current_user_sid_string().context("read current Windows user SID")?;
    let sddl = reset_sddl(&user_sid);
    set_acl_from_sddl(path, &sddl)
}

fn set_acl_from_sddl(path: &Path, sddl: &str) -> anyhow::Result<()> {
    let sddl_wide = to_wide_null(OsStr::new(sddl));
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl_wide.as_ptr()),
            1,
            &raw mut descriptor,
            None,
        )
        .context("convert test restore SDDL")?;
    }
    let descriptor_guard = LocalAllocGuard(HLOCAL(descriptor.0));
    let mut dacl_present = windows::core::BOOL::default();
    let mut dacl = ptr::null_mut();
    let mut dacl_defaulted = windows::core::BOOL::default();
    unsafe {
        GetSecurityDescriptorDacl(
            descriptor,
            &raw mut dacl_present,
            &raw mut dacl,
            &raw mut dacl_defaulted,
        )
        .context("extract test restore DACL")?;
    }
    if !dacl_present.as_bool() || dacl.is_null() {
        return Err(anyhow!(
            "test restore security descriptor did not contain a DACL"
        ));
    }
    let path_wide = to_wide_null(path.as_os_str());
    let handle = unsafe {
        CreateFileW(
            PCWSTR(path_wide.as_ptr()),
            (WRITE_DAC | READ_CONTROL).0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
        .with_context(|| format!("open {} for test ACL restore", path.display()))?
    };
    let handle_guard = HandleGuard(handle);
    let code = unsafe {
        SetSecurityInfo(
            handle_guard.0,
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(dacl),
            None,
        )
    };
    check_win32(code, "SetSecurityInfo test restore DACL")?;
    drop(handle_guard);
    drop(descriptor_guard);
    Ok(())
}

fn current_user_sid_string() -> anyhow::Result<String> {
    let mut token = HANDLE::default();
    unsafe {
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token)
            .context("OpenProcessToken")?;
    }
    let token_guard = HandleGuard(token);
    let mut required = 0u32;
    let first =
        unsafe { GetTokenInformation(token_guard.0, TokenUser, None, 0, &raw mut required) };
    if first.is_err() && required == 0 {
        return Err(anyhow!(
            "GetTokenInformation TokenUser length probe returned zero bytes"
        ));
    }
    let word_count = (required as usize).div_ceil(size_of::<usize>());
    let mut buffer = vec![0usize; word_count];
    unsafe {
        GetTokenInformation(
            token_guard.0,
            TokenUser,
            Some(buffer.as_mut_ptr().cast::<c_void>()),
            required,
            &raw mut required,
        )
        .context("GetTokenInformation TokenUser")?;
    }
    let token_user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
    let mut sid_string = PWSTR::null();
    unsafe {
        ConvertSidToStringSidW(token_user.User.Sid, &raw mut sid_string)
            .context("ConvertSidToStringSidW")?;
    }
    let sid_guard = LocalAllocGuard(HLOCAL(sid_string.0.cast::<c_void>()));
    let sid = unsafe { sid_string.to_string() }.context("decode current user SID")?;
    drop(sid_guard);
    drop(token_guard);
    Ok(sid)
}

fn expected_sddl(user_sid: &str) -> String {
    // No explicit Everyone deny ACE is used here. A World/Everyone deny ACE
    // also matches the current user's token and would prevent the required
    // current-user read access; with a protected DACL, non-listed trustees
    // are denied by Windows' normal DACL evaluation.
    format!("D:PAI(A;;FA;;;SY)(A;;FR;;;{user_sid})")
}

fn reset_sddl(user_sid: &str) -> String {
    format!("D:P(A;;FA;;;SY)(A;;FA;;;{user_sid})")
}

fn security_descriptor_to_sddl(
    descriptor: PSECURITY_DESCRIPTOR,
    security_info: OBJECT_SECURITY_INFORMATION,
) -> anyhow::Result<String> {
    let mut sddl = PWSTR::null();
    unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(
            descriptor,
            1,
            security_info,
            &raw mut sddl,
            None,
        )
        .context("ConvertSecurityDescriptorToStringSecurityDescriptorW")?;
    }
    let sddl_guard = LocalAllocGuard(HLOCAL(sddl.0.cast::<c_void>()));
    let value = unsafe { sddl.to_string() }.context("decode SDDL")?;
    drop(sddl_guard);
    Ok(value)
}

fn check_win32(code: WIN32_ERROR, context: &'static str) -> anyhow::Result<()> {
    if code == ERROR_SUCCESS {
        return Ok(());
    }
    Err(io_error_from_win32(code)).with_context(|| context)
}

fn io_error_from_win32(code: WIN32_ERROR) -> io::Error {
    io::Error::from_raw_os_error(code.0.cast_signed())
}

fn to_wide_null(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

struct LocalAllocGuard(HLOCAL);

impl Drop for LocalAllocGuard {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            let _freed = unsafe { LocalFree(Some(mem::take(&mut self.0))) };
        }
    }
}

struct HandleGuard(HANDLE);

impl Drop for HandleGuard {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            let _closed = unsafe { CloseHandle(mem::take(&mut self.0)) };
        }
    }
}

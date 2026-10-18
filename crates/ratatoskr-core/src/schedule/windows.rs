use anyhow::{Context, Result, ensure};
use std::{
    io,
    mem::size_of,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    ptr,
};
use windows_sys::Win32::{
    Foundation::{ERROR_INSUFFICIENT_BUFFER, LocalFree},
    Security::{
        Authorization::ConvertSidToStringSidW, GetTokenInformation, TOKEN_QUERY, TOKEN_USER,
        TokenUser,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

pub(super) fn current_user_sid() -> Result<String> {
    let mut token = ptr::null_mut();
    // OpenProcessToken initializes an owned token handle on success. OwnedHandle
    // closes it on every return path; the current-process pseudo handle is borrowed.
    ensure!(
        unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } != 0,
        "Open scheduler user token: {}",
        io::Error::last_os_error()
    );
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut required = 0;
    // The first call intentionally asks for the buffer size, not token data.
    let probe = unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            ptr::null_mut(),
            0,
            &mut required,
        )
    };
    let error = io::Error::last_os_error();
    ensure!(
        probe == 0 && error.raw_os_error() == Some(ERROR_INSUFFICIENT_BUFFER as i32),
        "Size scheduler user token: {error}"
    );
    ensure!(
        required as usize >= size_of::<TOKEN_USER>(),
        "Incomplete user token size"
    );
    // TOKEN_USER contains a pointer, so a byte Vec does not provide the required
    // alignment. Allocate pointer-aligned storage, including the trailing SID.
    let mut buffer = vec![0usize; (required as usize).div_ceil(size_of::<usize>())];
    ensure!(
        unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                buffer.as_mut_ptr().cast(),
                required,
                &mut required,
            )
        } != 0,
        "Read scheduler user token: {}",
        io::Error::last_os_error()
    );
    // The successful call initialized TOKEN_USER and the SID it points to in
    // buffer. Keep that buffer alive until ConvertSidToStringSidW has copied it.
    let user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
    let mut text = ptr::null_mut();
    ensure!(
        unsafe { ConvertSidToStringSidW(user.User.Sid, &mut text) } != 0,
        "Format scheduler user SID: {}",
        io::Error::last_os_error()
    );
    // Windows returns a NUL-terminated UTF-16 string allocated with LocalAlloc.
    let mut length = 0;
    while unsafe { *text.add(length) } != 0 {
        length += 1;
    }
    let sid = String::from_utf16(unsafe { std::slice::from_raw_parts(text, length) })
        .context("Scheduler user SID is not valid UTF-16");
    let remaining = unsafe { LocalFree(text.cast()) };
    ensure!(
        remaining.is_null(),
        "Free scheduler user SID: {}",
        io::Error::last_os_error()
    );
    sid
}

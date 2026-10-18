//! The real user's home directory, from the passwd entry rather than `$HOME` or XDG, which the caller sets.

use std::{
    ffi::{CStr, OsStr},
    os::unix::ffi::OsStrExt,
    path::PathBuf,
};

use anyhow::{Result, bail};

pub fn home() -> Result<PathBuf> {
    let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut buf = vec![0 as libc::c_char; 16 * 1024];
    let mut found = std::ptr::null_mut();
    // SAFETY: every pointer refers to storage that outlives the call, and `buf.len()` is its size.
    let status = unsafe {
        libc::getpwuid_r(
            libc::getuid(),
            entry.as_mut_ptr(),
            buf.as_mut_ptr(),
            buf.len(),
            &mut found,
        )
    };
    if status != 0 || found.is_null() {
        bail!("no passwd entry for this user");
    }
    // SAFETY: getpwuid_r succeeded, so `entry` is filled in and `pw_dir` points into `buf`.
    let home = unsafe { CStr::from_ptr(entry.assume_init().pw_dir) };
    Ok(PathBuf::from(OsStr::from_bytes(home.to_bytes())))
}

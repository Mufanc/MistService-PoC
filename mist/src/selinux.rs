use nix::libc;
use std::ffi::{CStr, CString};
use std::os::fd::{AsFd, AsRawFd};
use syscalls::Errno;

const SELINUX_ATTR: &CStr = c"security.selinux";

pub fn fsetcon<F: AsFd>(file: F, context: &str) -> anyhow::Result<()> {
    let context = CString::new(context)?;

    Errno::result(unsafe {
        libc::fsetxattr(
            file.as_fd().as_raw_fd(),
            SELINUX_ATTR.as_ptr(),
            context.as_ptr() as _,
            context.as_bytes_with_nul().len(),
            0,
        )
    })?;

    Ok(())
}

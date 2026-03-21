use crate::build_args;
use crate::ext::AsBytes;
use crate::ptrace::Tracee;
use crate::selinux::fsetcon;
use log::{debug, error, info};
use memfd::{FileSeal, MemfdOptions};
use nix::libc::{RTLD_NOW, c_int, off64_t, size_t};
use nix::sys::signal::Signal;
use nix::unistd::Pid;
use scopeguard::defer;
use std::ffi::c_void;
use std::fs::File;
use std::io::{Seek, SeekFrom};
use std::os::fd::{AsFd, AsRawFd, FromRawFd, IntoRawFd};
use std::path::Path;
use std::{fs, io, ptr};
use uds::UnixSeqpacketConn;

#[repr(C)]
pub struct DlextInfo {
    pub flags: u64,
    pub reserved_addr: *const c_void,
    pub reserved_size: size_t,
    pub relro_fd: c_int,
    pub library_fd: c_int,
    pub library_fd_offset: off64_t,
    pub library_namespace: *const c_void,
}

pub unsafe fn ptrace_inject(
    pid: Pid,
    library: impl AsRef<Path>,
    idmap: File,
) -> anyhow::Result<()> {
    info!("injecting {:?} into {pid}", library.as_ref().display());

    let library_local = {
        let fd = MemfdOptions::default()
            .allow_sealing(true)
            .create("libmist.so")?;

        let mut src = File::open(library.as_ref())?;
        let mut dst = fd.as_file();

        io::copy(&mut src, &mut dst)?;
        dst.sync_data()?;
        dst.seek(SeekFrom::Start(0))?;

        fd.add_seals(&[
            FileSeal::SealGrow,
            FileSeal::SealShrink,
            FileSeal::SealWrite,
            FileSeal::SealSeal,
        ])?;

        fsetcon(fd.as_file(), "u:object_r:system_file:s0")?;

        fd
    };

    let tracee = Tracee::attach(pid)?;

    info!("ready to inject!");

    defer! {
        let _ = tracee.kill(Signal::SIGCONT);
        let _ = tracee.detach();
    }

    let connection = tracee.connect()?;
    let library_remote = tracee.install_fd(&connection, library_local.as_file().as_fd())?;

    info!("library_remote = {library_remote:?}");

    let info = DlextInfo {
        flags: 0x10, // ANDROID_DLEXT_USE_LIBRARY_FD
        reserved_addr: ptr::null(),
        reserved_size: 0,
        relro_fd: 0,
        library_fd: library_remote.as_raw_fd(),
        library_fd_offset: 0,
        library_namespace: ptr::null(),
    };

    let library_name_address = tracee.stack + size_of_val(&info);

    tracee.poke_data(tracee.stack, info.as_bytes())?;
    tracee.poke_data(library_name_address, c"libmist.so".as_bytes())?;

    let handle = tracee.call_remote_func(
        tracee.resolve("libdl.so", "android_dlopen_ext")?,
        build_args!(library_name_address, RTLD_NOW, tracee.stack),
    )?;

    tracee.poke_data(tracee.stack, c"init_mist".as_bytes())?;

    let address = tracee.call_remote_func(
        tracee.resolve("libdl.so", "dlsym")?,
        build_args!(handle, tracee.stack),
    )?;

    let seqpacket = unsafe { UnixSeqpacketConn::from_raw_fd(connection.local.into_raw_fd()) };

    seqpacket.send_fds(&[], &[idmap.as_raw_fd()])?;

    tracee.call_remote_func(
        address as _,
        build_args!(connection.remote.forget(), library_remote.forget()),
    )?;

    // let res = tracee.call_remote_func(
    //     tracee.resolve("libc.so", "madvise")?,
    //     build_args!(tracee.stack, *PAGE_SIZE, MADV_DONTNEED),
    // )?;
    //
    // debug!("madvise = {res}");

    let mut found = false;

    for line in fs::read_to_string(format!("/proc/{}/maps", pid.as_raw()))?.lines() {
        if line.contains("libmist.so") {
            found = true;
            debug!("{line}");
        }
    }

    if found {
        info!("done.")
    } else {
        error!("failed to inject")
    }

    Ok(())
}

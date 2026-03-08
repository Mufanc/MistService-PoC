use crate::constants::SERVICE_MANAGER_PATH;
use anyhow::{Context, bail};
use log::{debug, error, info};
use nix::errno::Errno;
use nix::libc;
use nix::libc::{PTRACE_GETREGSET, PTRACE_SETREGSET, RTLD_NOW, iovec, user_regs_struct};
use nix::sys::signal::Signal;
use nix::sys::uio::RemoteIoVec;
use nix::sys::wait::{WaitPidFlag, WaitStatus};
use nix::sys::{ptrace, signal, uio, wait};
use nix::unistd::Pid;
use procfs::ProcError;
use procfs::process::{MMapPath, ProcState, Process};
use r3solvr::{BasicResolver, SymbolResolver};
use std::ffi::{CString, c_int, c_long, c_void};
use std::io::IoSlice;
use std::mem::MaybeUninit;
use std::path::Path;
use std::time::Duration;
use std::{fs, thread};

pub trait WaitStatusExt {
    fn signal(&self) -> Option<Signal>;
}

impl WaitStatusExt for WaitStatus {
    fn signal(&self) -> Option<Signal> {
        match self {
            WaitStatus::Exited(_, _) => None,
            WaitStatus::Signaled(_, sig, _) => Some(*sig),
            WaitStatus::Stopped(_, sig) => Some(*sig),
            WaitStatus::PtraceEvent(_, sig, _) => Some(*sig),
            WaitStatus::PtraceSyscall(_) => None,
            WaitStatus::Continued(_) => None,
            WaitStatus::StillAlive => None,
        }
    }
}

#[derive(Clone)]
pub struct RegSet(user_regs_struct);

#[allow(unused)]
impl RegSet {
    const SIZE: usize = size_of::<user_regs_struct>();

    fn new(regs: user_regs_struct) -> Self {
        Self(regs)
    }

    fn as_ptr(&self) -> *const c_void {
        &self.0 as *const user_regs_struct as _
    }

    pub fn get_fp(&self) -> usize {
        self.0.regs[29] as _
    }

    pub fn get_sp(&self) -> usize {
        self.0.sp as _
    }

    pub fn set_sp(&mut self, sp: usize) {
        self.0.sp = sp as _
    }

    pub fn align_sp(&mut self) {
        self.0.sp &= !0xf;
    }

    pub fn get_pc(&self) -> usize {
        self.0.pc as _
    }

    pub fn set_pc(&mut self, pc: usize) {
        self.0.pc = pc as _;
    }

    pub fn get_arg(&self, index: usize) -> c_long {
        if index < 8 {
            self.0.regs[index] as _
        } else {
            unreachable!("up to 8 parameters can be passed through registers")
        }
    }

    pub fn set_arg(&mut self, index: usize, value: c_long) {
        if index < 8 {
            self.0.regs[index] = value as _
        } else {
            unreachable!("up to 8 parameters can be passed through registers")
        }
    }

    pub fn get_lr(&self) -> usize {
        self.0.regs[30] as _
    }

    pub fn set_lr(&mut self, address: usize) {
        self.0.regs[30] = address as _
    }

    pub fn return_value(&self) -> c_long {
        self.0.regs[0] as _
    }
}

fn wait(p: Pid) -> anyhow::Result<WaitStatus> {
    Ok(wait::waitpid(p, Some(WaitPidFlag::__WALL))?)
}

fn poke_data(p: Pid, addr: usize, data: &[u8]) -> anyhow::Result<()> {
    let iov_remote = RemoteIoVec {
        base: addr,
        len: data.len(),
    };
    let iov_local = IoSlice::new(data);

    uio::process_vm_writev(p, &[iov_local], &[iov_remote]).context("failed to write memory")?;

    Ok(())
}

fn ptrace_raw(p: Pid, request: c_int, addr: usize, data: usize) -> nix::Result<c_long> {
    Errno::result(unsafe { libc::ptrace(request, p.as_raw(), addr, data) })
}

fn load_regs(p: Pid) -> anyhow::Result<RegSet> {
    let mut regs: MaybeUninit<user_regs_struct> = MaybeUninit::uninit();
    let iov = iovec {
        iov_base: regs.as_mut_ptr() as _,
        iov_len: RegSet::SIZE,
    };

    ptrace_raw(
        p,
        PTRACE_GETREGSET,
        1, /* NT_PRSTATUS */
        &iov as *const _ as _,
    )?;

    Ok(RegSet::new(unsafe { regs.assume_init() }))
}

fn store_regs(p: Pid, regs: &RegSet) -> anyhow::Result<()> {
    let iov = iovec {
        iov_base: regs.as_ptr() as _,
        iov_len: RegSet::SIZE,
    };

    ptrace_raw(
        p,
        PTRACE_SETREGSET,
        1, /* NT_PRSTATUS */
        &iov as *const _ as _,
    )?;

    Ok(())
}

pub unsafe fn ptrace_inject(p: Pid, file: impl AsRef<Path>) -> anyhow::Result<()> {
    info!("injecting {:?} into {p}", file.as_ref().display());

    signal::kill(p, Signal::SIGSTOP)?;

    // spin wait
    let maps = {
        let sleep_duration = Duration::from_millis(10);

        loop {
            let proc = Process::new(p.as_raw())?;
            let state = proc.stat().and_then(|stat| stat.state());

            debug!("process state: {state:?}");

            match state {
                Ok(ProcState::Stopped) => break proc.maps()?,
                Ok(_) => {}
                Err(ProcError::NotFound(_)) => {}
                Err(err) => bail!(err),
            }

            thread::sleep(sleep_duration);
        }
    };

    ptrace::attach(p)?;
    signal::kill(p, Signal::SIGCONT)?;

    loop {
        let status = wait(p)?;
        let info = ptrace::getsiginfo(p);

        debug!("status: {status:?}, siginfo: {info:?}");

        // Signal-delivery-stop by SIGCONT
        if let WaitStatus::Stopped(_, Signal::SIGCONT) = status
            && info.is_ok()
        {
            break;
        }

        ptrace::cont(p, status.signal())?;
    }

    let stack_base = maps
        .iter()
        .find_map(|map| {
            if let MMapPath::Stack = map.pathname {
                Some(map.address.0 as usize)
            } else {
                None
            }
        })
        .context("failed to find stack base")?;

    let executable_base = maps
        .iter()
        .find_map(|map| {
            if let MMapPath::Path(pathname) = &map.pathname
                && pathname.as_os_str() == SERVICE_MANAGER_PATH
            {
                Some(map.address.0 as usize)
            } else {
                None
            }
        })
        .context("failed to find executable base")?;

    let libdl_info = maps
        .iter()
        .find_map(|map| {
            if let MMapPath::Path(pathname) = &map.pathname
                && pathname
                    .to_str()
                    .is_some_and(|pathname| pathname.ends_with("/libdl.so"))
            {
                Some((pathname.to_owned(), map.address.0 as usize))
            } else {
                None
            }
        })
        .context("failed to find libc base")?;

    info!(
        "stack base: {stack_base:#x}, executable base: {executable_base:#x}, libdl: {libdl_info:?}"
    );

    let resolver = BasicResolver::from_file(libdl_info.0)?;
    let dlopen = resolver.lookup_symbol("dlopen")?;

    info!("dlopen: {dlopen:?}");

    let file = file.as_ref().to_string_lossy().to_string();
    let file_cstr = CString::new(file.clone())?;

    // Todo: cleanup stack
    poke_data(p, stack_base, file_cstr.to_bytes_with_nul())?;

    let mut regs = load_regs(p)?;
    let backup = regs.clone();

    regs.align_sp();
    regs.set_pc(dlopen.addr + libdl_info.1);
    regs.set_arg(0, stack_base as c_long);
    regs.set_arg(1, RTLD_NOW as c_long);
    regs.set_lr(stack_base);

    store_regs(p, &regs)?;
    ptrace::cont(p, Signal::SIGCONT)?;

    let mut status = wait(p)?;

    loop {
        debug!("status = {status:?}");

        match status {
            WaitStatus::Stopped(_, Signal::SIGSEGV) => break,
            WaitStatus::Stopped(_, Signal::SIGCHLD) => {}
            WaitStatus::Stopped(_, Signal::SIGCONT) => {}
            _ => bail!("stopped by {status:?}, expected SIGSEGV"),
        }

        ptrace::cont(p, status.signal())?;
        status = wait(p)?;
    }

    regs = load_regs(p)?;

    if regs.get_pc() != stack_base {
        let address = regs.get_pc() as u64;
        let map = maps
            .iter()
            .find(|map| address >= map.address.0 && address < map.address.1);

        bail!("wrong return address: 0x{address:0>12x} in {map:?}");
    }

    store_regs(p, &backup)?;
    ptrace::detach(p, None)?;

    let mut found = false;

    for line in fs::read_to_string(format!("/proc/{}/maps", p.as_raw()))?.lines() {
        if line.contains(&file) {
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

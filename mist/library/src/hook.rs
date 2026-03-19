use crate::constants::{DUMP_FLAG_PRIORITY_HIDE, SERVICE_MANAGER_PATH};
use android_logger::Config;
use anyhow::{Context, bail};
use core::slice;
use log::{LevelFilter, error, info};
use memmap2::Mmap;
use nix::libc::uid_t;
use procfs::process::{MMapPath, MemoryMaps, Process};
use r3solvr::{BasicResolver, Query, SymbolResolver};
use std::ffi::{c_long, c_void};
use std::mem;
use std::os::fd::{FromRawFd, OwnedFd, RawFd};
use std::path::PathBuf;
use std::ptr;
use std::sync::OnceLock;
use uds::UnixSeqpacketConn;
use wisp::Wisp;

static IPC_THREAD_STATE_SELF_OR_NULL: OnceLock<extern "C" fn() -> *const c_void> = OnceLock::new();
static IPC_THREAD_STATE_GET_CALLING_UID: OnceLock<extern "C" fn(handle: *const c_void) -> uid_t> =
    OnceLock::new();

static IDMAP: OnceLock<Mmap> = OnceLock::new();

struct LibraryFinder {
    maps: MemoryMaps,
}

impl LibraryFinder {
    fn new() -> anyhow::Result<Self> {
        Ok(Self {
            maps: Process::myself()?.maps()?,
        })
    }

    fn find_library(
        &self,
        pattern: &str,
        suffix: bool,
    ) -> anyhow::Result<(PathBuf, *const c_void)> {
        let matches = |pathname: &PathBuf| {
            if suffix {
                pathname.to_string_lossy().ends_with(pattern)
            } else {
                pathname.to_string_lossy() == pattern
            }
        };

        self.maps
            .iter()
            .find_map(|map| {
                if let MMapPath::Path(pathname) = &map.pathname
                    && matches(pathname)
                {
                    Some((pathname.to_owned(), map.address.0 as *const c_void))
                } else {
                    None
                }
            })
            .context("cannot find library")
    }
}

fn query(symbol: &'static str) -> Query<'static> {
    Query::new(symbol).with_debugdata(true).with_prefix(true)
}

fn can_access(uid: uid_t) -> bool {
    if uid < 10000 {
        return true;
    }

    if uid >= 20000 {
        return false;
    }

    if let Some(idmap) = IDMAP.get() {
        let index = (uid - 10000) as usize;
        let byte = unsafe { ptr::read_volatile(idmap.as_ptr().add(index >> 3)) };

        byte & (1 << (index & 7)) != 0
    } else {
        false
    }
}

extern "C" fn intercept_list_service(args: *mut c_long) {
    let args = unsafe { slice::from_raw_parts_mut(args, 3) };

    let dump_priority = args[1] as i32;

    info!("listServices: dump priority = {dump_priority:0>32b}");

    if dump_priority & DUMP_FLAG_PRIORITY_HIDE != 0 {
        let mut keep = false;

        if let (Some(ipc_thread_state_self_or_null), Some(ipc_thread_state_get_calling_uid)) = (
            IPC_THREAD_STATE_SELF_OR_NULL.get(),
            IPC_THREAD_STATE_GET_CALLING_UID.get(),
        ) {
            let ipc_thread_state = ipc_thread_state_self_or_null();

            if !ipc_thread_state.is_null()
                && can_access(ipc_thread_state_get_calling_uid(ipc_thread_state))
            {
                keep = true;
            }
        }

        if !keep {
            args[1] = (dump_priority & !DUMP_FLAG_PRIORITY_HIDE) as _;
        }
    }
}

fn run_catching(seqpacket_fd: RawFd, library_fd: RawFd) -> anyhow::Result<()> {
    let connection = unsafe {
        OwnedFd::from_raw_fd(library_fd); // close library_fd
        UnixSeqpacketConn::from_raw_fd(seqpacket_fd)
    };

    let mut fds = [0; 1];

    connection.recv_fds(&mut [], &mut fds)?;

    let idmap_fd = unsafe { OwnedFd::from_raw_fd(fds[0]) };
    let idmap = unsafe { Mmap::map(&idmap_fd)? };

    IDMAP.get_or_init(|| idmap);

    let finder = LibraryFinder::new()?;

    let (_, executable_base) = finder.find_library(SERVICE_MANAGER_PATH, false)?;

    {
        let (libbinder_pathname, libbinder_base) = finder.find_library("/libbinder.so", true)?;
        let resolver = BasicResolver::from_file(libbinder_pathname)?;

        let ipc_thread_state_self_or_null_fn =
            resolver.lookup_symbol("_ZN7android14IPCThreadState10selfOrNullEv")?;

        let ipc_thread_state_get_calling_uid_fn =
            resolver.lookup_symbol("_ZNK7android14IPCThreadState13getCallingUidEv")?;

        unsafe {
            IPC_THREAD_STATE_SELF_OR_NULL.get_or_init(|| {
                mem::transmute(libbinder_base.byte_add(ipc_thread_state_self_or_null_fn.addr))
            });

            IPC_THREAD_STATE_GET_CALLING_UID.get_or_init(|| {
                mem::transmute(libbinder_base.byte_add(ipc_thread_state_get_calling_uid_fn.addr))
            });
        }
    }

    let list_service_fn = {
        let resolver = BasicResolver::from_file("/proc/self/exe")?;
        resolver.lookup_symbol(query("_ZN7android14ServiceManager12listServicesE"))?
    };

    let stub = unsafe {
        Wisp::intercept_fn(
            executable_base.byte_add(list_service_fn.addr),
            intercept_list_service,
        )
    };

    match stub {
        Ok(stub) => mem::forget(stub),
        Err(err) => bail!("failed to intercept `listServices`: {:?}", err),
    }

    Ok(())
}

#[unsafe(no_mangle)]
pub extern "C" fn init_mist(seqpacket_fd: RawFd, library_fd: RawFd) {
    android_logger::init_once(
        Config::default()
            .with_tag("Mist")
            .with_max_level(LevelFilter::Debug),
    );

    if let Err(err) = run_catching(seqpacket_fd, library_fd) {
        error!("{err:?}");
    }
}

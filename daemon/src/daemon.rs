use crate::daemon::mist::IMistService::{BnMistService, IMistService};
use crate::selinux::fsetcon;
use anyhow::bail;
use memmap2::MmapMut;
use mist_common::binder::AddServiceEx;
use rsbinder::{Interface, ProcessState, Status, StatusCode, hub};
use std::convert::Into;
use std::fs;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
use mist_common::constants::{DUMP_FLAG_PRIORITY_HIDE, MIST_SERVICE_NAME};

include!(concat!(env!("OUT_DIR"), "/mist.rs"));

static MIST_IDMAP_DIR: LazyLock<PathBuf> = LazyLock::new(|| "/data/adb/mist".into());
static MIST_IDMAP_FILE: LazyLock<PathBuf> = LazyLock::new(|| MIST_IDMAP_DIR.join("idmap"));

struct MistService {
    idmap: Mutex<MmapMut>,
}

impl MistService {
    fn new(idmap: File) -> anyhow::Result<Self> {
        let mmap = unsafe { MmapMut::map_mut(&idmap)? };

        Ok(Self {
            idmap: Mutex::new(mmap),
        })
    }
}

fn validate_uid(uid: i32) -> rsbinder::status::Result<usize> {
    if (10000..20000).contains(&uid) {
        Ok((uid - 10000) as usize)
    } else {
        Err(StatusCode::BadValue.into())
    }
}

impl Interface for MistService {
    fn dump(&self, writer: &mut dyn Write, _args: &[String]) -> rsbinder::Result<()> {
        let _ = writer.write("Hello, World!".as_bytes());
        Ok(())
    }
}

#[allow(non_snake_case)]
impl IMistService for MistService {
    fn descriptor() -> &'static str
    where
        Self: Sized,
    {
        "xyz.mufanc.IMistService"
    }

    fn idmapList(&self) -> rsbinder::status::Result<Vec<i32>> {
        let idmap = self.idmap.lock().unwrap();
        let mut result = Vec::new();

        for (byte_idx, &byte) in idmap.iter().enumerate() {
            if byte == 0 {
                continue;
            }

            for bit in 0..8u8 {
                let index = byte_idx * 8 + bit as usize;

                if index >= 10000 {
                    break;
                }

                if byte & (1 << (bit & 7)) != 0 {
                    result.push((index + 10000) as i32);
                }
            }
        }

        Ok(result)
    }

    fn idmapGet(&self, id: i32) -> rsbinder::status::Result<bool> {
        let index = validate_uid(id)?;
        let idmap = self.idmap.lock().unwrap();
        let byte = idmap[index >> 3];

        Ok(byte & (1 << (index & 7)) != 0)
    }

    fn idmapSet(&self, id: i32, value: bool) -> rsbinder::status::Result<()> {
        let index = validate_uid(id)?;
        let mut idmap = self.idmap.lock().unwrap();
        let byte_index = index >> 3;
        let bit_mask = 1u8 << (index & 7);

        if value {
            idmap[byte_index] |= bit_mask;
        } else {
            idmap[byte_index] &= !bit_mask;
        }

        idmap
            .flush()
            .map_err(|_| Status::from(StatusCode::Unknown))?;

        Ok(())
    }

    fn idmapClear(&self) -> rsbinder::status::Result<()> {
        let mut idmap = self.idmap.lock().unwrap();

        idmap.fill(0);
        idmap
            .flush()
            .map_err(|_| Status::from(StatusCode::Unknown))?;

        Ok(())
    }
}

pub fn prepare_idmap() -> anyhow::Result<(File, File)> {
    fs::create_dir_all(&*MIST_IDMAP_DIR)?;

    let file_rw = File::options()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&*MIST_IDMAP_FILE)?;

    file_rw.set_len(1250)?;
    fsetcon(&file_rw, "u:object_r:system_file:s0")?;

    let file_ro = File::options().read(true).open(&*MIST_IDMAP_FILE)?;

    Ok((file_rw, file_ro))
}

pub fn run(idmap: File) -> anyhow::Result<()> {
    ProcessState::init_default();
    ProcessState::start_thread_pool();

    let service = BnMistService::new_binder(MistService::new(idmap)?);
    hub::default().add_service(
        MIST_SERVICE_NAME,
        service.as_binder(),
        false,
        DUMP_FLAG_PRIORITY_HIDE,
    )?;

    ProcessState::join_thread_pool()?;
    bail!("wtf??")
}

fn with_service<T>(func: impl FnOnce(&dyn IMistService) -> anyhow::Result<T>) -> anyhow::Result<T> {
    ProcessState::init_default();

    let service = match hub::get_interface::<dyn IMistService>(MIST_SERVICE_NAME) {
        Ok(service) => service,
        Err(_) => bail!("Service not found, is the daemon running?"),
    };

    if service.as_binder().ping_binder().is_err() {
        bail!("Service is not responding")
    }

    func(service.as_ref())
}

pub fn idmap_list() -> anyhow::Result<Vec<i32>> {
    with_service(|service| Ok(service.idmapList()?))
}

pub fn idmap_get(id: i32) -> anyhow::Result<bool> {
    with_service(|service| Ok(service.idmapGet(id)?))
}

pub fn idmap_set(id: i32, value: bool) -> anyhow::Result<()> {
    with_service(|service| Ok(service.idmapSet(id, value)?))
}

pub fn idmap_clear() -> anyhow::Result<()> {
    with_service(|service| Ok(service.idmapClear()?))
}

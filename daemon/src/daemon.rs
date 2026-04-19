use crate::daemon::mist::IMistService::{BnMistService, IMistService, IMistServiceAsyncService};
use crate::selinux::fsetcon;
use anyhow::bail;
use mist_common::binder::AddServiceEx;
use mist_common::constants::{DUMP_FLAG_PRIORITY_HIDE, MIST_SERVICE_NAME};
use mist_common::idmap::{IDMAP_SIZE, IdmapWriter};
use clap::Subcommand;
use rsbinder::TokioRuntime;
use rsbinder::{Interface, ProcessState, StatusCode, hub};
use std::convert::Into;
use std::{fs, future};
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
use tokio::runtime::Handle;

include!(concat!(env!("OUT_DIR"), "/mist.rs"));

static MIST_IDMAP_DIR: LazyLock<PathBuf> = LazyLock::new(|| "/data/adb/mist".into());
static MIST_IDMAP_FILE: LazyLock<PathBuf> = LazyLock::new(|| MIST_IDMAP_DIR.join("idmap"));

fn current_rt() -> TokioRuntime<Handle> {
    TokioRuntime(Handle::current())
}

struct MistService {
    idmap: Mutex<IdmapWriter>,
}

impl MistService {
    fn new(idmap: File) -> anyhow::Result<Self> {
        let writer = unsafe { IdmapWriter::from_fd(&idmap)? };

        Ok(Self {
            idmap: Mutex::new(writer),
        })
    }
}

impl Interface for MistService {
    fn dump(&self, writer: &mut dyn Write, _args: &[String]) -> rsbinder::Result<()> {
        let _ = writer.write("Hello, World!\n".as_bytes());
        Ok(())
    }
}

#[allow(non_snake_case)]
#[async_trait::async_trait]
impl IMistServiceAsyncService for MistService {
    fn descriptor() -> &'static str
    where
        Self: Sized,
    {
        "xyz.mufanc.IMistService"
    }

    async fn idmapList(&self) -> rsbinder::status::Result<Vec<i32>> {
        let idmap = self.idmap.lock().unwrap();
        Ok(idmap.get_all().into_iter().map(|uid| uid as i32).collect())
    }

    async fn idmapGet(&self, id: i32) -> rsbinder::status::Result<bool> {
        let idmap = self.idmap.lock().unwrap();
        idmap
            .get(id as u32)
            .ok_or_else(|| StatusCode::BadValue.into())
    }

    async fn idmapSet(&self, id: i32, value: bool) -> rsbinder::status::Result<()> {
        let mut idmap = self.idmap.lock().unwrap();
        idmap
            .set(id as u32, value)
            .map_err(|_| StatusCode::Unknown.into())
    }

    async fn idmapClear(&self) -> rsbinder::status::Result<()> {
        let mut idmap = self.idmap.lock().unwrap();
        idmap.clear().map_err(|_| StatusCode::Unknown.into())
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

    file_rw.set_len(IDMAP_SIZE)?;
    fsetcon(&file_rw, "u:object_r:system_file:s0")?;

    let file_ro = File::options().read(true).open(&*MIST_IDMAP_FILE)?;

    Ok((file_rw, file_ro))
}

pub async fn run(idmap: File) -> anyhow::Result<()> {
    ProcessState::init_default();
    ProcessState::start_thread_pool();

    let service = BnMistService::new_async_binder(MistService::new(idmap)?, current_rt());

    hub::default().add_service(
        MIST_SERVICE_NAME,
        service.as_binder(),
        false,
        DUMP_FLAG_PRIORITY_HIDE,
    )?;

    future::pending::<()>().await;
    bail!("wtf??")
}

#[derive(Subcommand)]
pub enum IdmapCommands {
    #[command(about = "List all enabled UIDs")]
    List,
    #[command(about = "Get idmap value for a UID")]
    Get {
        #[arg(help = "UID (10000-19999)")]
        id: i32,
    },
    #[command(about = "Set idmap value for a UID")]
    Set {
        #[arg(help = "UID (10000-19999)")]
        id: i32,
        #[arg(action = clap::ArgAction::Set, help = "Enable or disable")]
        value: bool,
    },
    #[command(about = "Clear all idmap entries")]
    Clear,
}

pub fn handle_idmap_command(command: IdmapCommands) -> anyhow::Result<()> {
    ProcessState::init_default();

    let service = match hub::get_interface::<dyn IMistService>(MIST_SERVICE_NAME) {
        Ok(service) => service,
        Err(_) => bail!("Service not found, is the daemon running?"),
    };

    if service.as_binder().ping_binder().is_err() {
        bail!("Service is not responding")
    }

    match command {
        IdmapCommands::List => {
            let list = service.idmapList()?;
            for id in list {
                println!("{id}");
            }
        }
        IdmapCommands::Get { id } => {
            let value = service.idmapGet(id)?;
            println!("{value}");
        }
        IdmapCommands::Set { id, value } => {
            service.idmapSet(id, value)?;
        }
        IdmapCommands::Clear => {
            service.idmapClear()?;
        }
    }

    Ok(())
}

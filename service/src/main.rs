use std::io::Write;
use anyhow::bail;
use clap::{Parser, Subcommand};
use rsbinder::{hub, Binder, Interface, Parcel, ProcessState, Remotable, StatusCode, TransactionCode};
use rsbinder::hub::{DUMP_FLAG_PRIORITY_ALL, DUMP_FLAG_PRIORITY_DEFAULT};
use mist_common::binder::AddServiceEx;
use mist_common::constants::DUMP_FLAG_PRIORITY_HIDE;

const fn transaction_code(a: char, b: char, c: char) -> u32 {
    (('_' as u32) << 24) | ((a as u32) << 16) | ((b as u32) << 8) | (c as u32)
}

const TRANSACTION_CODE_SAMPLE: u32 = transaction_code('S', 'P', 'L');

#[derive(Parser)]
#[command(disable_help_subcommand = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    #[command(about = "List all services registered in the service manager")]
    List,
    #[command(about = "Start the sample service and register it with the service manager")]
    Service,
}

struct SampleService;

impl Interface for SampleService {
    fn dump(&self, writer: &mut dyn Write, _args: &[String]) -> rsbinder::Result<()> {
        let _ = writer.write("Hello from SampleService\n".as_bytes());
        Ok(())
    }
}

impl Remotable for SampleService {
    fn descriptor() -> &'static str
    where
        Self: Sized,
    {
        "xyz.mufanc.mist.sample"
    }

    fn on_transact(&self, code: TransactionCode, _reader: &mut Parcel, reply: &mut Parcel) -> rsbinder::Result<()> {
        let mut success = false;

        if code == TRANSACTION_CODE_SAMPLE {
            success = true;
            reply.write("Reply from SampleService")?;
        }

        if success {
            Ok(())
        } else {
            Err(StatusCode::UnknownTransaction)
        }
    }

    fn on_dump(&self, writer: &mut dyn Write, args: &[String]) -> rsbinder::Result<()> {
        self.dump(writer, args)
    }
}

fn run_list() -> anyhow::Result<()> {
    ProcessState::init_default();

    let services = hub::list_services(DUMP_FLAG_PRIORITY_ALL | DUMP_FLAG_PRIORITY_HIDE);

    for (index, service) in services.into_iter().enumerate() {
        println!("{}\t{}", index, service);
    }

    Ok(())
}

fn run_service() -> anyhow::Result<()> {
    ProcessState::init_default();
    ProcessState::start_thread_pool();

    let visible = Binder::new(SampleService);
    let hidden = Binder::new(SampleService);

    hub::default().add_service("mist/sample_visible", visible.as_binder(), false, DUMP_FLAG_PRIORITY_DEFAULT)?;
    hub::default().add_service("mist/sample_hidden", hidden.as_binder(), true, DUMP_FLAG_PRIORITY_HIDE)?;

    ProcessState::join_thread_pool()?;
    bail!("wtf??")
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::List => {
            run_list()?;
        }
        Commands::Service => {
            run_service()?;
        }
    }

    Ok(())
}

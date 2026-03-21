use android_logger::Config;
use clap::{Parser, Subcommand};
use log::LevelFilter;
use nix::unistd::Pid;
use std::env;

mod daemon;
mod ext;
mod inject;
mod properties;
mod ptrace;
mod resolver;
mod selinux;

#[derive(Parser)]
#[command(disable_help_subcommand = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    #[command(about = "Inject into servicemanager and start daemon (for internal use only)")]
    Inject {
        #[arg(help = "Path to the library file")]
        file: String,
    },
    #[command(about = "Manage idmap")]
    Idmap {
        #[command(subcommand)]
        command: IdmapCommands,
    },
}

#[derive(Subcommand)]
enum IdmapCommands {
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

fn main() -> anyhow::Result<()> {
    if env::var("MAGISK").is_ok() {
        android_logger::init_once(
            Config::default()
                .with_tag("Mist")
                .with_max_level(LevelFilter::Debug),
        );
    } else {
        env_logger::init();
    }

    let cli = Cli::parse();

    match cli.command {
        Commands::Inject { file } => {
            let pid: i32 = properties::get("init.svc_debug_pid.servicemanager")?.parse()?;
            let (idmap_rw, idmap_ro) = daemon::prepare_idmap()?;

            unsafe {
                inject::ptrace_inject(Pid::from_raw(pid), file, idmap_ro)?;
            }

            daemon::run(idmap_rw)?;
        }
        Commands::Idmap { command } => match command {
            IdmapCommands::List => {
                let list = daemon::idmap_list()?;
                for id in list {
                    println!("{id}");
                }
            }
            IdmapCommands::Get { id } => {
                let value = daemon::idmap_get(id)?;
                println!("{value}");
            }
            IdmapCommands::Set { id, value } => {
                daemon::idmap_set(id, value)?;
            }
            IdmapCommands::Clear => {
                daemon::idmap_clear()?;
            }
        },
    }

    Ok(())
}

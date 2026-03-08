use android_logger::Config;
use anyhow::Context;
use log::LevelFilter;
use nix::unistd::Pid;
use std::env;

mod constants;
mod inject;
mod properties;

fn main() -> anyhow::Result<()> {
    if env::var("MAGISK").is_ok() {
        android_logger::init_once(
            Config::default()
                .with_tag("Mist")
                .with_max_level(LevelFilter::Debug),
        )
    } else {
        env_logger::init();
    }

    let pid: i32 = properties::get("init.svc_debug_pid.servicemanager")?.parse()?;
    let file = env::args().nth(1).context("No file provided")?;

    unsafe {
        inject::ptrace_inject(Pid::from_raw(pid), file)?;
    }

    Ok(())
}

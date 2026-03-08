use nix::libc::uid_t;

pub const SERVICE_MANAGER_PATH: &str = "/system/bin/servicemanager";
pub const DUMP_FLAG_PRIORITY_HIDE: i32 = 1 << 24;

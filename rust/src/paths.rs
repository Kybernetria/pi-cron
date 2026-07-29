use std::path::PathBuf;

pub fn state_dir() -> PathBuf {
    std::env::var_os("PI_CRON_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("XDG_STATE_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home().join(".local/state"))
                .join("pi-cron")
        })
}
pub fn runtime_dir() -> PathBuf {
    std::env::var_os("PI_CRON_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("XDG_RUNTIME_DIR")
                .map(PathBuf::from)
                .map(|p| p.join("pi-cron"))
                .unwrap_or_else(|| state_dir().join("run"))
        })
}
pub fn database() -> PathBuf {
    std::env::var_os("PI_CRON_DB")
        .map(PathBuf::from)
        .unwrap_or_else(|| state_dir().join("jobs.sqlite"))
}
pub fn socket() -> PathBuf {
    std::env::var_os("PI_CRON_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| runtime_dir().join("daemon.sock"))
}
pub fn lock() -> PathBuf {
    runtime_dir().join("daemon.lock")
}
fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

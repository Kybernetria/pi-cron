use crate::{
    paths, repository::Repository, runner::Runner, scheduler::Scheduler, service::Service, socket,
};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    os::{
        fd::AsRawFd,
        unix::fs::{OpenOptionsExt, PermissionsExt},
    },
    path::Path,
    sync::Arc,
};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

pub async fn run() -> Result<(), String> {
    let _lock = SingletonLock::acquire(&paths::lock())?;
    let repository = Arc::new(Mutex::new(Repository::open(&paths::database())?));
    let cancel = CancellationToken::new();
    let runner = Runner::new(cancel.clone());
    let scheduler = Scheduler::new(repository.clone(), runner.clone(), cancel.clone());
    scheduler.start().await?;
    let service = Service::new(repository, scheduler.clone(), runner);
    let signal_cancel = cancel.clone();
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            let mut term =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("SIGTERM handler");
            tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
        signal_cancel.cancel();
    });
    let result = socket::serve(&paths::socket(), service, cancel.clone()).await;
    cancel.cancel();
    scheduler.stop().await;
    result
}

struct SingletonLock {
    _file: File,
}
impl SingletonLock {
    fn acquire(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
        }
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(path)
            .map_err(|e| e.to_string())?;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err("pi-cron daemon already running".into());
        }
        file.set_len(0).map_err(|e| e.to_string())?;
        write!(file, "{}", std::process::id()).map_err(|e| e.to_string())?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
        Ok(Self { _file: file })
    }
}

use crate::{
    service::Service,
    types::{DaemonError, DaemonRequest, DaemonResponse},
};
use std::{fs, os::unix::fs::PermissionsExt, path::Path};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

const REQUEST_LIMIT: usize = 1_048_576;

pub async fn serve(path: &Path, service: Service, cancel: CancellationToken) -> Result<(), String> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    if path.exists() {
        fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    let listener =
        UnixListener::bind(path).map_err(|e| format!("could not bind {}: {e}", path.display()))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    let mut handlers = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => { let (stream, _) = accepted.map_err(|e| e.to_string())?; let service = service.clone(); let cancel = cancel.clone(); handlers.spawn(async move { handle(stream, service, cancel).await; }); }
            _ = handlers.join_next(), if !handlers.is_empty() => {},
            _ = cancel.cancelled() => break,
        }
    }
    drop(listener);
    let _ = fs::remove_file(path);
    while handlers.join_next().await.is_some() {}
    Ok(())
}

async fn handle(mut stream: UnixStream, service: Service, cancel: CancellationToken) {
    let mut data = Vec::new();
    let mut buffer = [0u8; 8192];
    let response = loop {
        tokio::select! {
            read = stream.read(&mut buffer) => match read {
                Ok(0) => break failure("unknown", "INVALID_REQUEST", "request ended before newline"),
                Err(e) => break failure("unknown", "INVALID_REQUEST", &e.to_string()),
                Ok(n) => {
                    data.extend_from_slice(&buffer[..n]);
                    if data.len() > REQUEST_LIMIT { break failure("unknown", "REQUEST_TOO_LARGE", "request too large"); }
                    if let Some(index) = data.iter().position(|b| *b == b'\n') { break respond(&data[..index], service).await; }
                }
            },
            _ = cancel.cancelled() => break failure("unknown", "SHUTTING_DOWN", "daemon is shutting down"),
        }
    };
    if let Ok(encoded) = serde_json::to_vec(&response) {
        let _ = stream.write_all(&encoded).await;
        let _ = stream.write_all(b"\n").await;
    }
}
async fn respond(line: &[u8], service: Service) -> DaemonResponse {
    let parsed: Result<DaemonRequest, _> = serde_json::from_slice(line);
    match parsed {
        Ok(request) if !request.id.is_empty() && !request.method.is_empty() => {
            match service.dispatch(&request.method, request.params).await {
                Ok(result) => DaemonResponse {
                    id: request.id,
                    ok: true,
                    result: Some(result),
                    error: None,
                },
                Err(message) => failure(&request.id, "INVALID_REQUEST", &message),
            }
        }
        Ok(request) => failure(&request.id, "INVALID_REQUEST", "invalid request envelope"),
        Err(e) => failure(
            "unknown",
            "INVALID_REQUEST",
            &format!("invalid JSON request: {e}"),
        ),
    }
}
fn failure(id: &str, code: &str, message: &str) -> DaemonResponse {
    DaemonResponse {
        id: id.into(),
        ok: false,
        result: None,
        error: Some(DaemonError {
            code: code.into(),
            message: message.into(),
        }),
    }
}

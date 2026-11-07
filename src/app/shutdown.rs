use std::{future::Future, io};

pub(super) fn shutdown_signal() -> io::Result<impl Future<Output = io::Result<()>>> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let mut interrupt =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
        Ok(async move {
            let signal = tokio::select! {
                signal = terminate.recv() => signal,
                signal = interrupt.recv() => signal,
            };
            signal.ok_or_else(|| io::Error::other("gestionnaire de signaux arrêté"))
        })
    }
    #[cfg(not(unix))]
    Ok(tokio::signal::ctrl_c())
}

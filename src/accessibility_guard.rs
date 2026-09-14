//! Explicit, foreground GNOME accessibility hold-open. Never started by observation tools.
use anyhow::{bail, Context, Result};
use atspi::events::window::ActivateEvent;
use atspi_connection::AccessibilityConnection;
use futures_util::StreamExt;
use std::{future::Future, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    time::timeout,
};

const SCHEMA: &str = "org.gnome.desktop.interface";
const KEY: &str = "toolkit-accessibility";
const DEADLINE: Duration = Duration::from_secs(5);

async fn until_stopped<T>(
    stop: &mut (impl Future<Output = ()> + Unpin),
    operation: impl Future<Output = Result<T>>,
) -> Result<Option<T>> {
    tokio::select! {
        biased;
        _ = stop => Ok(None),
        result = operation => result.map(Some),
    }
}

async fn gsettings(args: Vec<&'static str>) -> Result<String> {
    let mut command = Command::new("gsettings");
    command.args(args);
    let output = crate::command_runner::output(command, "accessibility guard setting").await?;
    if !output.status.success() {
        bail!(
            "gsettings failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

async fn ensure_enabled<F, Fut>(run: &mut F) -> Result<bool>
where
    F: FnMut(Vec<&'static str>) -> Fut,
    Fut: Future<Output = Result<String>>,
{
    match run(vec!["get", SCHEMA, KEY]).await?.as_str() {
        "true" => Ok(false),
        "false" => {
            run(vec!["set", SCHEMA, KEY, "true"]).await?;
            if run(vec!["get", SCHEMA, KEY]).await? != "true" {
                bail!("accessibility guard could not verify toolkit-accessibility=true");
            }
            Ok(true)
        }
        _ => bail!("accessibility guard received a non-boolean toolkit-accessibility value"),
    }
}

pub(crate) async fn run() -> Result<()> {
    // Install stop handlers before any side effects. Closing this process never disables
    // another accessibility client or restores a stale snapshot over a user's new choice.
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let stop = async {
        tokio::select! { _ = interrupt.recv() => {}, _ = terminate.recv() => {} }
    };
    tokio::pin!(stop);
    eprintln!("Accessibility guard requested: keeps GNOME toolkit-accessibility enabled while running. Stop with Ctrl-C or SIGTERM before disabling accessibility.");
    let Some(connection) = until_stopped(&mut stop, async {
        timeout(DEADLINE, AccessibilityConnection::new())
            .await
            .context("accessibility guard connection timed out")?
            .map_err(Into::into)
    })
    .await?
    else {
        return Ok(());
    };
    if until_stopped(&mut stop, async {
        timeout(DEADLINE, connection.register_event::<ActivateEvent>())
            .await
            .context("accessibility listener registration timed out")?
            .map_err(Into::into)
    })
    .await?
    .is_none()
    {
        return Ok(());
    }
    // Subscribe narrowly and discard event contents. No focus changes, speech, or logging
    // of application names. Holding this connection registers us with the bus launcher.
    let mut events = connection.event_stream();
    let result = async {
        let mut monitor = Command::new("gsettings")
            .args(["monitor", SCHEMA, KEY])
            .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null())
            .kill_on_drop(true).spawn().context("start accessibility setting monitor")?;
        let stdout = monitor.stdout.take().context("setting monitor stdout missing")?;
        let mut lines = BufReader::new(stdout).lines();
        let result: Result<Option<()>> = until_stopped(&mut stop, async {
            ensure_enabled(&mut gsettings).await?;
            eprintln!("Accessibility guard active: passive listener registered; saved toolkit setting verified. No accessibility applications were enabled.");
            // Periodic readback also covers a change between monitor spawn and its
            // subscription becoming ready. Do not assume a monitor line proves state.
            let mut readback = tokio::time::interval(Duration::from_secs(1));
            loop {
                tokio::select! {
                    event = events.next() => {
                        if event.is_none() { bail!("accessibility event stream disconnected"); }
                        // Unrelated registry signals need not decode as our event type.
                    }
                    line = lines.next_line() => {
                        if line?.is_none() { bail!("accessibility setting monitor exited"); }
                        if ensure_enabled(&mut gsettings).await? {
                            eprintln!("Accessibility guard restored toolkit-accessibility=true.");
                        }
                    }
                    _ = readback.tick() => { ensure_enabled(&mut gsettings).await?; }
                }
            }
        }).await;
        let _ = monitor.start_kill();
        let _ = timeout(DEADLINE, monitor.wait()).await;
        result.map(|_| ())
    }.await;
    drop(events);
    let _ = timeout(DEADLINE, connection.deregister_event::<ActivateEvent>()).await;
    eprintln!("Accessibility guard stopped: no more setting writes; current saved setting left unchanged.");
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    async fn exercise(values: &[&str]) -> (Result<bool>, Vec<Vec<&'static str>>) {
        let mut values: VecDeque<_> = values.iter().map(|x| x.to_string()).collect();
        let mut calls = Vec::new();
        let result = ensure_enabled(&mut |args| {
            calls.push(args);
            std::future::ready(values.pop_front().context("unexpected command"))
        })
        .await;
        (result, calls)
    }

    #[tokio::test]
    async fn already_enabled_never_writes() {
        let (result, calls) = exercise(&["true"]).await;
        assert!(!result.unwrap());
        assert_eq!(calls, vec![vec!["get", SCHEMA, KEY]]);
    }

    #[tokio::test]
    async fn reset_is_reasserted_and_read_back() {
        let (result, calls) = exercise(&["false", "", "true"]).await;
        assert!(result.unwrap());
        assert_eq!(
            calls,
            vec![
                vec!["get", SCHEMA, KEY],
                vec!["set", SCHEMA, KEY, "true"],
                vec!["get", SCHEMA, KEY]
            ]
        );
    }

    #[tokio::test]
    async fn failed_readback_does_not_claim_success() {
        assert!(exercise(&["false", "", "false"]).await.0.is_err());
        assert!(exercise(&[]).await.0.is_err());
        let (result, calls) = exercise(&["unexpected"]).await;
        assert!(result.is_err());
        assert_eq!(calls.len(), 1);
    }

    #[tokio::test]
    async fn stop_during_read_does_not_write_or_claim_active() {
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
        let stop = async {
            let _ = stop_rx.await;
        };
        tokio::pin!(stop);
        let mut calls = Vec::new();
        let mut stop_tx = Some(stop_tx);
        let result = until_stopped(
            &mut stop,
            ensure_enabled(&mut |args| {
                calls.push(args);
                stop_tx.take().unwrap().send(()).unwrap();
                std::future::pending::<Result<String>>()
            }),
        )
        .await
        .unwrap();
        assert!(result.is_none());
        assert_eq!(calls, vec![vec!["get", SCHEMA, KEY]]);
    }
}

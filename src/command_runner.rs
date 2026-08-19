use anyhow::{anyhow, Context, Result};
use std::{
    io::{self, Read},
    os::{fd::AsRawFd, unix::process::CommandExt as _},
    process::{Command as StdCommand, Output, Stdio},
    thread,
    time::Instant as StdInstant,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::{Child, Command},
    sync::oneshot,
    task::JoinHandle,
    time::{Duration, Instant},
};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(2);
const REAP_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_COMMAND_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
const BLOCKING_POLL_INTERVAL: Duration = Duration::from_millis(10);
const MAX_BLOCKING_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_BLOCKING_DRAIN_BYTES: usize = 64 * 1024;

pub(crate) async fn output(command: Command, action: &str) -> Result<Output> {
    output_with_timeout(command, action, COMMAND_TIMEOUT).await
}

pub(crate) async fn output_with_timeout(
    command: Command,
    action: &str,
    timeout: Duration,
) -> Result<Output> {
    output_with_input(command, action, timeout, None).await
}

pub(crate) async fn output_with_stdin(
    command: Command,
    action: &str,
    timeout: Duration,
    input: Vec<u8>,
) -> Result<Output> {
    output_with_input(command, action, timeout, Some(input)).await
}

pub(crate) fn output_blocking(command: &mut StdCommand, action: &str) -> Result<Output> {
    output_blocking_with_timeout(command, action, COMMAND_TIMEOUT)
}

pub(crate) fn output_blocking_with_timeout(
    command: &mut StdCommand,
    action: &str,
    timeout: Duration,
) -> Result<Output> {
    command
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .with_context(|| format!("failed to {action}"))?;
    let pgid = i32::try_from(child.id()).context("child process id did not fit in i32")?;
    let mut stdout = child
        .stdout
        .take()
        .context("command stdout was not piped")?;
    let mut stderr = child
        .stderr
        .take()
        .context("command stderr was not piped")?;
    if let Err(error) =
        set_nonblocking(stdout.as_raw_fd()).and_then(|()| set_nonblocking(stderr.as_raw_fd()))
    {
        terminate_blocking_process(&mut child, pgid);
        return Err(error).with_context(|| format!("failed to configure {action} output pipes"));
    }

    let deadline = StdInstant::now() + timeout;
    let mut stdout_bytes = Vec::new();
    let mut stderr_bytes = Vec::new();
    let mut stdout_eof = false;
    let mut stderr_eof = false;
    loop {
        if !stdout_eof {
            match drain_nonblocking(&mut stdout, &mut stdout_bytes) {
                Ok(eof) => stdout_eof = eof,
                Err(error) => {
                    terminate_blocking_process(&mut child, pgid);
                    return Err(error)
                        .with_context(|| format!("failed to collect {action} stdout"));
                }
            }
        }
        if !stderr_eof {
            match drain_nonblocking(&mut stderr, &mut stderr_bytes) {
                Ok(eof) => stderr_eof = eof,
                Err(error) => {
                    terminate_blocking_process(&mut child, pgid);
                    return Err(error)
                        .with_context(|| format!("failed to collect {action} stderr"));
                }
            }
        }
        if stdout_eof && stderr_eof {
            match child.try_wait() {
                Ok(Some(status)) => {
                    return Ok(Output {
                        status,
                        stdout: stdout_bytes,
                        stderr: stderr_bytes,
                    });
                }
                Ok(None) => {}
                Err(error) => {
                    terminate_blocking_process(&mut child, pgid);
                    return Err(error).with_context(|| format!("failed to wait for {action}"));
                }
            }
        }
        if StdInstant::now() >= deadline {
            terminate_blocking_process(&mut child, pgid);
            return Err(timeout_error(action, timeout));
        }
        thread::sleep(BLOCKING_POLL_INTERVAL);
    }
}

async fn output_with_input(
    mut command: Command,
    action: &str,
    timeout: Duration,
    input: Option<Vec<u8>>,
) -> Result<Output> {
    command
        .kill_on_drop(true)
        .process_group(0)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = command
        .spawn()
        .with_context(|| format!("failed to {action}"))?;
    supervise_child(child, action, timeout, input).await
}

pub(crate) async fn output_child(child: Child, action: &str, timeout: Duration) -> Result<Output> {
    supervise_child(child, action, timeout, None).await
}

async fn supervise_child(
    mut child: Child,
    action: &str,
    timeout: Duration,
    input: Option<Vec<u8>>,
) -> Result<Output> {
    let pid = child
        .id()
        .and_then(|pid| i32::try_from(pid).ok())
        .context("spawned command did not have a usable process id")?;
    let mut process_group = ProcessGroupGuard::new(pid);
    let stdout = child
        .stdout
        .take()
        .context("command stdout was not piped")?;
    let stderr = child
        .stderr
        .take()
        .context("command stderr was not piped")?;
    let stdout_reader = tokio::spawn(read_pipe(stdout));
    let stderr_reader = tokio::spawn(read_pipe(stderr));
    let stdin_writer = match input {
        Some(input) => {
            let mut stdin = child.stdin.take().context("command stdin was not piped")?;
            Some(tokio::spawn(async move { stdin.write_all(&input).await }))
        }
        None => None,
    };

    let (cancel_tx, cancel_rx) = oneshot::channel();
    let (result_tx, result_rx) = oneshot::channel();
    let deadline = Instant::now() + timeout;
    let action = action.to_string();
    tokio::spawn(async move {
        supervise_command(
            child,
            &mut process_group,
            stdout_reader,
            stderr_reader,
            stdin_writer,
            cancel_rx,
            deadline,
            timeout,
            &action,
            result_tx,
        )
        .await;
    });

    let mut cancellation = CancelOnDrop(Some(cancel_tx));
    let result = result_rx
        .await
        .context("command supervisor stopped before returning a result")?;
    cancellation.0 = None;
    result
}

#[allow(clippy::too_many_arguments)]
async fn supervise_command(
    mut child: Child,
    process_group: &mut ProcessGroupGuard,
    mut stdout_reader: JoinHandle<std::io::Result<Vec<u8>>>,
    mut stderr_reader: JoinHandle<std::io::Result<Vec<u8>>>,
    mut stdin_writer: Option<JoinHandle<std::io::Result<()>>>,
    mut cancel_rx: oneshot::Receiver<()>,
    deadline: Instant,
    timeout: Duration,
    action: &str,
    result_tx: oneshot::Sender<Result<Output>>,
) {
    enum Completion {
        Finished(Result<Output>),
        Timeout,
        Cancelled,
    }

    let completion = {
        // Keep the leader waitable until every inherited output descriptor is
        // closed, so its process-group id cannot be reused during cleanup.
        let command_completion = async {
            let stdin = async {
                match stdin_writer.as_mut() {
                    Some(writer) => writer
                        .await
                        .context("command stdin writer task failed")?
                        .context("failed to write command stdin"),
                    None => Ok(()),
                }
            };
            let stdout = async {
                (&mut stdout_reader)
                    .await
                    .context("command stdout reader task failed")?
                    .context("failed to read command stdout")
            };
            let stderr = async {
                (&mut stderr_reader)
                    .await
                    .context("command stderr reader task failed")?
                    .context("failed to read command stderr")
            };
            let ((), stdout, stderr) = tokio::try_join!(stdin, stdout, stderr)?;
            let status = child.wait().await.context("failed to wait for command")?;
            Ok(Output {
                status,
                stdout,
                stderr,
            })
        };
        tokio::pin!(command_completion);
        tokio::select! {
            result = &mut command_completion => Completion::Finished(result),
            _ = tokio::time::sleep_until(deadline) => Completion::Timeout,
            _ = &mut cancel_rx => Completion::Cancelled,
        }
    };

    let result = match completion {
        Completion::Finished(Ok(output)) => {
            process_group.disarm();
            Ok(output)
        }
        Completion::Finished(Err(error)) => {
            terminate_command(
                &mut child,
                process_group,
                &mut stdout_reader,
                &mut stderr_reader,
                stdin_writer.as_mut(),
            )
            .await;
            Err(error).with_context(|| format!("failed to {action}"))
        }
        Completion::Timeout => {
            terminate_command(
                &mut child,
                process_group,
                &mut stdout_reader,
                &mut stderr_reader,
                stdin_writer.as_mut(),
            )
            .await;
            Err(timeout_error(action, timeout))
        }
        Completion::Cancelled => {
            terminate_command(
                &mut child,
                process_group,
                &mut stdout_reader,
                &mut stderr_reader,
                stdin_writer.as_mut(),
            )
            .await;
            Err(anyhow!("cancelled while trying to {action}"))
        }
    };
    let _ = result_tx.send(result);
}

async fn read_pipe(mut pipe: impl AsyncRead + Unpin) -> std::io::Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let length = pipe.read(&mut buffer).await?;
        if length == 0 {
            break;
        }
        if output.len().saturating_add(length) > MAX_COMMAND_OUTPUT_BYTES {
            return Err(std::io::Error::other(format!(
                "command output exceeded {MAX_COMMAND_OUTPUT_BYTES} bytes"
            )));
        }
        output.extend_from_slice(&buffer[..length]);
    }
    Ok(output)
}

fn set_nonblocking(fd: std::os::fd::RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn drain_nonblocking(reader: &mut impl Read, output: &mut Vec<u8>) -> io::Result<bool> {
    let mut buffer = [0_u8; 8192];
    let mut drained = 0;
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => return Ok(true),
            Ok(length) => {
                if output.len().saturating_add(length) > MAX_BLOCKING_OUTPUT_BYTES {
                    return Err(io::Error::other(format!(
                        "command output exceeded {MAX_BLOCKING_OUTPUT_BYTES} bytes"
                    )));
                }
                output.extend_from_slice(&buffer[..length]);
                drained += length;
                if drained >= MAX_BLOCKING_DRAIN_BYTES {
                    return Ok(false);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
}

fn terminate_blocking_process(child: &mut std::process::Child, pgid: i32) {
    unsafe {
        libc::kill(-pgid, libc::SIGKILL);
    }
    let _ = child.kill();
    let deadline = StdInstant::now() + REAP_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => return,
            Ok(None) if StdInstant::now() < deadline => thread::sleep(BLOCKING_POLL_INTERVAL),
            Ok(None) => return,
        }
    }
}

async fn terminate_command(
    child: &mut Child,
    process_group: &mut ProcessGroupGuard,
    stdout_reader: &mut JoinHandle<std::io::Result<Vec<u8>>>,
    stderr_reader: &mut JoinHandle<std::io::Result<Vec<u8>>>,
    stdin_writer: Option<&mut JoinHandle<std::io::Result<()>>>,
) {
    process_group.kill();
    let _ = child.start_kill();
    let _ = tokio::time::timeout(REAP_TIMEOUT, child.wait()).await;
    stdout_reader.abort();
    stderr_reader.abort();
    if let Some(stdin_writer) = stdin_writer {
        stdin_writer.abort();
    }
    process_group.disarm();
}

fn timeout_error(action: &str, timeout: Duration) -> anyhow::Error {
    anyhow!(
        "timed out after {} ms while trying to {action}",
        timeout.as_millis()
    )
}

struct ProcessGroupGuard {
    pgid: i32,
    armed: bool,
}

impl ProcessGroupGuard {
    fn new(pgid: i32) -> Self {
        Self { pgid, armed: true }
    }

    fn kill(&self) {
        if self.armed {
            unsafe {
                libc::kill(-self.pgid, libc::SIGKILL);
            }
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ProcessGroupGuard {
    fn drop(&mut self) {
        self.kill();
    }
}

struct CancelOnDrop(Option<oneshot::Sender<()>>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(cancel) = self.0.take() {
            let _ = cancel.send(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf, time::Instant};

    #[tokio::test]
    async fn timeout_kills_the_child_process() {
        let pid_path = temporary_pid_path("leader");
        let mut command = Command::new("sh");
        command.args([
            "-c",
            &format!("printf %s $$ > '{}'; exec sleep 60", pid_path.display()),
        ]);
        let started = Instant::now();

        let error = output_with_timeout(command, "run test child", Duration::from_millis(20))
            .await
            .unwrap_err();

        assert!(error.to_string().contains("timed out"));
        assert!(started.elapsed() < Duration::from_millis(500));
        let pid = wait_for_pid(&pid_path).await;
        wait_for_process_exit(pid).await;
        let _ = fs::remove_file(pid_path);
    }

    #[tokio::test]
    async fn timeout_kills_descendants_that_hold_output_pipes() {
        let leader_path = temporary_pid_path("group-leader");
        let descendant_path = temporary_pid_path("group-descendant");
        let mut command = Command::new("sh");
        command.args([
            "-c",
            &format!(
                "printf %s $$ > '{}'; sleep 60 & printf %s $! > '{}'; wait",
                leader_path.display(),
                descendant_path.display()
            ),
        ]);

        let error = output_with_timeout(command, "run process tree", Duration::from_millis(100))
            .await
            .unwrap_err();

        assert!(error.to_string().contains("timed out"));
        let leader = wait_for_pid(&leader_path).await;
        let descendant = wait_for_pid(&descendant_path).await;
        wait_for_process_exit(leader).await;
        wait_for_process_exit(descendant).await;
        let _ = fs::remove_file(leader_path);
        let _ = fs::remove_file(descendant_path);
    }

    #[tokio::test]
    async fn caller_cancellation_kills_the_process_group() {
        let leader_path = temporary_pid_path("cancel-leader");
        let descendant_path = temporary_pid_path("cancel-descendant");
        let mut command = Command::new("sh");
        command.args([
            "-c",
            &format!(
                "printf %s $$ > '{}'; sleep 60 & printf %s $! > '{}'; wait",
                leader_path.display(),
                descendant_path.display()
            ),
        ]);
        let task = tokio::spawn(output_with_timeout(
            command,
            "run cancellable process tree",
            Duration::from_secs(60),
        ));
        let leader = wait_for_pid(&leader_path).await;
        let descendant = wait_for_pid(&descendant_path).await;

        task.abort();
        let _ = task.await;

        wait_for_process_exit(leader).await;
        wait_for_process_exit(descendant).await;
        let _ = fs::remove_file(leader_path);
        let _ = fs::remove_file(descendant_path);
    }

    #[tokio::test]
    async fn cancellation_after_leader_exit_kills_pipe_holding_descendant() {
        let descendant_path = temporary_pid_path("post-exit-descendant");
        let mut command = Command::new("sh");
        command.args([
            "-c",
            &format!(
                "sleep 60 & printf %s $! > '{}'; exit 0",
                descendant_path.display()
            ),
        ]);
        let task = tokio::spawn(output_with_timeout(
            command,
            "collect descendant output",
            Duration::from_secs(60),
        ));
        let descendant = wait_for_pid(&descendant_path).await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(
            !task.is_finished(),
            "descendant should still hold output pipes"
        );

        task.abort();
        let _ = task.await;

        wait_for_process_exit(descendant).await;
        let _ = fs::remove_file(descendant_path);
    }

    #[tokio::test]
    async fn output_with_stdin_writes_and_closes_input() {
        let mut command = Command::new("sh");
        command.args(["-c", "cat"]);

        let output = output_with_stdin(
            command,
            "echo stdin",
            Duration::from_secs(1),
            b"portal input".to_vec(),
        )
        .await
        .expect("stdin command should complete");

        assert!(output.status.success());
        assert_eq!(output.stdout, b"portal input");
    }

    #[tokio::test]
    async fn command_output_is_bounded() {
        let mut command = Command::new("sh");
        command.args(["-c", "head -c 9000000 /dev/zero"]);

        let error =
            output_with_timeout(command, "capture excessive output", Duration::from_secs(5))
                .await
                .unwrap_err();

        assert!(format!("{error:#}").contains("output exceeded"));
    }

    #[tokio::test]
    async fn command_output_drains_large_stdout_and_stderr() {
        let mut command = Command::new("sh");
        command.args([
            "-c",
            "yes stdout | head -c 200000; yes stderr | head -c 200000 >&2",
        ]);

        let output = output_with_timeout(command, "collect noisy output", Duration::from_secs(5))
            .await
            .expect("noisy command should complete");

        assert!(output.status.success());
        assert!(output.stdout.len() >= 200_000);
        assert!(output.stderr.len() >= 200_000);
    }

    #[tokio::test]
    async fn default_blocking_timeout_kills_the_process_group() {
        let leader_path = temporary_pid_path("blocking-default-leader");
        let descendant_path = temporary_pid_path("blocking-default-descendant");
        let mut command = StdCommand::new("sh");
        command.args([
            "-c",
            &format!(
                "printf %s $$ > '{}'; sleep 60 & printf %s $! > '{}'; wait",
                leader_path.display(),
                descendant_path.display()
            ),
        ]);
        let started = Instant::now();

        let error = output_blocking(&mut command, "run blocking process tree").unwrap_err();

        assert!(error
            .to_string()
            .contains("timed out after 2000 ms while trying to run blocking process tree"));
        assert!(started.elapsed() < Duration::from_secs(4));
        let leader = fs::read_to_string(&leader_path)
            .expect("leader should record its pid")
            .parse()
            .expect("leader pid should be numeric");
        let descendant = fs::read_to_string(&descendant_path)
            .expect("descendant should record its pid")
            .parse()
            .expect("descendant pid should be numeric");
        wait_for_process_exit(leader).await;
        wait_for_process_exit(descendant).await;
        let _ = fs::remove_file(leader_path);
        let _ = fs::remove_file(descendant_path);
    }

    fn temporary_pid_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "computer-use-linux-command-runner-{label}-{}-{}.pid",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    async fn wait_for_pid(path: &PathBuf) -> u32 {
        for _ in 0..100 {
            if let Ok(value) = fs::read_to_string(path) {
                if let Ok(pid) = value.parse() {
                    return pid;
                }
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("child did not record its pid")
    }

    async fn wait_for_process_exit(pid: u32) {
        for _ in 0..100 {
            if !PathBuf::from(format!("/proc/{pid}")).exists() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        unsafe {
            libc::kill(pid as i32, libc::SIGKILL);
        }
        panic!("process {pid} was not killed and reaped")
    }
}

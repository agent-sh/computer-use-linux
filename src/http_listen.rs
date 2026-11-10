use crate::server::ComputerUseLinux;
use anyhow::{Context, Result};
use axum::extract::Request;
use axum::http::header::{AUTHORIZATION, WWW_AUTHENTICATE};
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::Router;
use hyper::server::conn::http1;
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::service::TowerToHyperService;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use sha2::Digest;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::signal::unix::{signal, SignalKind};

pub(crate) struct ListenConfig {
    pub addr: SocketAddr,
    pub allowed_hosts: Vec<String>,
    pub token: String,
}

pub(crate) fn token_digest(token: &str) -> [u8; 32] {
    sha2::Sha256::digest(token.as_bytes()).into()
}

/// Reads the bearer token, then removes both variables so children do not inherit them.
pub(crate) fn load_listen_token() -> Result<String> {
    const FILE_VAR: &str = "COMPUTER_USE_LINUX_MCP_TOKEN_FILE";
    const TOKEN_VAR: &str = "COMPUTER_USE_LINUX_MCP_TOKEN";
    let file = std::env::var_os(FILE_VAR);
    let value = std::env::var_os(TOKEN_VAR);
    std::env::remove_var(FILE_VAR);
    std::env::remove_var(TOKEN_VAR);
    let token = match (file, value) {
        (Some(_), Some(_)) => {
            anyhow::bail!("set only one of {FILE_VAR} and {TOKEN_VAR}")
        }
        (Some(path), None) => std::fs::read_to_string(&path).map_err(|error| {
            anyhow::anyhow!(
                "could not read {FILE_VAR} ({}): {error}",
                std::path::Path::new(&path).display()
            )
        })?,
        (None, Some(value)) => value.to_string_lossy().into_owned(),
        (None, None) => String::new(),
    };
    let token = token.trim();
    if token.is_empty() {
        anyhow::bail!("--listen needs {FILE_VAR} or {TOKEN_VAR}");
    }
    Ok(token.to_string())
}

pub(crate) fn validate_listen(config: &ListenConfig, shell_enabled: bool) -> Result<()> {
    if shell_enabled {
        anyhow::bail!("--listen is refused while COMPUTER_USE_LINUX_ENABLE_SHELL=1, because every HTTP client would get run_shell");
    }
    if !config.addr.ip().is_loopback() && config.allowed_hosts.is_empty() {
        anyhow::bail!(
            "--listen {} is not a loopback address; name the hostnames clients use with --allowed-host",
            config.addr
        );
    }
    Ok(())
}

pub(crate) fn bearer_matches(header: Option<&HeaderValue>, expected: &[u8; 32]) -> bool {
    let Some(header) = header.and_then(|value| value.to_str().ok()) else {
        return false;
    };
    let Some((scheme, credentials)) = header.split_once(' ') else {
        return false;
    };
    let credentials = credentials.trim();
    if !scheme.eq_ignore_ascii_case("bearer") || credentials.is_empty() {
        return false;
    }
    let presented = token_digest(credentials);
    let diff = presented
        .iter()
        .zip(expected)
        .fold(0u8, |diff, (left, right)| diff | (left ^ right));
    diff == 0
}

/// Hyper only enforces this with a timer, which `axum::serve` does not set.
pub(crate) const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(10);

/// Serves until SIGTERM or SIGINT and returns that signal's number.
pub(crate) async fn serve_http(server: ComputerUseLinux, config: ListenConfig) -> Result<i32> {
    let ListenConfig {
        addr,
        allowed_hosts: allowed_hosts_extra,
        token,
    } = config;
    let digest = token_digest(&token);
    drop(token);
    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("could not listen on {addr}"))?;
    let bound = listener
        .local_addr()
        .context("could not read the bound address")?;
    eprintln!("listening {bound}");
    let mut allowed_hosts = vec!["localhost".to_string(), "127.0.0.1".into(), "::1".into()];
    allowed_hosts.extend(allowed_hosts_extra);
    let http_config = StreamableHttpServerConfig::default()
        .with_allowed_hosts(allowed_hosts)
        .enforce_origin_validation();
    let shutdown = http_config.cancellation_token.clone();
    let service = StreamableHttpService::new(
        {
            let server = server.clone();
            move || Ok(server.clone())
        },
        Arc::new(LocalSessionManager::default()),
        http_config,
    );
    let app = Router::new()
        .nest_service("/mcp", service)
        .layer(middleware::from_fn(
            move |request: Request, next: Next| async move {
                if bearer_matches(request.headers().get(AUTHORIZATION), &digest) {
                    next.run(request).await
                } else {
                    unauthorized()
                }
            },
        ));
    let service = TowerToHyperService::new(app);
    let mut connections = tokio::task::JoinSet::new();
    let signal = loop {
        let stream = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => stream,
                Err(error) => {
                    // Usually fd exhaustion; back off instead of spinning, as axum::serve does.
                    eprintln!("[computer-use-linux] accept failed: {error}");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            },
            _ = terminate.recv() => break libc::SIGTERM,
            _ = interrupt.recv() => break libc::SIGINT,
            // Drop finished connections so the set does not grow for the server's lifetime.
            Some(_) = connections.join_next(), if !connections.is_empty() => continue,
        };
        let service = service.clone();
        let shutdown = shutdown.clone();
        connections.spawn(async move {
            let connection = http1::Builder::new()
                .timer(TokioTimer::new())
                .header_read_timeout(HEADER_READ_TIMEOUT)
                .serve_connection(TokioIo::new(stream), service);
            tokio::pin!(connection);
            tokio::select! {
                _ = connection.as_mut() => {}
                _ = shutdown.cancelled() => {
                    connection.as_mut().graceful_shutdown();
                    let _ = connection.await;
                }
            }
        });
    };
    drop(listener);
    // Ends rmcp's sessions and SSE streams so open connections can finish.
    shutdown.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(5), async {
        while connections.join_next().await.is_some() {}
    })
    .await;
    Ok(signal)
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(WWW_AUTHENTICATE, "Bearer")],
        "unauthorized",
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(value: &str) -> HeaderValue {
        HeaderValue::from_str(value).unwrap()
    }

    #[test]
    fn bearer_matches_any_case_and_only_the_token() {
        let expected = token_digest("secret");
        for good in [
            "Bearer secret",
            "bearer secret",
            "BEARER secret",
            "Bearer   secret",
        ] {
            assert!(bearer_matches(Some(&header(good)), &expected), "{good}");
        }
        for bad in [
            "Bearer secre",
            "Bearer secrets",
            "Bearer",
            "Bearer ",
            "Basic secret",
            "secret",
        ] {
            assert!(!bearer_matches(Some(&header(bad)), &expected), "{bad}");
        }
        assert!(!bearer_matches(None, &expected));
    }

    const FILE_VAR: &str = "COMPUTER_USE_LINUX_MCP_TOKEN_FILE";
    const TOKEN_VAR: &str = "COMPUTER_USE_LINUX_MCP_TOKEN";

    fn temp_token(name: &str, content: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("cul-token-{}-{name}", std::process::id()));
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn token_file_is_trimmed_and_removed_from_the_environment() {
        let _guard = crate::pixelflux::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let path = temp_token("trim", "secret\n");
        std::env::remove_var(TOKEN_VAR);
        std::env::set_var(FILE_VAR, &path);
        assert_eq!(load_listen_token().unwrap(), "secret");
        assert!(std::env::var_os(FILE_VAR).is_none());
        assert!(std::env::var_os(TOKEN_VAR).is_none());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn token_sources_refuse_ambiguity_and_absence() {
        let _guard = crate::pixelflux::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let load = |file: Option<&std::path::Path>, token: Option<&str>| {
            match file {
                Some(path) => std::env::set_var(FILE_VAR, path),
                None => std::env::remove_var(FILE_VAR),
            }
            match token {
                Some(value) => std::env::set_var(TOKEN_VAR, value),
                None => std::env::remove_var(TOKEN_VAR),
            }
            load_listen_token().unwrap_err().to_string()
        };
        let absent =
            "--listen needs COMPUTER_USE_LINUX_MCP_TOKEN_FILE or COMPUTER_USE_LINUX_MCP_TOKEN";
        let real = temp_token("both", "hunter2");
        let both = load(Some(&real), Some("hunter2"));
        assert_eq!(
            both,
            "set only one of COMPUTER_USE_LINUX_MCP_TOKEN_FILE and COMPUTER_USE_LINUX_MCP_TOKEN"
        );
        assert_eq!(load(None, None), absent);
        assert_eq!(load(None, Some(" \t\n")), absent);
        let blank = temp_token("blank", " \n");
        assert_eq!(load(Some(&blank), None), absent);
        let missing = std::env::temp_dir().join("cul-token-does-not-exist");
        let error = load(Some(&missing), None);
        assert!(
            error.starts_with(&format!(
                "could not read COMPUTER_USE_LINUX_MCP_TOKEN_FILE ({}): ",
                missing.display()
            )),
            "{error}"
        );
        assert!(!both.contains("hunter2"));
        std::fs::remove_file(real).unwrap();
        std::fs::remove_file(blank).unwrap();
    }

    #[test]
    fn listen_refusals() {
        let config = |addr: &str, hosts: &[&str]| ListenConfig {
            addr: addr.parse().unwrap(),
            allowed_hosts: hosts.iter().map(|host| host.to_string()).collect(),
            token: "secret".to_string(),
        };
        assert_eq!(
            validate_listen(&config("127.0.0.1:5200", &[]), true)
                .unwrap_err()
                .to_string(),
            "--listen is refused while COMPUTER_USE_LINUX_ENABLE_SHELL=1, because every HTTP client would get run_shell"
        );
        for addr in ["0.0.0.0:5200", "[::]:5200", "10.0.0.5:5200"] {
            assert_eq!(
                validate_listen(&config(addr, &[]), false)
                    .unwrap_err()
                    .to_string(),
                format!("--listen {addr} is not a loopback address; name the hostnames clients use with --allowed-host")
            );
        }
        for addr in ["127.0.0.1:5200", "[::1]:5200"] {
            validate_listen(&config(addr, &[]), false).unwrap();
        }
        validate_listen(&config("0.0.0.0:5200", &["a.svc"]), false).unwrap();
    }
}

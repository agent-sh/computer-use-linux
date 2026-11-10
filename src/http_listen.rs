use crate::server::ComputerUseLinux;
use anyhow::{Context, Result};
use axum::extract::Request;
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::Router;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use std::net::SocketAddr;
use std::sync::Arc;

pub(crate) struct ListenConfig {
    pub addr: SocketAddr,
    pub allowed_hosts: Vec<String>,
    pub token: String,
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

pub(crate) fn token_matches(header: Option<&HeaderValue>, token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    let Some(header) = header.and_then(|value| value.to_str().ok()) else {
        return false;
    };
    let Some(presented) = header.strip_prefix("Bearer ") else {
        return false;
    };
    if presented.len() != token.len() {
        return false;
    }
    let mut diff = 0u8;
    for (left, right) in presented.bytes().zip(token.bytes()) {
        diff |= left ^ right;
    }
    diff == 0
}

pub(crate) async fn serve_http(server: ComputerUseLinux, config: ListenConfig) -> Result<()> {
    let ListenConfig { addr, token, .. } = config;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("could not listen on {addr}"))?;
    let bound = listener
        .local_addr()
        .context("could not read the bound address")?;
    eprintln!("listening {bound}");
    let mut http_config = StreamableHttpServerConfig::default();
    http_config.json_response = true;
    let service = StreamableHttpService::new(
        {
            let server = server.clone();
            move || Ok(server.clone())
        },
        Arc::new(LocalSessionManager::default()),
        http_config,
    );
    let token = Arc::<str>::from(token);
    let app = Router::new()
        .nest_service("/mcp", service)
        .layer(middleware::from_fn(move |request: Request, next: Next| {
            let token = Arc::clone(&token);
            async move {
                if token_matches(request.headers().get(AUTHORIZATION), &token) {
                    next.run(request).await
                } else {
                    unauthorized()
                }
            }
        }));
    axum::serve(listener, app)
        .await
        .context("MCP HTTP listener stopped")?;
    Ok(())
}

fn unauthorized() -> Response {
    (StatusCode::UNAUTHORIZED, "unauthorized").into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(value: &str) -> HeaderValue {
        HeaderValue::from_str(value).unwrap()
    }

    #[test]
    fn token_matches_only_exact_bearer() {
        assert!(token_matches(Some(&header("Bearer secret")), "secret"));
        assert!(!token_matches(None, "secret"));
        assert!(!token_matches(Some(&header("Basic secret")), "secret"));
        assert!(!token_matches(Some(&header("Bearer other")), "secret"));
        assert!(!token_matches(
            Some(&header("Bearer secret-extra")),
            "secret"
        ));
        assert!(!token_matches(Some(&header("bearer secret")), "secret"));
        assert!(!token_matches(Some(&header("Bearer secret")), ""));
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

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

pub(crate) async fn serve_http(
    server: ComputerUseLinux,
    addr: SocketAddr,
    token: String,
) -> Result<()> {
    if token.is_empty() {
        anyhow::bail!("COMPUTER_USE_LINUX_MCP_TOKEN is required with --listen");
    }
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("could not listen on {addr}"))?;
    let bound = listener
        .local_addr()
        .context("could not read the bound address")?;
    eprintln!("listening {bound}");
    let mut config = StreamableHttpServerConfig::default();
    config.json_response = true;
    let service = StreamableHttpService::new(
        {
            let server = server.clone();
            move || Ok(server.clone())
        },
        Arc::new(LocalSessionManager::default()),
        config,
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
}

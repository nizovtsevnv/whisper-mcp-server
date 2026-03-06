use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::Router;
use tracing::info;
use whisper_rs::WhisperContext;

struct AppState {
    whisper_ctx: Arc<WhisperContext>,
    language: String,
    threads: i32,
    token: Option<String>,
    sessions: Mutex<HashSet<String>>,
}

pub async fn run_http_server(
    ctx: Arc<WhisperContext>,
    host: &str,
    port: u16,
    token: Option<String>,
    language: &str,
    threads: i32,
) {
    let state = Arc::new(AppState {
        whisper_ctx: ctx,
        language: language.to_string(),
        threads,
        token,
        sessions: Mutex::new(HashSet::new()),
    });

    let app = Router::new()
        .route("/mcp", post(handle_post).delete(handle_delete))
        .with_state(state);

    let addr = format!("{host}:{port}");
    info!("HTTP server listening on {addr}");

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("failed to bind HTTP listener");

    axum::serve(listener, app).await.expect("HTTP server error");
}

fn check_auth(state: &AppState, headers: &HeaderMap) -> Result<(), StatusCode> {
    let expected = match &state.token {
        Some(t) => t,
        None => return Ok(()),
    };

    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if let Some(token) = auth.strip_prefix("Bearer ") {
        if token == expected.as_str() {
            return Ok(());
        }
    }

    Err(StatusCode::UNAUTHORIZED)
}

fn get_session_id(headers: &HeaderMap) -> Option<String> {
    headers
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

async fn handle_post(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: String,
) -> Response {
    if let Err(status) = check_auth(&state, &headers) {
        return status.into_response();
    }

    // Check Content-Type
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !content_type.contains("application/json") {
        return (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Content-Type must be application/json",
        )
            .into_response();
    }

    // Peek at the method to determine if this is an initialize request
    let is_initialize = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v.get("method").and_then(|m| m.as_str()).map(String::from))
        .as_deref()
        == Some("initialize");

    if !is_initialize {
        // Require valid session ID for non-initialize requests
        let session_id = match get_session_id(&headers) {
            Some(id) => id,
            None => {
                return (StatusCode::BAD_REQUEST, "Missing Mcp-Session-Id header").into_response()
            }
        };

        let sessions = state.sessions.lock().expect("session lock poisoned");
        if !sessions.contains(&session_id) {
            return (StatusCode::BAD_REQUEST, "Invalid session ID").into_response();
        }
    }

    // Dispatch the request using spawn_blocking for whisper inference
    let ctx = Arc::clone(&state.whisper_ctx);
    let language = state.language.clone();
    let threads = state.threads;
    let request_body = body;

    let result = tokio::task::spawn_blocking(move || {
        crate::mcp::dispatch_request(&request_body, &ctx, &language, threads)
    })
    .await
    .expect("dispatch task panicked");

    match result {
        Some(response_json) => {
            let mut builder = Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "application/json");

            // If this was an initialize request, create a session
            if is_initialize {
                let session_id = uuid::Uuid::new_v4().to_string();
                state
                    .sessions
                    .lock()
                    .expect("session lock poisoned")
                    .insert(session_id.clone());
                builder = builder.header("mcp-session-id", &session_id);
            }

            builder.body(Body::from(response_json)).unwrap()
        }
        None => {
            // Notification — no response body needed
            (StatusCode::ACCEPTED, "").into_response()
        }
    }
}

async fn handle_delete(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if let Err(status) = check_auth(&state, &headers) {
        return status.into_response();
    }

    let session_id = match get_session_id(&headers) {
        Some(id) => id,
        None => return (StatusCode::BAD_REQUEST, "Missing Mcp-Session-Id header").into_response(),
    };

    let mut sessions = state.sessions.lock().expect("session lock poisoned");
    if sessions.remove(&session_id) {
        (StatusCode::OK, "Session terminated").into_response()
    } else {
        (StatusCode::NOT_FOUND, "Session not found").into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_auth_valid() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer secret123".parse().unwrap());

        // Simulate state with token
        let result = check_auth_standalone(Some("secret123"), &headers);
        assert!(result.is_ok());
    }

    #[test]
    fn test_check_auth_invalid() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer wrong".parse().unwrap());

        let result = check_auth_standalone(Some("secret123"), &headers);
        assert!(result.is_err());
    }

    #[test]
    fn test_check_auth_missing() {
        let headers = HeaderMap::new();

        let result = check_auth_standalone(Some("secret123"), &headers);
        assert!(result.is_err());
    }

    #[test]
    fn test_no_token_configured() {
        let headers = HeaderMap::new();

        let result = check_auth_standalone(None, &headers);
        assert!(result.is_ok());
    }

    #[test]
    fn test_session_lifecycle() {
        let sessions: Mutex<HashSet<String>> = Mutex::new(HashSet::new());

        // Add a session
        let session_id = uuid::Uuid::new_v4().to_string();
        sessions.lock().unwrap().insert(session_id.clone());
        assert!(sessions.lock().unwrap().contains(&session_id));

        // Remove the session
        assert!(sessions.lock().unwrap().remove(&session_id));
        assert!(!sessions.lock().unwrap().contains(&session_id));
    }

    #[test]
    fn test_missing_session_id() {
        let headers = HeaderMap::new();
        assert!(get_session_id(&headers).is_none());
    }

    #[test]
    fn test_get_session_id_present() {
        let mut headers = HeaderMap::new();
        headers.insert("mcp-session-id", "test-id-123".parse().unwrap());
        assert_eq!(get_session_id(&headers), Some("test-id-123".to_string()));
    }

    /// Standalone auth check that doesn't need AppState (avoids WhisperContext).
    fn check_auth_standalone(token: Option<&str>, headers: &HeaderMap) -> Result<(), StatusCode> {
        let expected = match token {
            Some(t) => t,
            None => return Ok(()),
        };

        let auth = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        if let Some(bearer_token) = auth.strip_prefix("Bearer ") {
            if bearer_token == expected {
                return Ok(());
            }
        }

        Err(StatusCode::UNAUTHORIZED)
    }
}

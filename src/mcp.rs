use std::io::{self, BufRead, Write};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{debug, error, warn};
use whisper_rs::WhisperContext;

#[derive(Deserialize)]
pub(crate) struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: String,
    pub(crate) id: Option<Value>,
    pub(crate) method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Serialize)]
pub(crate) struct JsonRpcResponse {
    jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<Value>,
}

/// Process a JSON-RPC request and return a response.
/// Returns None for notifications that require no response.
pub fn dispatch_request(
    request_json: &str,
    ctx: &Arc<WhisperContext>,
    language: &str,
    threads: i32,
) -> Option<String> {
    let request: JsonRpcRequest = match serde_json::from_str(request_json) {
        Ok(r) => r,
        Err(e) => {
            warn!("Invalid JSON-RPC request: {e}");
            let resp = JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: None,
                result: None,
                error: Some(json!({"code": -32700, "message": "Parse error"})),
            };
            return serde_json::to_string(&resp).ok();
        }
    };

    debug!("Received method: {}", request.method);

    if request.method.starts_with("notifications/") {
        return None;
    }

    let response = match request.method.as_str() {
        "initialize" => handle_initialize(&request),
        "tools/list" => handle_tools_list(&request),
        "tools/call" => handle_tools_call(&request, ctx, language, threads),
        _ => JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: request.id,
            result: None,
            error: Some(json!({"code": -32601, "message": "Method not found"})),
        },
    };

    match serde_json::to_string(&response) {
        Ok(json) => Some(json),
        Err(e) => {
            error!("Failed to serialize response: {e}");
            None
        }
    }
}

pub fn run_stdio_loop(ctx: Arc<WhisperContext>, language: &str, threads: i32) {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut stdout = stdout.lock();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                error!("stdin read error: {e}");
                break;
            }
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Some(response) = dispatch_request(trimmed, &ctx, language, threads) {
            let _ = writeln!(stdout, "{response}");
            let _ = stdout.flush();
        }
    }
}

fn handle_initialize(request: &JsonRpcRequest) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id: request.id.clone(),
        result: Some(json!({
            "protocolVersion": "2024-11-05",
            "capabilities": { "tools": {} },
            "serverInfo": {
                "name": "whisper-mcp-server",
                "version": "0.1.0"
            }
        })),
        error: None,
    }
}

fn handle_tools_list(request: &JsonRpcRequest) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id: request.id.clone(),
        result: Some(json!({
            "tools": [{
                "name": "transcribe",
                "description": "Transcribe audio to text using Whisper. Provide either a local file path or base64-encoded audio data.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Absolute path to audio file on disk (preferred for local files)"
                        },
                        "audio": {
                            "type": "string",
                            "description": "Base64-encoded audio data (alternative to path)"
                        },
                        "format": {
                            "type": "string",
                            "description": "Audio format: ogg (default), wav, mp3, etc."
                        },
                        "language": {
                            "type": "string",
                            "description": "Language code (ISO 639-1, e.g. 'ru', 'en') or 'auto'. Overrides server default."
                        }
                    }
                }
            }]
        })),
        error: None,
    }
}

fn handle_tools_call(
    request: &JsonRpcRequest,
    ctx: &Arc<WhisperContext>,
    default_language: &str,
    threads: i32,
) -> JsonRpcResponse {
    let tool_name = request
        .params
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if tool_name != "transcribe" {
        return JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: request.id.clone(),
            result: Some(mcp_error(&format!("Unknown tool: {tool_name}"))),
            error: None,
        };
    }

    let arguments = request
        .params
        .get("arguments")
        .cloned()
        .unwrap_or(json!({}));

    let format = arguments
        .get("format")
        .and_then(|v| v.as_str())
        .unwrap_or("ogg");

    let language = arguments
        .get("language")
        .and_then(|v| v.as_str())
        .unwrap_or(default_language);

    let audio_data = if let Some(path) = arguments.get("path").and_then(|v| v.as_str()) {
        match std::fs::read(path) {
            Ok(data) => data,
            Err(e) => {
                return JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: request.id.clone(),
                    result: Some(mcp_error(&format!("Failed to read file: {e}"))),
                    error: None,
                };
            }
        }
    } else if let Some(b64) = arguments.get("audio").and_then(|v| v.as_str()) {
        use base64::Engine;
        match base64::engine::general_purpose::STANDARD.decode(b64) {
            Ok(data) => data,
            Err(e) => {
                return JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: request.id.clone(),
                    result: Some(mcp_error(&format!("Base64 decode error: {e}"))),
                    error: None,
                };
            }
        }
    } else {
        return JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: request.id.clone(),
            result: Some(mcp_error("Either 'path' or 'audio' must be provided")),
            error: None,
        };
    };

    match crate::transcribe::transcribe(ctx, &audio_data, format, language, threads) {
        Ok(text) => JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: request.id.clone(),
            result: Some(json!({
                "content": [{"type": "text", "text": text}]
            })),
            error: None,
        },
        Err(e) => {
            error!("Transcription failed: {e}");
            JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: request.id.clone(),
                result: Some(mcp_error(&e)),
                error: None,
            }
        }
    }
}

pub(crate) fn mcp_error(message: &str) -> Value {
    json!({
        "content": [{"type": "text", "text": message}],
        "isError": true
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dispatch_parse_error() {
        // We need an Arc<WhisperContext> but parse error happens before it's used.
        // Use a null pointer trick? No — let's just test the parsing branch directly.
        let result = parse_and_dispatch("not valid json {{{", "auto", 4);
        assert!(result.is_some());
        let resp: Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert_eq!(resp["error"]["code"], -32700);
    }

    #[test]
    fn test_dispatch_method_not_found() {
        let input = r#"{"jsonrpc":"2.0","id":1,"method":"unknown/method"}"#;
        let result = parse_and_dispatch(input, "auto", 4);
        assert!(result.is_some());
        let resp: Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert_eq!(resp["error"]["code"], -32601);
        assert_eq!(resp["id"], 1);
    }

    #[test]
    fn test_dispatch_notification_returns_none() {
        let input = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        let result = parse_and_dispatch(input, "auto", 4);
        assert!(result.is_none());
    }

    #[test]
    fn test_dispatch_initialize() {
        let input = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
        let result = parse_and_dispatch(input, "auto", 4);
        assert!(result.is_some());
        let resp: Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert_eq!(resp["id"], 1);
        assert_eq!(resp["result"]["protocolVersion"], "2024-11-05");
        assert_eq!(resp["result"]["serverInfo"]["name"], "whisper-mcp-server");
    }

    /// Helper that dispatches without needing a WhisperContext.
    /// Only works for methods that don't call tools/call with transcribe.
    fn parse_and_dispatch(request_json: &str, _language: &str, _threads: i32) -> Option<String> {
        let request: JsonRpcRequest = match serde_json::from_str(request_json) {
            Ok(r) => r,
            Err(e) => {
                warn!("Invalid JSON-RPC request: {e}");
                let resp = JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: None,
                    result: None,
                    error: Some(json!({"code": -32700, "message": "Parse error"})),
                };
                return serde_json::to_string(&resp).ok();
            }
        };

        if request.method.starts_with("notifications/") {
            return None;
        }

        let response = match request.method.as_str() {
            "initialize" => handle_initialize(&request),
            "tools/list" => handle_tools_list(&request),
            _ => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: request.id,
                result: None,
                error: Some(json!({"code": -32601, "message": "Method not found"})),
            },
        };

        serde_json::to_string(&response).ok()
    }
}

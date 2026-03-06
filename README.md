# whisper-mcp-server

Speech-to-text MCP server powered by [whisper.cpp](https://github.com/ggerganov/whisper.cpp).

Standalone binary that exposes a `transcribe` tool over MCP (Model Context Protocol) via stdio or HTTP transport using JSON-RPC 2.0.

## Features

- **Tool `transcribe`** — accepts audio as a local file path or base64-encoded data
- **Native audio decoding** — WAV (hound), OGG/Opus (opus + ogg), MP3/FLAC/AAC/Vorbis (symphonia) — no ffmpeg required
- **Automatic resampling** — converts any sample rate to 16 kHz mono (whisper requirement)
- **Optional CUDA** — GPU acceleration via whisper.cpp CUDA backend
- **Language override** — per-request or server-wide language setting
- **HTTP transport** — MCP Streamable HTTP with Bearer token authentication and session management
- **Dual transport** — stdio (default) or HTTP mode via `--transport` flag

## Architecture

Single crate, five source modules:

| Module | Responsibility |
|---|---|
| `src/main.rs` | CLI parsing (clap), model loading, entry point, transport selection |
| `src/mcp.rs` | JSON-RPC 2.0 dispatch, MCP tool result helpers, stdio read/write loop |
| `src/http.rs` | HTTP transport: axum server, Bearer auth, session management |
| `src/transcribe.rs` | Whisper inference: state creation, parameter setup, segment extraction |
| `src/audio.rs` | Audio decoding (hound, symphonia, opus+ogg), linear resampling |

## CLI Arguments

```
whisper-mcp-server --model <PATH> [OPTIONS]

Options:
  --model <PATH>         Path to whisper model file (.bin) [required]
  --language <LANG>      Language for recognition (ISO 639-1, or "auto") [default: auto]
  --device <DEVICE>      Device: "cpu" or "cuda" [default: cpu]
  --threads <N>          Number of inference threads [default: 4]
  --transport <MODE>     Transport mode: stdio or http [default: stdio]
  --host <HOST>          Host to bind HTTP server [default: 127.0.0.1]
  --port <PORT>          Port for HTTP server [default: 8080]
  --token <TOKEN>        Bearer token for HTTP authentication (optional)
```

## Build

### Prerequisites

- Rust toolchain (stable)
- `musl` target for static builds: `rustup target add x86_64-unknown-linux-musl`
- CMake (whisper.cpp build dependency)
- libclang (bindgen dependency)

### Static musl build

```bash
cargo build --release --target x86_64-unknown-linux-musl
```

The resulting binary at `target/x86_64-unknown-linux-musl/release/whisper-mcp-server` is fully statically linked.

### CUDA build

CUDA requires glibc — use the default target:

```bash
cargo build --release --features cuda
```

## Runtime Dependencies

- **Whisper model file** — download from [huggingface.co/ggerganov/whisper.cpp](https://huggingface.co/ggerganov/whisper.cpp)

## Rust Dependencies

| Crate | Purpose |
|---|---|
| `whisper-rs` | Rust bindings for whisper.cpp |
| `clap` | CLI argument parsing |
| `serde`, `serde_json` | JSON serialization for MCP protocol |
| `base64` | Decoding base64-encoded audio input |
| `hound` | Native WAV file reading |
| `symphonia` | Native decoding of MP3, FLAC, AAC, Vorbis, PCM, ADPCM |
| `opus`, `ogg` | Native OGG/Opus decoding |
| `tracing`, `tracing-subscriber` | Structured logging to stderr |
| `axum` | HTTP server framework for MCP HTTP transport |
| `tokio` | Async runtime for HTTP transport |
| `uuid` | Session ID generation (UUID v4) |

## MCP Protocol

The server supports two transport modes:

- **stdio** (default) — communicates over stdin/stdout, one JSON object per line
- **HTTP** — MCP Streamable HTTP on `POST /mcp` and `DELETE /mcp`

### HTTP Transport

Start the server in HTTP mode:

```bash
whisper-mcp-server --model ggml-base.bin --transport http --port 8080 --token secret123
```

**Authentication**: when `--token` is set, all requests must include `Authorization: Bearer <token>`. Without `--token`, authentication is disabled.

**Sessions**: the `initialize` request returns an `Mcp-Session-Id` header. All subsequent requests must include this header. Sessions are terminated via `DELETE /mcp`.

Example session:

```bash
# Initialize (get session ID)
curl -s -D- -X POST http://127.0.0.1:8080/mcp \
  -H "Authorization: Bearer secret123" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}'

# Use the Mcp-Session-Id from the response headers for subsequent requests
curl -s -X POST http://127.0.0.1:8080/mcp \
  -H "Authorization: Bearer secret123" \
  -H "Content-Type: application/json" \
  -H "Mcp-Session-Id: <session-id>" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/list"}'

# Terminate session
curl -s -X DELETE http://127.0.0.1:8080/mcp \
  -H "Authorization: Bearer secret123" \
  -H "Mcp-Session-Id: <session-id>"
```

### Stdio Transport

The server communicates over stdin/stdout using JSON-RPC 2.0, one JSON object per line.

### Initialize

Request:
```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}
```

Response:
```json
{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"whisper-mcp-server","version":"0.1.0"}}}
```

### List tools

Request:
```json
{"jsonrpc":"2.0","id":2,"method":"tools/list"}
```

Response:
```json
{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"transcribe","description":"Transcribe audio to text using Whisper. Provide either a local file path or base64-encoded audio data.","inputSchema":{"type":"object","properties":{"path":{"type":"string","description":"Absolute path to audio file on disk (preferred for local files)"},"audio":{"type":"string","description":"Base64-encoded audio data (alternative to path)"},"format":{"type":"string","description":"Audio format: ogg (default), wav, mp3, etc."},"language":{"type":"string","description":"Language code (ISO 639-1, e.g. 'ru', 'en') or 'auto'. Overrides server default."}}}}]}}
```

### Transcribe (file path)

Request:
```json
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"transcribe","arguments":{"path":"/tmp/recording.wav","format":"wav"}}}
```

Response:
```json
{"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"Hello, world!"}]}}
```

## CI/CD

GitHub Actions workflows:

- **CI** (`ci.yml`) — runs `cargo fmt`, `cargo clippy`, `cargo test` on every push/PR to `main`/`develop`
- **Release** (`release.yml`) — builds static binaries for 5 targets on tag push (`v*`), uploads as release assets

Release process:
1. Create a git tag: `git tag v0.1.0 && git push --tags`
2. CI builds binaries for linux (glibc, musl), windows, macOS (x86_64, arm64)
3. Create a GitHub release from the tag — CI attaches build artifacts automatically

To update `cargoHash` in `flake.nix` after changing dependencies:
```bash
./scripts/update-cargo-hash.sh
```

## Usage

### Claude Desktop

Add to `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "whisper": {
      "command": "/path/to/whisper-mcp-server",
      "args": ["--model", "/path/to/ggml-base.bin"]
    }
  }
}
```

### Any MCP client

The server reads JSON-RPC requests from stdin and writes responses to stdout. Logs go to stderr. Connect any MCP-compatible client using stdio transport.

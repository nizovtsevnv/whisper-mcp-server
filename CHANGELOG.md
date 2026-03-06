# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [v0.1.1] - 2026-03-06

### Added
- MCP `transcribe` tool over JSON-RPC 2.0 with stdio and HTTP transports
- Audio decoding: WAV (hound), OGG/Opus, MP3/FLAC/AAC/Vorbis (symphonia)
- Automatic resampling to 16 kHz mono for Whisper inference
- HTTP transport with Bearer auth and session management
- Optional CUDA GPU acceleration via whisper.cpp backend
- `--version` CLI flag
- CI/CD: GitHub Actions for checks and multi-platform release builds

### Changed
- Rename `--token` CLI argument to `--auth`
- Extract `dispatch_request` in mcp.rs for transport reuse

[v0.1.1]: https://github.com/nizovtsevnv/whisper-mcp-server/releases/tag/v0.1.1

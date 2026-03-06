use std::sync::Arc;

use clap::Parser;
use tracing::info;
use whisper_rs::{WhisperContext, WhisperContextParameters};

mod audio;
mod http;
mod mcp;
mod transcribe;

#[derive(Parser)]
#[command(name = "whisper-mcp-server")]
struct Args {
    /// Path to whisper model file (.bin)
    #[arg(long)]
    model: String,

    /// Language for recognition (ISO 639-1, or "auto")
    #[arg(long, default_value = "auto")]
    language: String,

    /// Device: "cpu" or "cuda"
    #[arg(long, default_value = "cpu")]
    device: String,

    /// Number of inference threads
    #[arg(long, default_value_t = 4)]
    threads: i32,

    /// Transport mode: stdio or http
    #[arg(long, default_value = "stdio")]
    transport: String,

    /// Host to bind HTTP server (http transport only)
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// Port for HTTP server (http transport only)
    #[arg(long, default_value_t = 8080)]
    port: u16,

    /// Bearer token for HTTP authentication (http transport only)
    #[arg(long)]
    token: Option<String>,
}

fn main() {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    let args = Args::parse();

    info!("Loading model from {}", args.model);

    let ctx_params = WhisperContextParameters::default();

    let ctx = WhisperContext::new_with_params(&args.model, ctx_params)
        .expect("failed to load whisper model");
    let ctx = Arc::new(ctx);

    info!("Model loaded, starting MCP server");

    match args.transport.as_str() {
        "stdio" => mcp::run_stdio_loop(ctx, &args.language, args.threads),
        "http" => {
            let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
            rt.block_on(http::run_http_server(
                ctx,
                &args.host,
                args.port,
                args.token,
                &args.language,
                args.threads,
            ));
        }
        other => {
            eprintln!("Unknown transport: {other}");
            std::process::exit(1);
        }
    }
}

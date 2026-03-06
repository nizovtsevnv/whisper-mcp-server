use std::sync::Arc;

use tracing::info;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext};

use crate::audio;

pub fn transcribe(
    ctx: &Arc<WhisperContext>,
    audio_data: &[u8],
    format: &str,
    language: &str,
    threads: i32,
) -> Result<String, String> {
    let samples = match format {
        "wav" => audio::decode_wav(audio_data)?,
        "ogg" | "opus" => audio::decode_opus(audio_data)?,
        _ => audio::decode_symphonia(audio_data, format)?,
    };

    info!("Decoded {} samples for transcription", samples.len());

    let mut state = ctx
        .create_state()
        .map_err(|e| format!("Failed to create whisper state: {e}"))?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });

    if language != "auto" {
        params.set_language(Some(language));
    }
    params.set_n_threads(threads);
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);

    state
        .full(params, &samples)
        .map_err(|e| format!("Whisper inference error: {e}"))?;

    let n_segments = state
        .full_n_segments()
        .map_err(|e| format!("Failed to get segments: {e}"))?;

    let mut text = String::new();
    for i in 0..n_segments {
        let segment = state
            .full_get_segment_text(i)
            .map_err(|e| format!("Failed to get segment text: {e}"))?;
        text.push_str(&segment);
    }

    Ok(text.trim().to_string())
}

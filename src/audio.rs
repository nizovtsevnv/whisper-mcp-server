use std::io::Cursor;

use tracing::warn;

const TARGET_SAMPLE_RATE: u32 = 16_000;

pub fn decode_wav(data: &[u8]) -> Result<Vec<f32>, String> {
    let cursor = Cursor::new(data);
    let reader = hound::WavReader::new(cursor).map_err(|e| format!("WAV decode error: {e}"))?;
    let spec = reader.spec();
    let samples = read_wav_samples(reader)?;
    let mono = to_mono(&samples, spec.channels as usize);
    Ok(resample(&mono, spec.sample_rate, TARGET_SAMPLE_RATE))
}

fn read_wav_samples(mut reader: hound::WavReader<Cursor<&[u8]>>) -> Result<Vec<f32>, String> {
    let spec = reader.spec();
    match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<Vec<f32>, _>>()
            .map_err(|e| format!("WAV sample read error: {e}")),
        hound::SampleFormat::Int => {
            let max = (1u32 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / max))
                .collect::<Result<Vec<f32>, _>>()
                .map_err(|e| format!("WAV sample read error: {e}"))
        }
    }
}

pub fn decode_opus(data: &[u8]) -> Result<Vec<f32>, String> {
    use ogg::reading::PacketReader;
    use opus::{Channels, Decoder as OpusDecoder};

    let cursor = Cursor::new(data);
    let mut packet_reader = PacketReader::new(cursor);

    // First packet: OpusHead header
    let head_packet = packet_reader
        .read_packet()
        .map_err(|e| format!("OGG read error: {e}"))?
        .ok_or("Empty OGG stream")?;

    let head_data = &head_packet.data;
    if head_data.len() < 19 || &head_data[..8] != b"OpusHead" {
        return Err("Invalid OpusHead header".to_string());
    }

    let channel_count = head_data[9] as usize;
    if channel_count == 0 || channel_count > 2 {
        return Err(format!("Unsupported channel count: {channel_count}"));
    }

    let channels = if channel_count == 1 {
        Channels::Mono
    } else {
        Channels::Stereo
    };

    // Opus always decodes at 48 kHz
    let mut decoder = OpusDecoder::new(48_000, channels)
        .map_err(|e| format!("Opus decoder creation error: {e}"))?;

    // Second packet: OpusTags — skip
    let _ = packet_reader
        .read_packet()
        .map_err(|e| format!("OGG read error: {e}"))?;

    // Decode audio packets
    let mut all_samples: Vec<f32> = Vec::new();
    // Maximum Opus frame: 120 ms at 48 kHz = 5760 samples per channel
    let max_frame_size = 5760 * channel_count;
    let mut decode_buf = vec![0.0f32; max_frame_size];

    loop {
        match packet_reader.read_packet() {
            Ok(Some(packet)) => {
                let decoded = decoder
                    .decode_float(&packet.data, &mut decode_buf, false)
                    .map_err(|e| format!("Opus decode error: {e}"))?;
                all_samples.extend_from_slice(&decode_buf[..decoded * channel_count]);
            }
            Ok(None) => break,
            Err(e) => return Err(format!("OGG packet read error: {e}")),
        }
    }

    let mono = to_mono(&all_samples, channel_count);
    Ok(resample(&mono, 48_000, TARGET_SAMPLE_RATE))
}

pub fn decode_symphonia(data: &[u8], format_hint: &str) -> Result<Vec<f32>, String> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let cursor = Cursor::new(data.to_vec());
    let mss = MediaSourceStream::new(Box::new(cursor), Default::default());

    let mut hint = Hint::new();
    hint.with_extension(match format_hint {
        "aac" | "m4a" => "m4a",
        other => other,
    });

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| format!("Symphonia probe error: {e}"))?;

    let mut format_reader = probed.format;

    let track = format_reader
        .default_track()
        .ok_or("No audio track found")?;

    let track_id = track.id;
    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or("Unknown sample rate")?;
    let channels = track.codec_params.channels.map(|c| c.count()).unwrap_or(1);

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("Symphonia decoder creation error: {e}"))?;

    let mut all_samples: Vec<f32> = Vec::new();

    loop {
        let packet = match format_reader.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(e) => return Err(format!("Symphonia packet read error: {e}")),
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(symphonia::core::errors::Error::DecodeError(e)) => {
                warn!("Symphonia decode error (skipping packet): {e}");
                continue;
            }
            Err(e) => return Err(format!("Symphonia decode error: {e}")),
        };

        let mut sample_buf = SampleBuffer::<f32>::new(decoded.frames() as u64, *decoded.spec());
        sample_buf.copy_interleaved_ref(decoded);
        all_samples.extend_from_slice(sample_buf.samples());
    }

    let mono = to_mono(&all_samples, channels);
    Ok(resample(&mono, sample_rate, TARGET_SAMPLE_RATE))
}

fn to_mono(samples: &[f32], channels: usize) -> Vec<f32> {
    if channels == 1 {
        return samples.to_vec();
    }
    samples
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

fn resample(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate {
        return samples.to_vec();
    }
    let ratio = to_rate as f64 / from_rate as f64;
    let output_len = (samples.len() as f64 * ratio).ceil() as usize;
    let mut output = Vec::with_capacity(output_len);
    for i in 0..output_len {
        let src_idx = i as f64 / ratio;
        let idx = src_idx as usize;
        let frac = (src_idx - idx as f64) as f32;
        let sample = if idx + 1 < samples.len() {
            samples[idx] * (1.0 - frac) + samples[idx + 1] * frac
        } else if idx < samples.len() {
            samples[idx]
        } else {
            0.0
        };
        output.push(sample);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resample_same_rate() {
        let samples = vec![1.0, 2.0, 3.0];
        let result = resample(&samples, 16_000, 16_000);
        assert_eq!(result, samples);
    }

    #[test]
    fn test_resample_downsample() {
        let samples: Vec<f32> = (0..48_000).map(|i| (i as f32).sin()).collect();
        let result = resample(&samples, 48_000, 16_000);
        assert_eq!(result.len(), 16_000);
    }

    #[test]
    fn test_to_mono_stereo() {
        let samples = vec![1.0, 0.0, 0.5, 0.5, 0.0, 1.0];
        let result = to_mono(&samples, 2);
        assert_eq!(result, vec![0.5, 0.5, 0.5]);
    }

    #[test]
    fn test_to_mono_already_mono() {
        let samples = vec![1.0, 2.0, 3.0];
        let result = to_mono(&samples, 1);
        assert_eq!(result, samples);
    }

    #[test]
    fn test_decode_wav_valid() {
        let mut buf = Vec::new();
        {
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate: 16_000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            let cursor = Cursor::new(&mut buf);
            let mut writer = hound::WavWriter::new(cursor, spec).unwrap();
            for i in 0..160 {
                writer.write_sample((i * 100) as i16).unwrap();
            }
            writer.finalize().unwrap();
        }
        let result = decode_wav(&buf);
        assert!(result.is_ok());
        let samples = result.unwrap();
        assert_eq!(samples.len(), 160);
    }
}

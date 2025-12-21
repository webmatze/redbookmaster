// WAV format conversion for Red Book compliance
// Handles sample rate, bit depth, and channel conversion

use hound::{SampleFormat, WavReader, WavSpec, WavWriter};
use rubato::{FftFixedInOut, Resampler};
use std::path::Path;

use super::wav::{RED_BOOK_BITS_PER_SAMPLE, RED_BOOK_CHANNELS, RED_BOOK_SAMPLE_RATE};

/// Options for WAV conversion
#[derive(Debug, Clone)]
pub struct ConvertOptions {
    /// Target sample rate (default: 44100)
    pub sample_rate: u32,
    /// Target bit depth (default: 16)
    pub bits_per_sample: u16,
    /// Target channels (default: 2 for stereo)
    pub channels: u16,
    /// Dithering for bit depth reduction (recommended for 24->16 bit)
    pub dither: bool,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        Self {
            sample_rate: RED_BOOK_SAMPLE_RATE,
            bits_per_sample: RED_BOOK_BITS_PER_SAMPLE,
            channels: RED_BOOK_CHANNELS,
            dither: true,
        }
    }
}

impl ConvertOptions {
    /// Create options for Red Book compliant output
    pub fn red_book() -> Self {
        Self::default()
    }
}

/// Convert a WAV file to Red Book compliant format
pub fn convert_to_red_book(input: &Path, output: &Path) -> Result<ConversionResult, ConvertError> {
    convert_wav(input, output, ConvertOptions::red_book())
}

/// Convert a WAV file with custom options (streaming, memory-efficient)
pub fn convert_wav(
    input: &Path,
    output: &Path,
    options: ConvertOptions,
) -> Result<ConversionResult, ConvertError> {
    // Open input file
    let mut reader = WavReader::open(input)
        .map_err(|e| ConvertError::ReadError(input.to_path_buf(), e.to_string()))?;

    let input_spec = reader.spec();

    // Determine what conversions are needed
    let needs_resample = input_spec.sample_rate != options.sample_rate;
    let needs_bit_convert = input_spec.bits_per_sample != options.bits_per_sample;
    let needs_channel_convert = input_spec.channels != options.channels;

    // Write output file
    let output_spec = WavSpec {
        channels: options.channels,
        sample_rate: options.sample_rate,
        bits_per_sample: options.bits_per_sample,
        sample_format: SampleFormat::Int,
    };

    let mut writer = WavWriter::create(output, output_spec)
        .map_err(|e| ConvertError::WriteError(output.to_path_buf(), e.to_string()))?;

    // Process using streaming
    if needs_resample {
        // Use chunked processing with resampler
        convert_with_resampling(
            &mut reader,
            &mut writer,
            input_spec,
            &options,
            needs_channel_convert,
        )?;
    } else {
        // Simple streaming without resampling (much simpler)
        convert_streaming_no_resample(
            &mut reader,
            &mut writer,
            input_spec,
            &options,
            needs_channel_convert,
        )?;
    }

    writer
        .finalize()
        .map_err(|e| ConvertError::WriteError(output.to_path_buf(), e.to_string()))?;

    Ok(ConversionResult {
        input_sample_rate: input_spec.sample_rate,
        output_sample_rate: options.sample_rate,
        input_bits: input_spec.bits_per_sample,
        output_bits: options.bits_per_sample,
        input_channels: input_spec.channels,
        output_channels: options.channels,
        resampled: needs_resample,
        bit_converted: needs_bit_convert,
        channel_converted: needs_channel_convert,
    })
}

/// Streaming conversion without resampling (memory-efficient)
fn convert_streaming_no_resample<W: std::io::Write + std::io::Seek>(
    reader: &mut WavReader<std::io::BufReader<std::fs::File>>,
    writer: &mut WavWriter<W>,
    input_spec: WavSpec,
    options: &ConvertOptions,
    needs_channel_convert: bool,
) -> Result<(), ConvertError> {
    const CHUNK_FRAMES: usize = 8192;
    let in_channels = input_spec.channels as usize;
    let chunk_samples = CHUNK_FRAMES * in_channels;

    let mut dither_state: u32 = 12345;
    let mut next_dither = || -> f64 {
        if !options.dither {
            return 0.0;
        }
        dither_state = dither_state.wrapping_mul(1103515245).wrapping_add(12345);
        let r1 = (dither_state >> 16) as f64 / 32768.0 - 1.0;
        dither_state = dither_state.wrapping_mul(1103515245).wrapping_add(12345);
        let r2 = (dither_state >> 16) as f64 / 32768.0 - 1.0;
        (r1 + r2) * 0.5
    };

    // Process in chunks
    let mut input_buffer: Vec<f64> = Vec::with_capacity(chunk_samples);

    // Read and convert samples
    let sample_iter = create_sample_iterator(reader, input_spec)?;

    for sample in sample_iter {
        input_buffer.push(sample);

        if input_buffer.len() >= chunk_samples {
            // Process this chunk
            let converted = if needs_channel_convert {
                convert_channels(&input_buffer, input_spec.channels, options.channels)?
            } else {
                std::mem::take(&mut input_buffer)
            };

            write_chunk_to_wav(writer, &converted, options.bits_per_sample, &mut next_dither)?;

            if !needs_channel_convert {
                input_buffer = Vec::with_capacity(chunk_samples);
            } else {
                input_buffer.clear();
            }
        }
    }

    // Process remaining samples
    if !input_buffer.is_empty() {
        let converted = if needs_channel_convert {
            convert_channels(&input_buffer, input_spec.channels, options.channels)?
        } else {
            input_buffer
        };
        write_chunk_to_wav(writer, &converted, options.bits_per_sample, &mut next_dither)?;
    }

    Ok(())
}

/// Streaming conversion with resampling (memory-efficient)
fn convert_with_resampling<W: std::io::Write + std::io::Seek>(
    reader: &mut WavReader<std::io::BufReader<std::fs::File>>,
    writer: &mut WavWriter<W>,
    input_spec: WavSpec,
    options: &ConvertOptions,
    needs_channel_convert: bool,
) -> Result<(), ConvertError> {
    let in_channels = input_spec.channels as usize;
    let out_channels = options.channels as usize;
    let working_channels = if needs_channel_convert { out_channels } else { in_channels };

    // Create resampler
    let chunk_size = 1024;
    let mut resampler = FftFixedInOut::<f64>::new(
        input_spec.sample_rate as usize,
        options.sample_rate as usize,
        chunk_size,
        working_channels,
    )
    .map_err(|e| ConvertError::ResampleError(e.to_string()))?;

    let frames_needed = resampler.input_frames_next();
    let input_chunk_samples = frames_needed * in_channels;

    let mut dither_state: u32 = 12345;
    let mut next_dither = || -> f64 {
        if !options.dither {
            return 0.0;
        }
        dither_state = dither_state.wrapping_mul(1103515245).wrapping_add(12345);
        let r1 = (dither_state >> 16) as f64 / 32768.0 - 1.0;
        dither_state = dither_state.wrapping_mul(1103515245).wrapping_add(12345);
        let r2 = (dither_state >> 16) as f64 / 32768.0 - 1.0;
        (r1 + r2) * 0.5
    };

    // Pre-allocate reusable buffers for resampling
    let mut channel_buffers: Vec<Vec<f64>> = vec![Vec::with_capacity(frames_needed); working_channels];
    let mut input_buffer: Vec<f64> = Vec::with_capacity(input_chunk_samples);

    // Read samples
    let sample_iter = create_sample_iterator(reader, input_spec)?;

    for sample in sample_iter {
        input_buffer.push(sample);

        if input_buffer.len() >= input_chunk_samples {
            // Convert channels if needed
            let working_buffer = if needs_channel_convert {
                convert_channels(&input_buffer, input_spec.channels, options.channels)?
            } else {
                std::mem::take(&mut input_buffer)
            };

            // Deinterleave into channel buffers
            for ch in 0..working_channels {
                channel_buffers[ch].clear();
                for frame in 0..frames_needed {
                    let idx = frame * working_channels + ch;
                    if idx < working_buffer.len() {
                        channel_buffers[ch].push(working_buffer[idx]);
                    } else {
                        channel_buffers[ch].push(0.0);
                    }
                }
            }

            // Resample
            let output_chunk = resampler
                .process(&channel_buffers, None)
                .map_err(|e| ConvertError::ResampleError(e.to_string()))?;

            // Interleave and write
            let output_frames = output_chunk.get(0).map(|c| c.len()).unwrap_or(0);
            let mut interleaved = Vec::with_capacity(output_frames * working_channels);
            for frame in 0..output_frames {
                for ch in &output_chunk {
                    if frame < ch.len() {
                        interleaved.push(ch[frame]);
                    }
                }
            }

            write_chunk_to_wav(writer, &interleaved, options.bits_per_sample, &mut next_dither)?;

            if !needs_channel_convert {
                input_buffer = Vec::with_capacity(input_chunk_samples);
            } else {
                input_buffer.clear();
            }
        }
    }

    // Process remaining samples (pad to frames_needed)
    if !input_buffer.is_empty() {
        // Convert channels if needed
        let mut working_buffer = if needs_channel_convert {
            convert_channels(&input_buffer, input_spec.channels, options.channels)?
        } else {
            input_buffer
        };

        // Pad to full chunk
        let actual_frames = working_buffer.len() / working_channels;
        while working_buffer.len() < frames_needed * working_channels {
            working_buffer.push(0.0);
        }

        // Deinterleave into channel buffers
        for ch in 0..working_channels {
            channel_buffers[ch].clear();
            for frame in 0..frames_needed {
                let idx = frame * working_channels + ch;
                channel_buffers[ch].push(working_buffer[idx]);
            }
        }

        // Resample
        let output_chunk = resampler
            .process(&channel_buffers, None)
            .map_err(|e| ConvertError::ResampleError(e.to_string()))?;

        // Calculate how many output frames correspond to actual input frames
        let ratio = options.sample_rate as f64 / input_spec.sample_rate as f64;
        let output_frames_to_write = ((actual_frames as f64) * ratio).ceil() as usize;
        let available_frames = output_chunk.get(0).map(|c| c.len()).unwrap_or(0);
        let frames_to_write = output_frames_to_write.min(available_frames);

        // Interleave and write
        let mut interleaved = Vec::with_capacity(frames_to_write * working_channels);
        for frame in 0..frames_to_write {
            for ch in &output_chunk {
                if frame < ch.len() {
                    interleaved.push(ch[frame]);
                }
            }
        }

        write_chunk_to_wav(writer, &interleaved, options.bits_per_sample, &mut next_dither)?;
    }

    Ok(())
}

/// Create a sample iterator that converts to f64
fn create_sample_iterator(
    reader: &mut WavReader<std::io::BufReader<std::fs::File>>,
    spec: WavSpec,
) -> Result<Box<dyn Iterator<Item = f64> + '_>, ConvertError> {
    match (spec.sample_format, spec.bits_per_sample) {
        (SampleFormat::Int, 8) => {
            Ok(Box::new(reader.samples::<i8>().filter_map(|s| s.ok()).map(|s| s as f64 / i8::MAX as f64)))
        }
        (SampleFormat::Int, 16) => {
            Ok(Box::new(reader.samples::<i16>().filter_map(|s| s.ok()).map(|s| s as f64 / i16::MAX as f64)))
        }
        (SampleFormat::Int, 24) => {
            Ok(Box::new(reader.samples::<i32>().filter_map(|s| s.ok()).map(|s| s as f64 / (1 << 23) as f64)))
        }
        (SampleFormat::Int, 32) => {
            Ok(Box::new(reader.samples::<i32>().filter_map(|s| s.ok()).map(|s| s as f64 / i32::MAX as f64)))
        }
        (SampleFormat::Float, 32) => {
            Ok(Box::new(reader.samples::<f32>().filter_map(|s| s.ok()).map(|s| s as f64)))
        }
        _ => Err(ConvertError::UnsupportedFormat(format!(
            "{}-bit {:?}",
            spec.bits_per_sample, spec.sample_format
        ))),
    }
}

/// Write a chunk of f64 samples to WAV with proper bit depth conversion
fn write_chunk_to_wav<W: std::io::Write + std::io::Seek, F: FnMut() -> f64>(
    writer: &mut WavWriter<W>,
    samples: &[f64],
    bits_per_sample: u16,
    next_dither: &mut F,
) -> Result<(), ConvertError> {
    match bits_per_sample {
        16 => {
            let scale = i16::MAX as f64;
            for &sample in samples {
                let dithered = sample + next_dither() / scale;
                let clamped = dithered.clamp(-1.0, 1.0);
                let quantized = (clamped * scale).round() as i16;
                writer.write_sample(quantized).map_err(|e| {
                    ConvertError::WriteError(std::path::PathBuf::new(), e.to_string())
                })?;
            }
        }
        24 => {
            let scale = (1 << 23) as f64;
            for &sample in samples {
                let clamped = sample.clamp(-1.0, 1.0);
                let quantized = (clamped * scale).round() as i32;
                writer.write_sample(quantized).map_err(|e| {
                    ConvertError::WriteError(std::path::PathBuf::new(), e.to_string())
                })?;
            }
        }
        _ => {
            return Err(ConvertError::UnsupportedFormat(format!(
                "Cannot write {}-bit audio",
                bits_per_sample
            )));
        }
    }
    Ok(())
}

/// Convert between channel counts
fn convert_channels(
    samples: &[f64],
    from_channels: u16,
    to_channels: u16,
) -> Result<Vec<f64>, ConvertError> {
    match (from_channels, to_channels) {
        (1, 2) => {
            // Mono to stereo: duplicate each sample
            let mut stereo = Vec::with_capacity(samples.len() * 2);
            for &sample in samples {
                stereo.push(sample);
                stereo.push(sample);
            }
            Ok(stereo)
        }
        (2, 1) => {
            // Stereo to mono: average left and right
            let mut mono = Vec::with_capacity(samples.len() / 2);
            for chunk in samples.chunks(2) {
                if chunk.len() == 2 {
                    mono.push((chunk[0] + chunk[1]) / 2.0);
                }
            }
            Ok(mono)
        }
        (from, to) if from == to => Ok(samples.to_vec()),
        (from, to) => Err(ConvertError::UnsupportedChannelConversion(from, to)),
    }
}

/// Result of a WAV conversion
#[derive(Debug, Clone)]
pub struct ConversionResult {
    pub input_sample_rate: u32,
    pub output_sample_rate: u32,
    pub input_bits: u16,
    pub output_bits: u16,
    pub input_channels: u16,
    pub output_channels: u16,
    pub resampled: bool,
    pub bit_converted: bool,
    pub channel_converted: bool,
}

impl ConversionResult {
    /// Get a human-readable summary of the conversion
    pub fn summary(&self) -> String {
        let mut changes = Vec::new();

        if self.resampled {
            changes.push(format!(
                "{}Hz -> {}Hz",
                self.input_sample_rate, self.output_sample_rate
            ));
        }

        if self.bit_converted {
            changes.push(format!(
                "{}-bit -> {}-bit",
                self.input_bits, self.output_bits
            ));
        }

        if self.channel_converted {
            let from = if self.input_channels == 1 {
                "mono"
            } else {
                "stereo"
            };
            let to = if self.output_channels == 1 {
                "mono"
            } else {
                "stereo"
            };
            changes.push(format!("{} -> {}", from, to));
        }

        if changes.is_empty() {
            "No conversion needed".to_string()
        } else {
            changes.join(", ")
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConvertError {
    #[error("Failed to read '{0}': {1}")]
    ReadError(std::path::PathBuf, String),

    #[error("Failed to write '{0}': {1}")]
    WriteError(std::path::PathBuf, String),

    #[error("Unsupported format: {0}")]
    UnsupportedFormat(String),

    #[error("Cannot convert from {0} to {1} channels")]
    UnsupportedChannelConversion(u16, u16),

    #[error("Resampling error: {0}")]
    ResampleError(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_options_default() {
        let opts = ConvertOptions::default();
        assert_eq!(opts.sample_rate, 44100);
        assert_eq!(opts.bits_per_sample, 16);
        assert_eq!(opts.channels, 2);
    }

    #[test]
    fn test_mono_to_stereo() {
        let mono = vec![0.5, -0.5, 0.25];
        let stereo = convert_channels(&mono, 1, 2).unwrap();
        assert_eq!(stereo, vec![0.5, 0.5, -0.5, -0.5, 0.25, 0.25]);
    }

    #[test]
    fn test_stereo_to_mono() {
        let stereo = vec![0.5, 0.3, -0.5, -0.3, 0.2, 0.4];
        let mono = convert_channels(&stereo, 2, 1).unwrap();
        assert_eq!(mono.len(), 3);
        assert!((mono[0] - 0.4).abs() < 0.001); // (0.5 + 0.3) / 2
        assert!((mono[1] - -0.4).abs() < 0.001); // (-0.5 + -0.3) / 2
        assert!((mono[2] - 0.3).abs() < 0.001); // (0.2 + 0.4) / 2
    }

    #[test]
    fn test_conversion_result_summary() {
        let result = ConversionResult {
            input_sample_rate: 48000,
            output_sample_rate: 44100,
            input_bits: 24,
            output_bits: 16,
            input_channels: 1,
            output_channels: 2,
            resampled: true,
            bit_converted: true,
            channel_converted: true,
        };

        let summary = result.summary();
        assert!(summary.contains("48000Hz -> 44100Hz"));
        assert!(summary.contains("24-bit -> 16-bit"));
        assert!(summary.contains("mono -> stereo"));
    }
}

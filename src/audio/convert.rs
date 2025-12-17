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

/// Convert a WAV file with custom options
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

    // Read all samples as f64 for processing
    let samples = read_samples_as_f64(&mut reader, input_spec)?;

    // Apply conversions in order: channels -> resample -> bit depth
    let samples = if needs_channel_convert {
        convert_channels(&samples, input_spec.channels, options.channels)?
    } else {
        samples
    };

    let current_channels = if needs_channel_convert {
        options.channels
    } else {
        input_spec.channels
    };

    let samples = if needs_resample {
        resample(
            &samples,
            current_channels as usize,
            input_spec.sample_rate,
            options.sample_rate,
        )?
    } else {
        samples
    };

    // Write output file
    let output_spec = WavSpec {
        channels: options.channels,
        sample_rate: options.sample_rate,
        bits_per_sample: options.bits_per_sample,
        sample_format: SampleFormat::Int,
    };

    write_samples(output, output_spec, &samples, options.dither)?;

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

/// Read all samples from a WAV file as f64 (normalized to -1.0..1.0)
fn read_samples_as_f64(
    reader: &mut WavReader<std::io::BufReader<std::fs::File>>,
    spec: WavSpec,
) -> Result<Vec<f64>, ConvertError> {
    let num_samples = reader.len() as usize;
    let mut samples = Vec::with_capacity(num_samples);

    match (spec.sample_format, spec.bits_per_sample) {
        (SampleFormat::Int, 8) => {
            for sample in reader.samples::<i8>() {
                let s = sample.map_err(|e| ConvertError::ReadError(
                    std::path::PathBuf::new(),
                    e.to_string(),
                ))?;
                samples.push(s as f64 / i8::MAX as f64);
            }
        }
        (SampleFormat::Int, 16) => {
            for sample in reader.samples::<i16>() {
                let s = sample.map_err(|e| ConvertError::ReadError(
                    std::path::PathBuf::new(),
                    e.to_string(),
                ))?;
                samples.push(s as f64 / i16::MAX as f64);
            }
        }
        (SampleFormat::Int, 24) => {
            for sample in reader.samples::<i32>() {
                let s = sample.map_err(|e| ConvertError::ReadError(
                    std::path::PathBuf::new(),
                    e.to_string(),
                ))?;
                // 24-bit samples are stored in i32, shift to normalize
                samples.push(s as f64 / (1 << 23) as f64);
            }
        }
        (SampleFormat::Int, 32) => {
            for sample in reader.samples::<i32>() {
                let s = sample.map_err(|e| ConvertError::ReadError(
                    std::path::PathBuf::new(),
                    e.to_string(),
                ))?;
                samples.push(s as f64 / i32::MAX as f64);
            }
        }
        (SampleFormat::Float, 32) => {
            for sample in reader.samples::<f32>() {
                let s = sample.map_err(|e| ConvertError::ReadError(
                    std::path::PathBuf::new(),
                    e.to_string(),
                ))?;
                samples.push(s as f64);
            }
        }
        _ => {
            return Err(ConvertError::UnsupportedFormat(format!(
                "{}-bit {:?}",
                spec.bits_per_sample, spec.sample_format
            )));
        }
    }

    Ok(samples)
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

/// Resample audio using rubato
fn resample(
    samples: &[f64],
    channels: usize,
    from_rate: u32,
    to_rate: u32,
) -> Result<Vec<f64>, ConvertError> {
    if from_rate == to_rate {
        return Ok(samples.to_vec());
    }

    // Calculate resampling parameters
    let chunk_size = 1024;

    // Create resampler
    let mut resampler = FftFixedInOut::<f64>::new(
        from_rate as usize,
        to_rate as usize,
        chunk_size,
        channels,
    )
    .map_err(|e| ConvertError::ResampleError(e.to_string()))?;

    // Deinterleave samples into separate channels
    let num_frames = samples.len() / channels;
    let channel_data: Vec<Vec<f64>> = (0..channels)
        .map(|ch| {
            samples
                .iter()
                .skip(ch)
                .step_by(channels)
                .copied()
                .collect()
        })
        .collect();

    // Process in chunks
    let frames_needed = resampler.input_frames_next();
    let mut output_channels: Vec<Vec<f64>> = vec![Vec::new(); channels];

    let mut pos = 0;
    while pos < num_frames {
        // Prepare input chunk
        let chunk_frames = (num_frames - pos).min(frames_needed);

        // Pad if necessary
        let input_chunk: Vec<Vec<f64>> = channel_data
            .iter()
            .map(|ch| {
                let mut chunk: Vec<f64> = ch[pos..pos + chunk_frames].to_vec();
                // Pad with zeros if needed
                while chunk.len() < frames_needed {
                    chunk.push(0.0);
                }
                chunk
            })
            .collect();

        // Resample
        let output_chunk = resampler
            .process(&input_chunk, None)
            .map_err(|e| ConvertError::ResampleError(e.to_string()))?;

        // Collect output
        for (ch, data) in output_chunk.into_iter().enumerate() {
            output_channels[ch].extend(data);
        }

        pos += chunk_frames;
    }

    // Interleave output channels
    let output_frames = output_channels[0].len();
    let mut output = Vec::with_capacity(output_frames * channels);

    for frame in 0..output_frames {
        for ch in &output_channels {
            if frame < ch.len() {
                output.push(ch[frame]);
            }
        }
    }

    Ok(output)
}

/// Write samples to a WAV file
fn write_samples(
    output: &Path,
    spec: WavSpec,
    samples: &[f64],
    dither: bool,
) -> Result<(), ConvertError> {
    let mut writer = WavWriter::create(output, spec)
        .map_err(|e| ConvertError::WriteError(output.to_path_buf(), e.to_string()))?;

    // Simple TPDF dither generator
    let mut dither_state: u32 = 12345;
    let mut next_dither = || -> f64 {
        if !dither {
            return 0.0;
        }
        // Simple LCG for random numbers
        dither_state = dither_state.wrapping_mul(1103515245).wrapping_add(12345);
        let r1 = (dither_state >> 16) as f64 / 32768.0 - 1.0;
        dither_state = dither_state.wrapping_mul(1103515245).wrapping_add(12345);
        let r2 = (dither_state >> 16) as f64 / 32768.0 - 1.0;
        (r1 + r2) * 0.5 // TPDF dither
    };

    match spec.bits_per_sample {
        16 => {
            let scale = i16::MAX as f64;
            for &sample in samples {
                // Apply dither before quantization
                let dithered = sample + next_dither() / scale;
                let clamped = dithered.clamp(-1.0, 1.0);
                let quantized = (clamped * scale).round() as i16;
                writer
                    .write_sample(quantized)
                    .map_err(|e| ConvertError::WriteError(output.to_path_buf(), e.to_string()))?;
            }
        }
        24 => {
            let scale = (1 << 23) as f64;
            for &sample in samples {
                let clamped = sample.clamp(-1.0, 1.0);
                let quantized = (clamped * scale).round() as i32;
                writer
                    .write_sample(quantized)
                    .map_err(|e| ConvertError::WriteError(output.to_path_buf(), e.to_string()))?;
            }
        }
        _ => {
            return Err(ConvertError::UnsupportedFormat(format!(
                "Cannot write {}-bit audio",
                spec.bits_per_sample
            )));
        }
    }

    writer
        .finalize()
        .map_err(|e| ConvertError::WriteError(output.to_path_buf(), e.to_string()))?;

    Ok(())
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

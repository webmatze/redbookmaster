// Audio concatenation functionality
// Combines multiple tracks into a single WAV file with gaps

use hound::{WavReader, WavWriter};
use std::path::Path;
use std::time::Duration;

use super::wav::{red_book_spec, WavError, RED_BOOK_SAMPLE_RATE};
use crate::core::Track;

/// Concatenate tracks into a single WAV file
pub fn concatenate_tracks(tracks: &[Track], output: &Path) -> Result<(), ConcatError> {
    if tracks.is_empty() {
        return Err(ConcatError::NoTracks);
    }

    let spec = red_book_spec();
    let mut writer = WavWriter::create(output, spec)
        .map_err(|e| ConcatError::WriteError(output.to_path_buf(), e.to_string()))?;

    for track in tracks {
        // Write pregap (silence)
        write_silence(&mut writer, track.pregap)?;

        // Write track audio
        write_track_audio(&mut writer, &track.source_file)?;

        // Write postgap (silence)
        write_silence(&mut writer, track.postgap)?;
    }

    writer
        .finalize()
        .map_err(|e| ConcatError::WriteError(output.to_path_buf(), e.to_string()))?;

    Ok(())
}

/// Write silence (zeroes) for the given duration
fn write_silence(writer: &mut WavWriter<std::io::BufWriter<std::fs::File>>, duration: Duration) -> Result<(), ConcatError> {
    let num_samples = (duration.as_secs_f64() * RED_BOOK_SAMPLE_RATE as f64) as u32;

    // Write stereo silence (2 channels)
    for _ in 0..num_samples {
        writer.write_sample(0i16).map_err(|e| ConcatError::SampleError(e.to_string()))?;
        writer.write_sample(0i16).map_err(|e| ConcatError::SampleError(e.to_string()))?;
    }

    Ok(())
}

/// Write audio from a track file
fn write_track_audio(writer: &mut WavWriter<std::io::BufWriter<std::fs::File>>, path: &Path) -> Result<(), ConcatError> {
    let mut reader = WavReader::open(path)
        .map_err(|e| ConcatError::ReadError(path.to_path_buf(), e.to_string()))?;

    let spec = reader.spec();

    // Verify format matches
    if spec.sample_rate != RED_BOOK_SAMPLE_RATE {
        return Err(ConcatError::FormatMismatch(format!(
            "Sample rate {} doesn't match Red Book {}",
            spec.sample_rate, RED_BOOK_SAMPLE_RATE
        )));
    }

    // Read and write samples
    match spec.bits_per_sample {
        16 => {
            for sample in reader.samples::<i16>() {
                let sample = sample.map_err(|e| ConcatError::ReadError(path.to_path_buf(), e.to_string()))?;
                writer.write_sample(sample).map_err(|e| ConcatError::SampleError(e.to_string()))?;
            }
        }
        24 => {
            // Convert 24-bit to 16-bit
            for sample in reader.samples::<i32>() {
                let sample = sample.map_err(|e| ConcatError::ReadError(path.to_path_buf(), e.to_string()))?;
                let sample_16 = (sample >> 8) as i16;
                writer.write_sample(sample_16).map_err(|e| ConcatError::SampleError(e.to_string()))?;
            }
        }
        _ => {
            return Err(ConcatError::FormatMismatch(format!(
                "Unsupported bit depth: {}",
                spec.bits_per_sample
            )));
        }
    }

    Ok(())
}

/// Calculate the total size of concatenated audio
pub fn calculate_output_size(tracks: &[Track]) -> u64 {
    let total_samples: u64 = tracks
        .iter()
        .map(|t| {
            let pregap_samples = (t.pregap.as_secs_f64() * RED_BOOK_SAMPLE_RATE as f64) as u64;
            let audio_samples = (t.duration.as_secs_f64() * RED_BOOK_SAMPLE_RATE as f64) as u64;
            let postgap_samples = (t.postgap.as_secs_f64() * RED_BOOK_SAMPLE_RATE as f64) as u64;
            (pregap_samples + audio_samples + postgap_samples) * 2 // stereo
        })
        .sum();

    // 2 bytes per sample (16-bit) * total samples + WAV header (44 bytes)
    total_samples * 2 + 44
}

#[derive(Debug, thiserror::Error)]
pub enum ConcatError {
    #[error("No tracks to concatenate")]
    NoTracks,

    #[error("Failed to read '{0}': {1}")]
    ReadError(std::path::PathBuf, String),

    #[error("Failed to write '{0}': {1}")]
    WriteError(std::path::PathBuf, String),

    #[error("Format mismatch: {0}")]
    FormatMismatch(String),

    #[error("Sample error: {0}")]
    SampleError(String),

    #[error("WAV error: {0}")]
    WavError(#[from] WavError),
}

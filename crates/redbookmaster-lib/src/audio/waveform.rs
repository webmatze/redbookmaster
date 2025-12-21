//! Waveform analysis and peak extraction
//!
//! Provides functionality to extract peak amplitude data from WAV files
//! for visualization in the GUI.

use std::path::Path;
use hound::WavReader;

/// Waveform data containing min/max peaks for visualization
#[derive(Debug, Clone)]
pub struct WaveformData {
    /// Peak data as (min, max) pairs normalized to -1.0..1.0
    pub peaks: Vec<(f32, f32)>,
    /// Sample rate of the source file
    pub sample_rate: u32,
    /// Total number of samples in the source file
    pub total_samples: u64,
    /// Duration in seconds
    pub duration_secs: f64,
}

impl WaveformData {
    /// Create empty waveform data
    pub fn empty() -> Self {
        Self {
            peaks: Vec::new(),
            sample_rate: 44100,
            total_samples: 0,
            duration_secs: 0.0,
        }
    }

    /// Get a slice of peaks for a given range (0.0 to 1.0)
    ///
    /// Uses vectorizable fold pattern for better auto-SIMD optimization
    #[inline]
    pub fn get_peaks_for_range(&self, start: f32, end: f32, num_bins: usize) -> Vec<(f32, f32)> {
        if self.peaks.is_empty() || num_bins == 0 {
            return vec![(0.0, 0.0); num_bins];
        }

        let start_idx = ((start * self.peaks.len() as f32) as usize).min(self.peaks.len() - 1);
        let end_idx = ((end * self.peaks.len() as f32) as usize).min(self.peaks.len());

        if start_idx >= end_idx {
            return vec![(0.0, 0.0); num_bins];
        }

        let range_peaks = &self.peaks[start_idx..end_idx];
        let samples_per_bin = range_peaks.len() as f32 / num_bins as f32;

        (0..num_bins)
            .map(|i| {
                let bin_start = (i as f32 * samples_per_bin) as usize;
                let bin_end = (((i + 1) as f32 * samples_per_bin) as usize).min(range_peaks.len());

                if bin_start >= bin_end {
                    return (0.0, 0.0);
                }

                // Use fold pattern which LLVM can auto-vectorize
                range_peaks[bin_start..bin_end]
                    .iter()
                    .fold((0.0f32, 0.0f32), |(min_acc, max_acc), &(min, max)| {
                        (min_acc.min(min), max_acc.max(max))
                    })
            })
            .collect()
    }
}

/// Extract waveform peaks from a WAV file using streaming (memory-efficient)
///
/// # Arguments
/// * `path` - Path to the WAV file
/// * `target_peaks` - Target number of peaks to extract (typically width of display)
///
/// # Returns
/// Waveform data with normalized peaks
pub fn extract_peaks(path: &Path, target_peaks: usize) -> Result<WaveformData, WaveformError> {
    let reader = WavReader::open(path)
        .map_err(|e| WaveformError::ReadError(path.to_path_buf(), e.to_string()))?;

    let spec = reader.spec();
    let sample_rate = spec.sample_rate;
    let channels = spec.channels as usize;
    let bits_per_sample = spec.bits_per_sample;
    let total_samples = reader.duration() as u64;
    let duration_secs = total_samples as f64 / sample_rate as f64;

    // Calculate frames per peak (a frame contains all channels)
    let total_frames = total_samples as usize;
    let frames_per_peak = (total_frames / target_peaks).max(1);

    let mut peaks = Vec::with_capacity(target_peaks);

    // Process based on sample format using streaming
    match spec.sample_format {
        hound::SampleFormat::Int => {
            let max_value = (1i32 << (bits_per_sample - 1)) as f32;
            extract_peaks_streaming_int(reader, channels, frames_per_peak, max_value, &mut peaks);
        }
        hound::SampleFormat::Float => {
            extract_peaks_streaming_float(reader, channels, frames_per_peak, &mut peaks);
        }
    }

    Ok(WaveformData {
        peaks,
        sample_rate,
        total_samples,
        duration_secs,
    })
}

/// Streaming peak extraction for integer samples (memory-efficient)
///
/// Note: This function is I/O bound (reading from disk). The min/max operations
/// use f32::min/max which LLVM can auto-vectorize when processing cached data.
/// True SIMD would require buffering samples which defeats memory-efficiency.
#[inline]
fn extract_peaks_streaming_int<R: std::io::Read>(
    reader: WavReader<R>,
    channels: usize,
    frames_per_peak: usize,
    max_value: f32,
    peaks: &mut Vec<(f32, f32)>,
) {
    let mut min_val = 0.0f32;
    let mut max_val = 0.0f32;
    let mut frame_count = 0;
    let mut channel_idx = 0;
    let mut sample_sum: i64 = 0;
    let inv_max = 1.0 / max_value; // Multiply instead of divide in hot path
    let inv_channels = 1.0 / channels as f64; // Pre-compute for mono mixing

    for sample_result in reader.into_samples::<i32>() {
        let sample = match sample_result {
            Ok(s) => s,
            Err(_) => continue,
        };

        sample_sum += sample as i64;
        channel_idx += 1;

        // Complete frame (all channels read)
        if channel_idx == channels {
            let mono_sample = (sample_sum as f64 * inv_channels) as f32 * inv_max;
            min_val = min_val.min(mono_sample);
            max_val = max_val.max(mono_sample);

            frame_count += 1;
            channel_idx = 0;
            sample_sum = 0;

            // Complete peak bin
            if frame_count >= frames_per_peak {
                peaks.push((min_val, max_val));
                min_val = 0.0;
                max_val = 0.0;
                frame_count = 0;
            }
        }
    }

    // Push remaining frames as final peak
    if frame_count > 0 {
        peaks.push((min_val, max_val));
    }
}

/// Streaming peak extraction for float samples (memory-efficient)
///
/// Note: This function is I/O bound (reading from disk). The min/max operations
/// use f32::min/max which LLVM can auto-vectorize when processing cached data.
#[inline]
fn extract_peaks_streaming_float<R: std::io::Read>(
    reader: WavReader<R>,
    channels: usize,
    frames_per_peak: usize,
    peaks: &mut Vec<(f32, f32)>,
) {
    let mut min_val = 0.0f32;
    let mut max_val = 0.0f32;
    let mut frame_count = 0;
    let mut channel_idx = 0;
    let mut sample_sum: f32 = 0.0;
    let inv_channels = 1.0 / channels as f32; // Pre-compute for mono mixing

    for sample_result in reader.into_samples::<f32>() {
        let sample = match sample_result {
            Ok(s) => s,
            Err(_) => continue,
        };

        sample_sum += sample;
        channel_idx += 1;

        // Complete frame (all channels read)
        if channel_idx == channels {
            let mono_sample = sample_sum * inv_channels;
            min_val = min_val.min(mono_sample);
            max_val = max_val.max(mono_sample);

            frame_count += 1;
            channel_idx = 0;
            sample_sum = 0.0;

            // Complete peak bin
            if frame_count >= frames_per_peak {
                peaks.push((min_val, max_val));
                min_val = 0.0;
                max_val = 0.0;
                frame_count = 0;
            }
        }
    }

    // Push remaining frames as final peak
    if frame_count > 0 {
        peaks.push((min_val, max_val));
    }
}

/// Generate SVG path data for waveform rendering
///
/// # Arguments
/// * `peaks` - Peak data as (min, max) pairs
/// * `width` - Width of the display area
/// * `height` - Height of the display area
/// * `fill` - If true, generates a filled path; otherwise a line path
///
/// # Returns
/// SVG path data string
pub fn peaks_to_svg_path(peaks: &[(f32, f32)], width: f32, height: f32, fill: bool) -> String {
    if peaks.is_empty() {
        return String::new();
    }

    let center_y = height / 2.0;
    let bar_width = width / peaks.len() as f32;

    let mut path = String::with_capacity(peaks.len() * 20);

    if fill {
        // Create a filled polygon path
        // Start at bottom-left, go along top edge, then back along bottom edge

        // Top edge (max values)
        path.push_str(&format!("M 0 {:.1} ", center_y - peaks[0].1 * center_y));
        for (i, &(_min, max)) in peaks.iter().enumerate() {
            let x = (i as f32 + 0.5) * bar_width;
            let y = center_y - max * center_y;
            path.push_str(&format!("L {:.1} {:.1} ", x, y));
        }

        // Bottom edge (min values, reversed)
        for (i, &(min, _max)) in peaks.iter().enumerate().rev() {
            let x = (i as f32 + 0.5) * bar_width;
            let y = center_y - min * center_y;
            path.push_str(&format!("L {:.1} {:.1} ", x, y));
        }

        path.push('Z');
    } else {
        // Create a simple line path through peak maxes
        path.push_str(&format!("M 0 {:.1} ", center_y - peaks[0].1 * center_y));
        for (i, &(_min, max)) in peaks.iter().enumerate() {
            let x = (i as f32 + 0.5) * bar_width;
            let y = center_y - max * center_y;
            path.push_str(&format!("L {:.1} {:.1} ", x, y));
        }
    }

    path
}

#[derive(Debug, thiserror::Error)]
pub enum WaveformError {
    #[error("Failed to read WAV file '{0}': {1}")]
    ReadError(std::path::PathBuf, String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_waveform() {
        let wf = WaveformData::empty();
        assert!(wf.peaks.is_empty());
        assert_eq!(wf.duration_secs, 0.0);
    }

    #[test]
    fn test_peaks_to_svg_path() {
        let peaks = vec![(-0.5, 0.5), (-0.3, 0.3), (-0.8, 0.8)];
        let path = peaks_to_svg_path(&peaks, 300.0, 100.0, false);
        assert!(!path.is_empty());
        assert!(path.starts_with("M"));
    }

    #[test]
    fn test_get_peaks_for_range() {
        let mut wf = WaveformData::empty();
        wf.peaks = vec![(-0.1, 0.1), (-0.2, 0.2), (-0.3, 0.3), (-0.4, 0.4)];

        let subset = wf.get_peaks_for_range(0.0, 0.5, 2);
        assert_eq!(subset.len(), 2);
    }
}

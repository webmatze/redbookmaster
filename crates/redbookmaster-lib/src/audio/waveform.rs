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

        let mut result = Vec::with_capacity(num_bins);

        for i in 0..num_bins {
            let bin_start = (i as f32 * samples_per_bin) as usize;
            let bin_end = ((i + 1) as f32 * samples_per_bin) as usize;
            let bin_end = bin_end.min(range_peaks.len());

            if bin_start >= bin_end {
                result.push((0.0, 0.0));
                continue;
            }

            let mut min_val = 0.0f32;
            let mut max_val = 0.0f32;

            for &(min, max) in &range_peaks[bin_start..bin_end] {
                min_val = min_val.min(min);
                max_val = max_val.max(max);
            }

            result.push((min_val, max_val));
        }

        result
    }
}

/// Extract waveform peaks from a WAV file
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

    // Calculate samples per peak
    let samples_per_peak = (total_samples as usize / target_peaks).max(1);

    let mut peaks = Vec::with_capacity(target_peaks);

    // Process based on sample format
    match spec.sample_format {
        hound::SampleFormat::Int => {
            let max_value = (1i32 << (bits_per_sample - 1)) as f32;
            let samples: Vec<i32> = reader.into_samples::<i32>()
                .filter_map(|s| s.ok())
                .collect();

            extract_peaks_from_samples(&samples, channels, samples_per_peak, max_value, &mut peaks);
        }
        hound::SampleFormat::Float => {
            let samples: Vec<f32> = reader.into_samples::<f32>()
                .filter_map(|s| s.ok())
                .collect();

            extract_peaks_from_float_samples(&samples, channels, samples_per_peak, &mut peaks);
        }
    }

    Ok(WaveformData {
        peaks,
        sample_rate,
        total_samples,
        duration_secs,
    })
}

fn extract_peaks_from_samples(
    samples: &[i32],
    channels: usize,
    samples_per_peak: usize,
    max_value: f32,
    peaks: &mut Vec<(f32, f32)>,
) {
    let frames = samples.len() / channels;
    let frames_per_peak = samples_per_peak;

    let mut frame_idx = 0;
    while frame_idx < frames {
        let end_frame = (frame_idx + frames_per_peak).min(frames);

        let mut min_val = 0.0f32;
        let mut max_val = 0.0f32;

        for f in frame_idx..end_frame {
            // Mix all channels to mono for display
            let mut sample_sum = 0i64;
            for ch in 0..channels {
                sample_sum += samples[f * channels + ch] as i64;
            }
            let sample = (sample_sum / channels as i64) as f32 / max_value;

            min_val = min_val.min(sample);
            max_val = max_val.max(sample);
        }

        peaks.push((min_val, max_val));
        frame_idx = end_frame;
    }
}

fn extract_peaks_from_float_samples(
    samples: &[f32],
    channels: usize,
    samples_per_peak: usize,
    peaks: &mut Vec<(f32, f32)>,
) {
    let frames = samples.len() / channels;
    let frames_per_peak = samples_per_peak;

    let mut frame_idx = 0;
    while frame_idx < frames {
        let end_frame = (frame_idx + frames_per_peak).min(frames);

        let mut min_val = 0.0f32;
        let mut max_val = 0.0f32;

        for f in frame_idx..end_frame {
            // Mix all channels to mono for display
            let mut sample_sum = 0.0f32;
            for ch in 0..channels {
                sample_sum += samples[f * channels + ch];
            }
            let sample = sample_sum / channels as f32;

            min_val = min_val.min(sample);
            max_val = max_val.max(sample);
        }

        peaks.push((min_val, max_val));
        frame_idx = end_frame;
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

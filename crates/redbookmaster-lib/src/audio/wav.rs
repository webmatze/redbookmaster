use hound::{WavReader, WavSpec};
use std::path::Path;
use std::time::Duration;

/// Red Book CD audio specifications
pub const RED_BOOK_SAMPLE_RATE: u32 = 44100;
pub const RED_BOOK_BITS_PER_SAMPLE: u16 = 16;
pub const RED_BOOK_CHANNELS: u16 = 2;

/// Information about a WAV file
#[derive(Debug, Clone)]
pub struct WavInfo {
    /// Sample rate in Hz
    pub sample_rate: u32,
    /// Bits per sample
    pub bits_per_sample: u16,
    /// Number of channels
    pub channels: u16,
    /// Duration of the audio
    pub duration: Duration,
    /// Number of samples (per channel)
    pub num_samples: u64,
    /// File path
    pub path: std::path::PathBuf,
}

impl WavInfo {
    /// Check if the WAV file is Red Book compliant
    pub fn is_red_book_compliant(&self) -> bool {
        self.sample_rate == RED_BOOK_SAMPLE_RATE
            && self.bits_per_sample == RED_BOOK_BITS_PER_SAMPLE
            && self.channels == RED_BOOK_CHANNELS
    }

    /// Get a list of format issues (empty if compliant)
    pub fn format_issues(&self) -> Vec<String> {
        let mut issues = Vec::new();

        if self.sample_rate != RED_BOOK_SAMPLE_RATE {
            issues.push(format!(
                "Sample rate is {}Hz (needs {}Hz)",
                self.sample_rate, RED_BOOK_SAMPLE_RATE
            ));
        }

        if self.bits_per_sample != RED_BOOK_BITS_PER_SAMPLE {
            issues.push(format!(
                "Bit depth is {} (needs {})",
                self.bits_per_sample, RED_BOOK_BITS_PER_SAMPLE
            ));
        }

        if self.channels != RED_BOOK_CHANNELS {
            issues.push(format!(
                "Channels: {} (needs {} stereo)",
                self.channels, RED_BOOK_CHANNELS
            ));
        }

        issues
    }

    /// Get a human-readable format description
    pub fn format_description(&self) -> String {
        format!(
            "{}Hz, {}-bit, {} channel{}",
            self.sample_rate,
            self.bits_per_sample,
            self.channels,
            if self.channels == 1 { "" } else { "s" }
        )
    }
}

/// Read information from a WAV file
pub fn read_wav_info(path: &Path) -> Result<WavInfo, WavError> {
    let reader = WavReader::open(path)
        .map_err(|e| WavError::OpenError(path.to_path_buf(), e.to_string()))?;

    let spec = reader.spec();
    let num_samples = reader.len() as u64 / spec.channels as u64;

    // Calculate duration
    let duration_secs = num_samples as f64 / spec.sample_rate as f64;
    let duration = Duration::from_secs_f64(duration_secs);

    Ok(WavInfo {
        sample_rate: spec.sample_rate,
        bits_per_sample: spec.bits_per_sample,
        channels: spec.channels,
        duration,
        num_samples,
        path: path.to_path_buf(),
    })
}

/// Validate a WAV file for Red Book compliance
pub fn validate_wav(path: &Path) -> Result<WavInfo, WavError> {
    let info = read_wav_info(path)?;

    if !info.is_red_book_compliant() {
        return Err(WavError::NotCompliant {
            path: path.to_path_buf(),
            issues: info.format_issues(),
        });
    }

    Ok(info)
}

/// Get WAV spec for writing Red Book compliant files
pub fn red_book_spec() -> WavSpec {
    WavSpec {
        channels: RED_BOOK_CHANNELS,
        sample_rate: RED_BOOK_SAMPLE_RATE,
        bits_per_sample: RED_BOOK_BITS_PER_SAMPLE,
        sample_format: hound::SampleFormat::Int,
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WavError {
    #[error("Failed to open WAV file '{0}': {1}")]
    OpenError(std::path::PathBuf, String),

    #[error("Failed to read WAV file '{0}': {1}")]
    ReadError(std::path::PathBuf, String),

    #[error("Failed to write WAV file '{0}': {1}")]
    WriteError(std::path::PathBuf, String),

    #[error("WAV file '{path}' is not Red Book compliant: {}", issues.join(", "))]
    NotCompliant {
        path: std::path::PathBuf,
        issues: Vec<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_red_book_spec() {
        let spec = red_book_spec();
        assert_eq!(spec.sample_rate, 44100);
        assert_eq!(spec.bits_per_sample, 16);
        assert_eq!(spec.channels, 2);
    }

    #[test]
    fn test_wav_info_compliance() {
        let info = WavInfo {
            sample_rate: 44100,
            bits_per_sample: 16,
            channels: 2,
            duration: Duration::from_secs(60),
            num_samples: 44100 * 60,
            path: std::path::PathBuf::from("test.wav"),
        };

        assert!(info.is_red_book_compliant());
        assert!(info.format_issues().is_empty());
    }

    #[test]
    fn test_wav_info_non_compliant() {
        let info = WavInfo {
            sample_rate: 48000,
            bits_per_sample: 24,
            channels: 1,
            duration: Duration::from_secs(60),
            num_samples: 48000 * 60,
            path: std::path::PathBuf::from("test.wav"),
        };

        assert!(!info.is_red_book_compliant());
        assert_eq!(info.format_issues().len(), 3);
    }
}

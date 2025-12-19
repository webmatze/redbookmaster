use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::metadata::Isrc;

/// Red Book CD frame rate: 75 frames per second
pub const FRAMES_PER_SECOND: u32 = 75;

/// Default pregap for track 1 (2 seconds as per Red Book spec)
pub const DEFAULT_TRACK1_PREGAP: Duration = Duration::from_secs(2);

/// Minimum track duration (4 seconds as per Red Book spec)
pub const MIN_TRACK_DURATION: Duration = Duration::from_secs(4);

/// Represents a single track on the CD
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    /// Track number (1-99)
    pub number: u8,

    /// Track title
    pub title: String,

    /// Track performer (overrides album performer if set)
    pub performer: Option<String>,

    /// Track songwriter (overrides album songwriter if set)
    pub songwriter: Option<String>,

    /// ISRC code for this track
    pub isrc: Option<Isrc>,

    /// Path to the source WAV file
    pub source_file: PathBuf,

    /// Duration of the track audio
    #[serde(with = "duration_serde")]
    pub duration: Duration,

    /// Gap before this track (silence)
    /// Default: 2 seconds for track 1, 0 for others
    #[serde(with = "duration_serde")]
    pub pregap: Duration,

    /// Gap after this track (silence)
    #[serde(with = "duration_serde")]
    pub postgap: Duration,

    /// INDEX 00 position for hidden track/pause
    #[serde(with = "option_duration_serde")]
    pub index00: Option<Duration>,
}

impl Track {
    /// Create a new track with default settings
    pub fn new(number: u8, title: String, source_file: PathBuf, duration: Duration) -> Self {
        let pregap = if number == 1 {
            DEFAULT_TRACK1_PREGAP
        } else {
            Duration::ZERO
        };

        Self {
            number,
            title,
            performer: None,
            songwriter: None,
            isrc: None,
            source_file,
            duration,
            pregap,
            postgap: Duration::ZERO,
            index00: None,
        }
    }

    /// Calculate the start time of this track in CD frames
    pub fn start_frame(&self, previous_end_frame: u32) -> u32 {
        previous_end_frame + duration_to_frames(self.pregap)
    }

    /// Calculate the end frame of this track
    pub fn end_frame(&self, start_frame: u32) -> u32 {
        start_frame + duration_to_frames(self.duration)
    }

    /// Get total frames including pregap and postgap
    pub fn total_frames(&self) -> u32 {
        duration_to_frames(self.pregap) +
        duration_to_frames(self.duration) +
        duration_to_frames(self.postgap)
    }

    /// Validate the track against Red Book specifications
    pub fn validate(&self) -> Result<(), TrackValidationError> {
        if self.number == 0 || self.number > 99 {
            return Err(TrackValidationError::InvalidTrackNumber(self.number));
        }

        if self.duration < MIN_TRACK_DURATION {
            return Err(TrackValidationError::TrackTooShort {
                track: self.number,
                duration: self.duration,
                minimum: MIN_TRACK_DURATION,
            });
        }

        if let Some(ref isrc) = self.isrc {
            isrc.validate().map_err(|e| TrackValidationError::InvalidIsrc {
                track: self.number,
                error: e,
            })?;
        }

        Ok(())
    }

    /// Format duration as MM:SS:FF (minutes:seconds:frames)
    pub fn format_duration(&self) -> String {
        format_duration_msf(self.duration)
    }

    /// Resolve the source file path relative to a project directory.
    /// If the path is already absolute, returns it as-is (for backwards compatibility).
    /// If the path is relative, joins it with the project directory.
    pub fn resolve_source_file(&self, project_dir: &Path) -> PathBuf {
        if self.source_file.is_absolute() {
            self.source_file.clone()
        } else {
            project_dir.join(&self.source_file)
        }
    }
}

/// Convert Duration to CD frames
pub fn duration_to_frames(duration: Duration) -> u32 {
    let total_millis = duration.as_millis() as u32;
    (total_millis * FRAMES_PER_SECOND) / 1000
}

/// Convert CD frames to Duration
pub fn frames_to_duration(frames: u32) -> Duration {
    let millis = (frames * 1000) / FRAMES_PER_SECOND;
    Duration::from_millis(millis as u64)
}

/// Format duration as MM:SS:FF (minutes:seconds:frames)
pub fn format_duration_msf(duration: Duration) -> String {
    let total_frames = duration_to_frames(duration);
    let frames = total_frames % FRAMES_PER_SECOND;
    let total_seconds = total_frames / FRAMES_PER_SECOND;
    let seconds = total_seconds % 60;
    let minutes = total_seconds / 60;
    format!("{:02}:{:02}:{:02}", minutes, seconds, frames)
}

/// Format duration as MM:SS for display
pub fn format_duration_ms(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let seconds = total_seconds % 60;
    let minutes = total_seconds / 60;
    format!("{}:{:02}", minutes, seconds)
}

#[derive(Debug, thiserror::Error)]
pub enum TrackValidationError {
    #[error("Invalid track number: {0} (must be 1-99)")]
    InvalidTrackNumber(u8),

    #[error("Track {track} is too short: {duration:?} (minimum: {minimum:?})")]
    TrackTooShort {
        track: u8,
        duration: Duration,
        minimum: Duration,
    },

    #[error("Track {track} has invalid ISRC: {error}")]
    InvalidIsrc {
        track: u8,
        error: String,
    },
}

// Custom serde for Duration (stored as milliseconds)
mod duration_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        duration.as_millis().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let millis = u64::deserialize(deserializer)?;
        Ok(Duration::from_millis(millis))
    }
}

mod option_duration_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Option<Duration>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match duration {
            Some(d) => Some(d.as_millis()).serialize(serializer),
            None => None::<u128>.serialize(serializer),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let millis: Option<u64> = Option::deserialize(deserializer)?;
        Ok(millis.map(Duration::from_millis))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_duration_to_frames() {
        assert_eq!(duration_to_frames(Duration::from_secs(1)), 75);
        assert_eq!(duration_to_frames(Duration::from_secs(2)), 150);
        assert_eq!(duration_to_frames(Duration::from_millis(1000)), 75);
    }

    #[test]
    fn test_format_duration_msf() {
        assert_eq!(format_duration_msf(Duration::from_secs(0)), "00:00:00");
        assert_eq!(format_duration_msf(Duration::from_secs(1)), "00:01:00");
        assert_eq!(format_duration_msf(Duration::from_secs(60)), "01:00:00");
        assert_eq!(format_duration_msf(Duration::from_secs(125)), "02:05:00");
    }
}

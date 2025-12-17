use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::metadata::{CdText, Mcn};
use super::track::{format_duration_ms, Track, TrackValidationError};

/// Maximum number of tracks on a Red Book CD
pub const MAX_TRACKS: usize = 99;

/// Maximum duration of a Red Book CD (79 minutes, 57 seconds)
pub const MAX_DURATION: Duration = Duration::from_secs(79 * 60 + 57);

/// Represents a complete album/CD project
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Album {
    /// Album title
    pub title: String,

    /// Album performer/artist
    pub performer: String,

    /// Album songwriter
    pub songwriter: Option<String>,

    /// Media Catalog Number (UPC/EAN)
    pub catalog: Option<Mcn>,

    /// CD-TEXT metadata
    #[serde(default)]
    pub cd_text: CdText,

    /// Tracks in order
    pub tracks: Vec<Track>,

    /// When the project was created
    pub created_at: DateTime<Utc>,

    /// When the project was last modified
    pub modified_at: DateTime<Utc>,
}

impl Album {
    /// Create a new album with the given title and performer
    pub fn new(title: String, performer: String) -> Self {
        let now = Utc::now();
        Self {
            title,
            performer,
            songwriter: None,
            catalog: None,
            cd_text: CdText::new(),
            tracks: Vec::new(),
            created_at: now,
            modified_at: now,
        }
    }

    /// Add a track to the album
    pub fn add_track(&mut self, mut track: Track) {
        track.number = (self.tracks.len() + 1) as u8;
        self.tracks.push(track);
        self.modified_at = Utc::now();
    }

    /// Remove a track by index (0-based)
    pub fn remove_track(&mut self, index: usize) -> Option<Track> {
        if index < self.tracks.len() {
            let track = self.tracks.remove(index);
            self.renumber_tracks();
            self.modified_at = Utc::now();
            Some(track)
        } else {
            None
        }
    }

    /// Move a track from one position to another
    pub fn move_track(&mut self, from: usize, to: usize) -> bool {
        if from >= self.tracks.len() || to >= self.tracks.len() {
            return false;
        }

        let track = self.tracks.remove(from);
        self.tracks.insert(to, track);
        self.renumber_tracks();
        self.modified_at = Utc::now();
        true
    }

    /// Get a track by number (1-based)
    pub fn get_track(&self, number: u8) -> Option<&Track> {
        self.tracks.get((number as usize).saturating_sub(1))
    }

    /// Get a mutable track by number (1-based)
    pub fn get_track_mut(&mut self, number: u8) -> Option<&mut Track> {
        self.tracks.get_mut((number as usize).saturating_sub(1))
    }

    /// Renumber all tracks sequentially
    fn renumber_tracks(&mut self) {
        for (i, track) in self.tracks.iter_mut().enumerate() {
            track.number = (i + 1) as u8;
        }
    }

    /// Calculate total duration of all tracks including gaps
    pub fn total_duration(&self) -> Duration {
        self.tracks
            .iter()
            .map(|t| t.duration + t.pregap + t.postgap)
            .sum()
    }

    /// Get number of tracks
    pub fn track_count(&self) -> usize {
        self.tracks.len()
    }

    /// Format total duration as MM:SS
    pub fn format_duration(&self) -> String {
        format_duration_ms(self.total_duration())
    }

    /// Validate the album against Red Book specifications
    pub fn validate(&self) -> Result<(), AlbumValidationError> {
        // Check number of tracks
        if self.tracks.is_empty() {
            return Err(AlbumValidationError::NoTracks);
        }

        if self.tracks.len() > MAX_TRACKS {
            return Err(AlbumValidationError::TooManyTracks {
                count: self.tracks.len(),
                max: MAX_TRACKS,
            });
        }

        // Check total duration
        let total = self.total_duration();
        if total > MAX_DURATION {
            return Err(AlbumValidationError::TooLong {
                duration: total,
                max: MAX_DURATION,
            });
        }

        // Validate catalog number if present
        if let Some(ref mcn) = self.catalog {
            mcn.validate().map_err(|e| AlbumValidationError::InvalidCatalog(e))?;
        }

        // Validate each track
        for track in &self.tracks {
            track.validate()?;
        }

        // Check track 1 has proper pregap
        if let Some(first_track) = self.tracks.first() {
            if first_track.pregap < super::track::DEFAULT_TRACK1_PREGAP {
                return Err(AlbumValidationError::Track1PregapTooShort {
                    pregap: first_track.pregap,
                    minimum: super::track::DEFAULT_TRACK1_PREGAP,
                });
            }
        }

        Ok(())
    }

    /// Display a summary of the album
    pub fn display_summary(&self) -> String {
        let mut summary = String::new();
        summary.push_str(&format!("Album: {}\n", self.title));
        summary.push_str(&format!("Artist: {}\n", self.performer));
        if let Some(ref songwriter) = self.songwriter {
            summary.push_str(&format!("Songwriter: {}\n", songwriter));
        }
        if let Some(ref catalog) = self.catalog {
            summary.push_str(&format!("Catalog: {}\n", catalog));
        }
        summary.push_str(&format!("Tracks: {}\n", self.track_count()));
        summary.push_str(&format!("Total: {}\n", self.format_duration()));
        summary
    }
}

impl Default for Album {
    fn default() -> Self {
        Self::new("Untitled Album".to_string(), "Unknown Artist".to_string())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AlbumValidationError {
    #[error("Album has no tracks")]
    NoTracks,

    #[error("Too many tracks: {count} (maximum: {max})")]
    TooManyTracks { count: usize, max: usize },

    #[error("Album too long: {duration:?} (maximum: {max:?})")]
    TooLong { duration: Duration, max: Duration },

    #[error("Invalid catalog number: {0}")]
    InvalidCatalog(String),

    #[error("Track 1 pregap too short: {pregap:?} (minimum: {minimum:?})")]
    Track1PregapTooShort {
        pregap: Duration,
        minimum: Duration,
    },

    #[error("Track validation error: {0}")]
    TrackError(#[from] TrackValidationError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_new_album() {
        let album = Album::new("Test Album".to_string(), "Test Artist".to_string());
        assert_eq!(album.title, "Test Album");
        assert_eq!(album.performer, "Test Artist");
        assert_eq!(album.tracks.len(), 0);
    }

    #[test]
    fn test_add_track() {
        let mut album = Album::new("Test".to_string(), "Artist".to_string());
        let track = Track::new(1, "Track 1".to_string(), PathBuf::from("test.wav"), Duration::from_secs(180));
        album.add_track(track);
        assert_eq!(album.tracks.len(), 1);
        assert_eq!(album.tracks[0].number, 1);
    }

    #[test]
    fn test_track_renumbering() {
        let mut album = Album::new("Test".to_string(), "Artist".to_string());

        for i in 1..=3 {
            let track = Track::new(i, format!("Track {}", i), PathBuf::from("test.wav"), Duration::from_secs(180));
            album.add_track(track);
        }

        album.remove_track(0);
        assert_eq!(album.tracks[0].number, 1);
        assert_eq!(album.tracks[1].number, 2);
    }

    #[test]
    fn test_validate_empty_album() {
        let album = Album::new("Test".to_string(), "Artist".to_string());
        assert!(matches!(album.validate(), Err(AlbumValidationError::NoTracks)));
    }
}

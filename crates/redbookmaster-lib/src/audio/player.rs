// Audio playback functionality using rodio

use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Audio player for previewing tracks
pub struct Player {
    /// The audio output stream (must be kept alive)
    _stream: OutputStream,
    /// Handle to the output stream
    stream_handle: OutputStreamHandle,
    /// The audio sink for controlling playback
    sink: Option<Sink>,
    /// Track if we're currently playing
    is_playing: Arc<AtomicBool>,
    /// Current playback position in milliseconds
    position_ms: Arc<AtomicU64>,
    /// Duration of current track in milliseconds
    duration_ms: u64,
    /// Path to the currently loaded file
    current_file: Option<std::path::PathBuf>,
}

impl Player {
    /// Create a new player instance
    pub fn new() -> Result<Self, PlayerError> {
        let (stream, stream_handle) = OutputStream::try_default()
            .map_err(|e| PlayerError::InitError(e.to_string()))?;

        Ok(Self {
            _stream: stream,
            stream_handle,
            sink: None,
            is_playing: Arc::new(AtomicBool::new(false)),
            position_ms: Arc::new(AtomicU64::new(0)),
            duration_ms: 0,
            current_file: None,
        })
    }

    /// Play a WAV file from the beginning
    pub fn play(&mut self, path: &Path) -> Result<Duration, PlayerError> {
        // Stop any current playback
        self.stop();

        // Open and decode the file
        let file = File::open(path)
            .map_err(|e| PlayerError::PlayError(format!("Failed to open file: {}", e)))?;

        let reader = BufReader::new(file);
        let source = Decoder::new(reader)
            .map_err(|e| PlayerError::PlayError(format!("Failed to decode audio: {}", e)))?;

        // Get the duration if available
        let duration = source.total_duration().unwrap_or(Duration::ZERO);
        self.duration_ms = duration.as_millis() as u64;

        // Create a new sink
        let sink = Sink::try_new(&self.stream_handle)
            .map_err(|e| PlayerError::InitError(e.to_string()))?;

        // Append the source and play
        sink.append(source);
        sink.play();

        self.sink = Some(sink);
        self.is_playing.store(true, Ordering::SeqCst);
        self.position_ms.store(0, Ordering::SeqCst);
        self.current_file = Some(path.to_path_buf());

        Ok(duration)
    }

    /// Play a WAV file, skipping to a specific position
    pub fn play_from(&mut self, path: &Path, skip: Duration) -> Result<Duration, PlayerError> {
        // Stop any current playback
        self.stop();

        // Open and decode the file
        let file = File::open(path)
            .map_err(|e| PlayerError::PlayError(format!("Failed to open file: {}", e)))?;

        let reader = BufReader::new(file);
        let source = Decoder::new(reader)
            .map_err(|e| PlayerError::PlayError(format!("Failed to decode audio: {}", e)))?;

        // Get the duration if available
        let duration = source.total_duration().unwrap_or(Duration::ZERO);
        self.duration_ms = duration.as_millis() as u64;

        // Skip to position
        let source = source.skip_duration(skip);

        // Create a new sink
        let sink = Sink::try_new(&self.stream_handle)
            .map_err(|e| PlayerError::InitError(e.to_string()))?;

        // Append the source and play
        sink.append(source);
        sink.play();

        self.sink = Some(sink);
        self.is_playing.store(true, Ordering::SeqCst);
        self.position_ms.store(skip.as_millis() as u64, Ordering::SeqCst);
        self.current_file = Some(path.to_path_buf());

        let remaining = duration.saturating_sub(skip);
        Ok(remaining)
    }

    /// Pause playback
    pub fn pause(&mut self) {
        if let Some(ref sink) = self.sink {
            sink.pause();
            self.is_playing.store(false, Ordering::SeqCst);
        }
    }

    /// Resume playback
    pub fn resume(&mut self) {
        if let Some(ref sink) = self.sink {
            sink.play();
            self.is_playing.store(true, Ordering::SeqCst);
        }
    }

    /// Toggle pause/play
    pub fn toggle_pause(&mut self) {
        if self.is_playing() {
            self.pause();
        } else {
            self.resume();
        }
    }

    /// Stop playback and clear the current track
    pub fn stop(&mut self) {
        if let Some(sink) = self.sink.take() {
            sink.stop();
        }
        self.is_playing.store(false, Ordering::SeqCst);
        self.position_ms.store(0, Ordering::SeqCst);
        self.current_file = None;
        self.duration_ms = 0;
    }

    /// Set volume (0.0 to 1.0)
    pub fn set_volume(&mut self, volume: f32) {
        if let Some(ref sink) = self.sink {
            sink.set_volume(volume.clamp(0.0, 1.0));
        }
    }

    /// Get current volume
    pub fn volume(&self) -> f32 {
        self.sink.as_ref().map(|s| s.volume()).unwrap_or(1.0)
    }

    /// Check if currently playing (not paused)
    pub fn is_playing(&self) -> bool {
        if let Some(ref sink) = self.sink {
            !sink.is_paused() && !sink.empty()
        } else {
            false
        }
    }

    /// Check if playback has finished
    pub fn is_finished(&self) -> bool {
        self.sink.as_ref().map(|s| s.empty()).unwrap_or(true)
    }

    /// Check if paused
    pub fn is_paused(&self) -> bool {
        self.sink.as_ref().map(|s| s.is_paused()).unwrap_or(false)
    }

    /// Get duration of current track
    pub fn duration(&self) -> Duration {
        Duration::from_millis(self.duration_ms)
    }

    /// Get the currently playing file path
    pub fn current_file(&self) -> Option<&Path> {
        self.current_file.as_deref()
    }

    /// Wait for playback to complete (blocking)
    pub fn wait_until_end(&self) {
        if let Some(ref sink) = self.sink {
            sink.sleep_until_end();
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PlayerError {
    #[error("Failed to initialize audio output: {0}")]
    InitError(String),

    #[error("Failed to play file: {0}")]
    PlayError(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_player_creation() {
        // This test may fail in CI environments without audio devices
        let result = Player::new();
        // We just check it doesn't panic; it may fail without audio hardware
        if result.is_ok() {
            let player = result.unwrap();
            assert!(!player.is_playing());
            assert!(player.is_finished());
        }
    }
}

//! Thread-safe audio playback engine
//!
//! Provides audio playback with play, pause, stop, seek, and volume controls.
//! Uses crossbeam channels for thread-safe communication between UI and audio threads.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crossbeam_channel::{bounded, Receiver, Sender};
use parking_lot::Mutex;
use rodio::{Decoder, OutputStream, Sink, Source};

/// Commands sent to the audio engine
#[derive(Debug)]
pub enum PlayerCommand {
    /// Load a track from the given path
    Load(PathBuf),
    /// Load a track and immediately start playing
    LoadAndPlay(PathBuf),
    /// Start or resume playback
    Play,
    /// Pause playback
    Pause,
    /// Stop playback and reset position
    Stop,
    /// Seek to position (0.0 to 1.0)
    Seek(f32),
    /// Set volume (0.0 to 1.0)
    SetVolume(f32),
    /// Shutdown the audio thread
    Shutdown,
}

/// Events sent from the audio engine
#[derive(Debug, Clone)]
pub enum PlayerEvent {
    /// Track loaded successfully with duration in milliseconds
    Loaded { duration_ms: u64 },
    /// Playback started
    Playing,
    /// Playback paused
    Paused,
    /// Playback stopped by user
    Stopped,
    /// Track finished playing naturally (reached end)
    TrackFinished,
    /// Position update (milliseconds from start)
    Position(u64),
    /// Error occurred
    Error(String),
}

/// Shared playback state accessible from main thread
pub struct PlaybackState {
    /// Current position in milliseconds
    pub position_ms: AtomicU64,
    /// Total duration in milliseconds
    pub duration_ms: AtomicU64,
    /// Is currently playing
    pub is_playing: AtomicBool,
    /// Is paused
    pub is_paused: AtomicBool,
    /// Current volume (0.0 to 1.0)
    pub volume: Mutex<f32>,
}

impl PlaybackState {
    pub fn new() -> Self {
        Self {
            position_ms: AtomicU64::new(0),
            duration_ms: AtomicU64::new(0),
            is_playing: AtomicBool::new(false),
            is_paused: AtomicBool::new(false),
            volume: Mutex::new(0.8),
        }
    }
}

impl Default for PlaybackState {
    fn default() -> Self {
        Self::new()
    }
}

/// Audio playback engine
pub struct AudioEngine {
    command_tx: Sender<PlayerCommand>,
    event_rx: Receiver<PlayerEvent>,
    #[allow(dead_code)]
    state: Arc<PlaybackState>,
    _thread_handle: thread::JoinHandle<()>,
}

impl AudioEngine {
    /// Create a new audio engine
    pub fn new() -> Result<Self, String> {
        let (command_tx, command_rx) = bounded::<PlayerCommand>(32);
        let (event_tx, event_rx) = bounded::<PlayerEvent>(32);
        let state = Arc::new(PlaybackState::new());
        let state_clone = state.clone();

        let thread_handle = thread::spawn(move || {
            audio_thread(command_rx, event_tx, state_clone);
        });

        Ok(Self {
            command_tx,
            event_rx,
            state,
            _thread_handle: thread_handle,
        })
    }

    /// Send a command to the audio engine
    pub fn send_command(&self, cmd: PlayerCommand) {
        let _ = self.command_tx.try_send(cmd);
    }

    /// Load a track
    pub fn load(&self, path: PathBuf) {
        self.send_command(PlayerCommand::Load(path));
    }

    /// Load a track and immediately start playing
    pub fn load_and_play(&self, path: PathBuf) {
        self.send_command(PlayerCommand::LoadAndPlay(path));
    }

    /// Start playback
    pub fn play(&self) {
        self.send_command(PlayerCommand::Play);
    }

    /// Pause playback
    pub fn pause(&self) {
        self.send_command(PlayerCommand::Pause);
    }

    /// Stop playback
    pub fn stop(&self) {
        self.send_command(PlayerCommand::Stop);
    }

    /// Seek to position (0.0 to 1.0)
    pub fn seek(&self, position: f32) {
        self.send_command(PlayerCommand::Seek(position));
    }

    /// Set volume (0.0 to 1.0)
    pub fn set_volume(&self, volume: f32) {
        self.send_command(PlayerCommand::SetVolume(volume));
    }

    /// Try to receive an event (non-blocking)
    pub fn try_recv_event(&self) -> Option<PlayerEvent> {
        self.event_rx.try_recv().ok()
    }

    /// Shutdown the audio engine
    pub fn shutdown(&self) {
        let _ = self.command_tx.try_send(PlayerCommand::Shutdown);
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Helper to create a new sink
fn create_sink(stream_handle: &rodio::OutputStreamHandle, volume: f32) -> Option<Sink> {
    Sink::try_new(stream_handle).ok().map(|sink| {
        sink.set_volume(volume);
        sink
    })
}

/// Audio thread main loop
fn audio_thread(
    command_rx: Receiver<PlayerCommand>,
    event_tx: Sender<PlayerEvent>,
    state: Arc<PlaybackState>,
) {
    // Initialize audio output
    let (_stream, stream_handle) = match OutputStream::try_default() {
        Ok(s) => s,
        Err(e) => {
            let _ = event_tx.try_send(PlayerEvent::Error(format!("Failed to open audio: {}", e)));
            return;
        }
    };

    // Create initial sink - we'll recreate it when needed since stop() kills it
    let mut sink: Option<Sink> = create_sink(&stream_handle, *state.volume.lock());

    let mut current_path: Option<PathBuf> = None;
    let mut playback_start_time: Option<std::time::Instant> = None;
    let mut pause_offset_ms: u64 = 0;

    loop {
        // Use shorter timeout when playing for responsive position updates,
        // longer timeout when idle to reduce thread wakeups
        let timeout = if state.is_playing.load(Ordering::Relaxed) && !state.is_paused.load(Ordering::Relaxed) {
            Duration::from_millis(50)
        } else {
            Duration::from_secs(1)
        };

        // Process commands (non-blocking with timeout)
        match command_rx.recv_timeout(timeout) {
            Ok(PlayerCommand::Load(path)) => {
                // Stop current playback by dropping old sink and creating new one
                drop(sink.take());
                sink = create_sink(&stream_handle, *state.volume.lock());

                // Load new file to get duration
                match std::fs::File::open(&path) {
                    Ok(file) => {
                        let buf_reader = std::io::BufReader::new(file);
                        match Decoder::new(buf_reader) {
                            Ok(source) => {
                                // Get duration
                                let duration = source.total_duration()
                                    .unwrap_or(Duration::from_secs(0));
                                let duration_ms = duration.as_millis() as u64;

                                // Update state
                                state.duration_ms.store(duration_ms, Ordering::Relaxed);
                                state.position_ms.store(0, Ordering::Relaxed);
                                state.is_playing.store(false, Ordering::Relaxed);
                                state.is_paused.store(false, Ordering::Relaxed);

                                current_path = Some(path);
                                playback_start_time = None;
                                pause_offset_ms = 0;

                                let _ = event_tx.try_send(PlayerEvent::Loaded { duration_ms });
                            }
                            Err(e) => {
                                let _ = event_tx.try_send(PlayerEvent::Error(
                                    format!("Failed to decode: {}", e)
                                ));
                            }
                        }
                    }
                    Err(e) => {
                        let _ = event_tx.try_send(PlayerEvent::Error(
                            format!("Failed to open file: {}", e)
                        ));
                    }
                }
            }

            Ok(PlayerCommand::LoadAndPlay(path)) => {
                // Stop current playback by dropping old sink and creating new one
                drop(sink.take());
                sink = create_sink(&stream_handle, *state.volume.lock());

                // Load and immediately play (single file open)
                match std::fs::File::open(&path) {
                    Ok(file) => {
                        let buf_reader = std::io::BufReader::new(file);
                        match Decoder::new(buf_reader) {
                            Ok(source) => {
                                // Get duration (doesn't consume the source)
                                let duration = source.total_duration()
                                    .unwrap_or(Duration::from_secs(0));
                                let duration_ms = duration.as_millis() as u64;

                                // Update state
                                state.duration_ms.store(duration_ms, Ordering::Relaxed);
                                state.position_ms.store(0, Ordering::Relaxed);

                                current_path = Some(path.clone());
                                pause_offset_ms = 0;

                                let _ = event_tx.try_send(PlayerEvent::Loaded { duration_ms });

                                // Use the same decoder for playback
                                if let Some(ref s) = sink {
                                    s.append(source);
                                    s.set_volume(*state.volume.lock());
                                    s.play();
                                    playback_start_time = Some(std::time::Instant::now());
                                    state.is_playing.store(true, Ordering::Relaxed);
                                    state.is_paused.store(false, Ordering::Relaxed);
                                    let _ = event_tx.try_send(PlayerEvent::Playing);
                                }
                            }
                            Err(e) => {
                                let _ = event_tx.try_send(PlayerEvent::Error(
                                    format!("Failed to decode: {}", e)
                                ));
                            }
                        }
                    }
                    Err(e) => {
                        let _ = event_tx.try_send(PlayerEvent::Error(
                            format!("Failed to open file: {}", e)
                        ));
                    }
                }
            }

            Ok(PlayerCommand::Play) => {
                if let Some(ref path) = current_path {
                    // Ensure we have a valid sink
                    if sink.is_none() {
                        sink = create_sink(&stream_handle, *state.volume.lock());
                    }

                    if let Some(ref s) = sink {
                        if s.is_paused() {
                            // Resume from pause
                            s.play();
                            playback_start_time = Some(std::time::Instant::now());
                            state.is_playing.store(true, Ordering::Relaxed);
                            state.is_paused.store(false, Ordering::Relaxed);
                            let _ = event_tx.try_send(PlayerEvent::Playing);
                        } else if s.empty() {
                            // Start fresh playback
                            match std::fs::File::open(path) {
                                Ok(file) => {
                                    let buf_reader = std::io::BufReader::new(file);
                                    if let Ok(source) = Decoder::new(buf_reader) {
                                        // Skip to current position if needed
                                        if pause_offset_ms > 0 {
                                            let skip_duration = Duration::from_millis(pause_offset_ms);
                                            s.append(source.skip_duration(skip_duration));
                                        } else {
                                            s.append(source);
                                        }
                                        s.set_volume(*state.volume.lock());
                                        s.play();
                                        playback_start_time = Some(std::time::Instant::now());
                                        state.is_playing.store(true, Ordering::Relaxed);
                                        state.is_paused.store(false, Ordering::Relaxed);
                                        let _ = event_tx.try_send(PlayerEvent::Playing);
                                    }
                                }
                                Err(e) => {
                                    let _ = event_tx.try_send(PlayerEvent::Error(
                                        format!("Failed to open file: {}", e)
                                    ));
                                }
                            }
                        }
                    }
                }
            }

            Ok(PlayerCommand::Pause) => {
                if let Some(ref s) = sink {
                    if !s.is_paused() && state.is_playing.load(Ordering::Relaxed) {
                        // Calculate current position before pausing
                        if let Some(start) = playback_start_time {
                            let elapsed = start.elapsed().as_millis() as u64;
                            pause_offset_ms = pause_offset_ms.saturating_add(elapsed);
                        }
                        s.pause();
                        playback_start_time = None;
                        state.is_playing.store(true, Ordering::Relaxed);
                        state.is_paused.store(true, Ordering::Relaxed);
                        let _ = event_tx.try_send(PlayerEvent::Paused);
                    }
                }
            }

            Ok(PlayerCommand::Stop) => {
                // Drop old sink and create fresh one
                drop(sink.take());
                sink = create_sink(&stream_handle, *state.volume.lock());

                playback_start_time = None;
                pause_offset_ms = 0;
                state.position_ms.store(0, Ordering::Relaxed);
                state.is_playing.store(false, Ordering::Relaxed);
                state.is_paused.store(false, Ordering::Relaxed);
                let _ = event_tx.try_send(PlayerEvent::Stopped);
            }

            Ok(PlayerCommand::Seek(position)) => {
                let duration_ms = state.duration_ms.load(Ordering::Relaxed);
                let target_ms = (position * duration_ms as f32) as u64;

                // Check if was playing before we recreate the sink
                let was_playing = state.is_playing.load(Ordering::Relaxed)
                    && !state.is_paused.load(Ordering::Relaxed);

                // Drop old sink and create fresh one
                drop(sink.take());
                sink = create_sink(&stream_handle, *state.volume.lock());

                pause_offset_ms = target_ms;
                state.position_ms.store(target_ms, Ordering::Relaxed);

                // Restart if was playing
                if was_playing {
                    if let (Some(ref s), Some(ref path)) = (&sink, &current_path) {
                        if let Ok(file) = std::fs::File::open(path) {
                            let buf_reader = std::io::BufReader::new(file);
                            if let Ok(source) = Decoder::new(buf_reader) {
                                let skip_duration = Duration::from_millis(target_ms);
                                s.append(source.skip_duration(skip_duration));
                                s.set_volume(*state.volume.lock());
                                s.play();
                                playback_start_time = Some(std::time::Instant::now());
                            }
                        }
                    }
                }
            }

            Ok(PlayerCommand::SetVolume(volume)) => {
                let volume = volume.clamp(0.0, 1.0);
                *state.volume.lock() = volume;
                if let Some(ref s) = sink {
                    s.set_volume(volume);
                }
            }

            Ok(PlayerCommand::Shutdown) => {
                drop(sink.take());
                break;
            }

            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                // Normal timeout, continue
            }

            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                break;
            }
        }

        // Update position if playing
        if state.is_playing.load(Ordering::Relaxed) && !state.is_paused.load(Ordering::Relaxed) {
            if let Some(start) = playback_start_time {
                let elapsed = start.elapsed().as_millis() as u64;
                let current_pos = pause_offset_ms.saturating_add(elapsed);
                let duration = state.duration_ms.load(Ordering::Relaxed);

                if current_pos >= duration {
                    // Playback finished naturally - recreate sink
                    drop(sink.take());
                    sink = create_sink(&stream_handle, *state.volume.lock());

                    playback_start_time = None;
                    pause_offset_ms = 0;
                    state.position_ms.store(0, Ordering::Relaxed);
                    state.is_playing.store(false, Ordering::Relaxed);
                    state.is_paused.store(false, Ordering::Relaxed);
                    let _ = event_tx.try_send(PlayerEvent::TrackFinished);
                } else {
                    state.position_ms.store(current_pos, Ordering::Relaxed);
                    let _ = event_tx.try_send(PlayerEvent::Position(current_pos));
                }
            }
        }
    }
}

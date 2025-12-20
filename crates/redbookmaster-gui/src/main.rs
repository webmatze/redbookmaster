// Red Book Master GUI
// Professional CD mastering application with Slint UI

mod player;

use std::rc::Rc;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use redbookmaster_lib::{Album, Project, Track, extract_peaks, WaveformData};
use redbookmaster_lib::core::track::format_duration_ms;
use redbookmaster_lib::audio::concat::concatenate_tracks;
use redbookmaster_lib::audio::convert::convert_to_red_book;
use redbookmaster_lib::audio::wav::{read_wav_info, WavInfo};
use redbookmaster_lib::{generate_cue, generate_toc, validate_cd_text};
use redbookmaster_lib::{cdrdao_available, list_drives};
use slint::Model;
use serde::{Deserialize, Serialize};

use player::{AudioEngine, PlayerEvent};

slint::include_modules!();

/// Number of waveform peaks to display
const WAVEFORM_BINS: usize = 500;

/// Show an error dialog with the given title and message
fn show_error_dialog(app: &MainWindow, title: &str, message: &str) {
    app.set_error_title(title.into());
    app.set_error_message(message.into());
    app.set_show_error_dialog(true);
}

/// Unmount any mounted optical discs on macOS
/// This is required before cdrdao can access the drive
#[cfg(target_os = "macos")]
fn unmount_optical_discs() {
    if let Ok(output) = std::process::Command::new("diskutil").args(["list"]).output() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            // Skip disk0 (system disk) and look for optical disc entries
            if line.contains("/dev/disk") && !line.contains("disk0") {
                if let Some(disk) = line.split_whitespace().next() {
                    let _ = std::process::Command::new("diskutil")
                        .args(["unmountDisk", disk])
                        .output();
                }
            }
        }
    }
}

/// User preferences that persist between sessions
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Preferences {
    /// Last window width
    window_width: Option<u32>,
    /// Last window height
    window_height: Option<u32>,
    /// Last opened project directory
    last_project_dir: Option<PathBuf>,
}

impl Preferences {
    /// Get the preferences file path
    fn file_path() -> Option<PathBuf> {
        dirs::config_dir().map(|p| p.join("redbookmaster").join("preferences.json"))
    }

    /// Load preferences from disk
    fn load() -> Self {
        Self::file_path()
            .and_then(|path| std::fs::read_to_string(&path).ok())
            .and_then(|contents| serde_json::from_str(&contents).ok())
            .unwrap_or_default()
    }

    /// Save preferences to disk
    fn save(&self) {
        if let Some(path) = Self::file_path() {
            // Create directory if needed
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            // Write preferences
            if let Ok(contents) = serde_json::to_string_pretty(self) {
                let _ = std::fs::write(&path, contents);
            }
        }
    }
}

/// Action pending after a warning dialog is confirmed
#[derive(Debug, Clone, PartialEq)]
enum PendingWarningAction {
    None,
    Export,
    Burn,
}

/// Application state
struct AppState {
    project: Option<Project>,
    project_path: Option<PathBuf>,
    /// Multi-track waveform cache (track_number -> cache)
    waveform_cache: HashMap<u8, WaveformCache>,
    /// Currently displayed track number
    displayed_track_num: Option<u8>,
    current_track_path: Option<PathBuf>,
    current_track_num: Option<u8>,
    /// Current zoom level (1.0 = full view, 2.0 = 2x zoom, etc.)
    zoom_level: f32,
    /// Scroll offset for zoomed view (0.0 to 1.0 - zoom_range)
    scroll_offset: f32,
    /// Files pending transcoding (path, wav_info)
    pending_transcode_files: Vec<(PathBuf, WavInfo)>,
    /// User preferences
    preferences: Preferences,
    /// Last saved window size (for change detection)
    last_saved_window_size: Option<(u32, u32)>,
    /// Action pending after warning dialog is confirmed
    pending_warning_action: PendingWarningAction,
    /// Skip CD-TEXT validation (set after user confirms warning)
    skip_cd_text_validation: bool,
}

/// Cached waveform data
struct WaveformCache {
    /// Full waveform data for zooming
    waveform_data: WaveformData,
    duration_str: String,
}

impl AppState {
    fn new() -> Self {
        let prefs = Preferences::load();
        let last_size = match (prefs.window_width, prefs.window_height) {
            (Some(w), Some(h)) => Some((w, h)),
            _ => None,
        };
        Self {
            project: None,
            project_path: None,
            waveform_cache: HashMap::new(),
            displayed_track_num: None,
            current_track_path: None,
            current_track_num: None,
            zoom_level: 1.0,
            scroll_offset: 0.0,
            pending_transcode_files: Vec::new(),
            preferences: prefs,
            last_saved_window_size: last_size,
            pending_warning_action: PendingWarningAction::None,
            skip_cd_text_validation: false,
        }
    }

    fn save_preferences(&mut self) {
        // Update last saved size
        self.last_saved_window_size = match (self.preferences.window_width, self.preferences.window_height) {
            (Some(w), Some(h)) => Some((w, h)),
            _ => None,
        };
        self.preferences.save();
    }

    /// Auto-save the project if a path and project exist
    fn auto_save(&self) {
        if let (Some(path), Some(project)) = (&self.project_path, &self.project) {
            if let Err(e) = project.save_to(path) {
                eprintln!("Auto-save failed: {}", e);
            }
        }
    }

    fn tracks_to_model(&self) -> Vec<TrackData> {
        let Some(project) = &self.project else {
            return Vec::new();
        };

        project.album.tracks.iter().map(|track| {
            TrackData {
                number: track.number as i32,
                title: track.title.clone().into(),
                duration: format_duration_ms(track.duration).into(),
                pregap: track.pregap.as_secs().to_string().into(),
                postgap: track.postgap.as_secs().to_string().into(),
                selected: false,
            }
        }).collect()
    }

    fn album_to_model(&self) -> AlbumData {
        let Some(project) = &self.project else {
            return AlbumData::default();
        };

        AlbumData {
            title: project.album.title.clone().into(),
            performer: project.album.performer.clone().into(),
            songwriter: project.album.songwriter.clone().unwrap_or_default().into(),
            catalog: project.album.catalog.as_ref().map(|c| c.to_string()).unwrap_or_default().into(),
        }
    }

    /// Get the track for a given track number
    fn get_track(&self, track_num: u8) -> Option<&Track> {
        self.project.as_ref()?.album.get_track(track_num)
    }

    /// Extract waveform for a track (stores full data for zooming)
    /// Get cached waveform for a track, or None if not cached
    fn get_cached_waveform(&self, track_num: u8) -> Option<&WaveformCache> {
        self.waveform_cache.get(&track_num)
    }

    /// Insert waveform into cache
    fn insert_waveform(&mut self, track_num: u8, waveform_data: WaveformData, duration_str: String) {
        self.waveform_cache.insert(track_num, WaveformCache {
            waveform_data,
            duration_str,
        });
    }

    /// Get peaks for current zoom level and scroll offset for the displayed track
    fn get_visible_peaks(&self) -> Vec<WaveformPeak> {
        let Some(track_num) = self.displayed_track_num else {
            return Vec::new();
        };

        let Some(cache) = self.waveform_cache.get(&track_num) else {
            return Vec::new();
        };

        // Calculate visible range based on zoom and scroll
        let view_size = 1.0 / self.zoom_level;
        let start = self.scroll_offset;
        let end = (start + view_size).min(1.0);

        // Get peaks for the visible range
        let peaks = cache.waveform_data.get_peaks_for_range(start, end, WAVEFORM_BINS);
        peaks.iter().map(|&(min, max)| WaveformPeak { min, max }).collect()
    }

    /// Get visible peaks for a specific track from cache
    fn get_visible_peaks_for_track(&self, track_num: u8) -> Vec<WaveformPeak> {
        let Some(cache) = self.waveform_cache.get(&track_num) else {
            return Vec::new();
        };

        // Calculate visible range based on zoom and scroll
        let view_size = 1.0 / self.zoom_level;
        let start = self.scroll_offset;
        let end = (start + view_size).min(1.0);

        // Get peaks for the visible range
        let peaks = cache.waveform_data.get_peaks_for_range(start, end, WAVEFORM_BINS);
        peaks.iter().map(|&(min, max)| WaveformPeak { min, max }).collect()
    }
}

fn main() -> Result<(), slint::PlatformError> {
    let app = MainWindow::new()?;
    let state = Rc::new(RefCell::new(AppState::new()));

    // Apply saved window size from preferences
    {
        let state_ref = state.borrow();
        if let (Some(width), Some(height)) = (
            state_ref.preferences.window_width,
            state_ref.preferences.window_height,
        ) {
            let size = slint::LogicalSize::new(width as f32, height as f32);
            app.window().set_size(size);
        }
    }

    // Initialize audio engine
    let audio_engine = match AudioEngine::new() {
        Ok(engine) => Arc::new(engine),
        Err(e) => {
            eprintln!("Failed to initialize audio: {}", e);
            return Err(slint::PlatformError::Other(e));
        }
    };

    // Initialize with empty state
    app.set_status_message("Welcome to Red Book Master".into());

    // Wire up callbacks
    let app_weak = app.as_weak();
    let state_clone = state.clone();
    app.on_new_project(move || {
        // Use last project directory if available
        let last_dir = {
            let state = state_clone.borrow();
            state.preferences.last_project_dir.clone()
        };

        // Show save file dialog - user picks name and location
        let mut dialog = rfd::FileDialog::new()
            .set_title("Create New Project")
            .add_filter("Red Book Master Project", &["rbm"])
            .set_file_name("untitled.rbm");

        if let Some(ref dir) = last_dir {
            dialog = dialog.set_directory(dir);
        }

        if let Some(path) = dialog.save_file() {
            // Get project name from the chosen filename (without extension)
            let project_name = path.file_stem()
                .and_then(|n| n.to_str())
                .unwrap_or("project")
                .to_string();

            // Get parent directory where user wants to save
            let parent_dir = path.parent().unwrap_or(std::path::Path::new("."));

            // Create project directory: {parent}/{name}_rbm/
            let project_dir = parent_dir.join(format!("{}_rbm", &project_name));
            if let Err(e) = std::fs::create_dir_all(&project_dir) {
                if let Some(app) = app_weak.upgrade() {
                    app.set_status_message(format!("Failed to create project directory: {}", e).into());
                }
                return;
            }

            // Create project file inside: {parent}/{name}_rbm/{name}.rbm
            let project_file = project_dir.join(format!("{}.rbm", &project_name));

            let mut state = state_clone.borrow_mut();

            // Create project with project name as album title
            let album = Album::new(project_name.clone(), "".to_string());
            let mut project = Project::new(album);
            project.project_dir = Some(project_dir.clone());
            project.file_path = Some(project_file.clone());

            // Auto-save immediately
            if let Err(e) = project.save_to(&project_file) {
                if let Some(app) = app_weak.upgrade() {
                    app.set_status_message(format!("Failed to create project: {}", e).into());
                }
                return;
            }

            state.project = Some(project);
            state.project_path = Some(project_file.clone());
            state.waveform_cache.clear();
            state.displayed_track_num = None;
            state.current_track_path = None;
            state.current_track_num = None;
            state.zoom_level = 1.0;
            state.scroll_offset = 0.0;

            // Save last project directory and window size
            state.preferences.last_project_dir = Some(parent_dir.to_path_buf());
            if let Some(app) = app_weak.upgrade() {
                // Convert physical pixels to logical pixels for cross-DPI consistency
                let size = app.window().size();
                let scale = app.window().scale_factor();
                state.preferences.window_width = Some((size.width as f32 / scale) as u32);
                state.preferences.window_height = Some((size.height as f32 / scale) as u32);
            }
            state.save_preferences();

            if let Some(app) = app_weak.upgrade() {
                let tracks: Vec<TrackData> = state.tracks_to_model();
                let model = Rc::new(slint::VecModel::from(tracks));
                app.set_tracks(model.into());
                app.set_album(state.album_to_model());
                app.set_status_message(format!("Created project: {}", project_dir.display()).into());
                app.set_selected_track_index(-1);
                app.set_has_project(true);
                // Clear waveform
                app.set_waveform_peaks(Rc::new(slint::VecModel::from(Vec::<WaveformPeak>::new())).into());
                app.set_waveform_duration("0:00".into());
            }
        }
    });

    let app_weak = app.as_weak();
    let state_clone = state.clone();
    app.on_open_project(move || {
        // Use last project directory if available
        let last_dir = {
            let state = state_clone.borrow();
            state.preferences.last_project_dir.clone()
        };

        let mut dialog = rfd::FileDialog::new()
            .add_filter("Red Book Master Project", &["rbm"])
            .set_title("Open Project");

        if let Some(ref dir) = last_dir {
            dialog = dialog.set_directory(dir);
        }

        if let Some(path) = dialog.pick_file() {
            match Project::load(&path) {
                Ok(project) => {
                    let mut state = state_clone.borrow_mut();
                    state.project = Some(project);
                    state.project_path = Some(path.clone());
                    state.waveform_cache.clear();
                    state.displayed_track_num = None;

                    // Save last project directory
                    if let Some(parent) = path.parent() {
                        state.preferences.last_project_dir = Some(parent.to_path_buf());
                    }
                    // Save window size (convert physical to logical pixels)
                    if let Some(app) = app_weak.upgrade() {
                        let size = app.window().size();
                        let scale = app.window().scale_factor();
                        state.preferences.window_width = Some((size.width as f32 / scale) as u32);
                        state.preferences.window_height = Some((size.height as f32 / scale) as u32);
                    }
                    state.save_preferences();

                    if let Some(app) = app_weak.upgrade() {
                        let tracks: Vec<TrackData> = state.tracks_to_model();
                        let model = Rc::new(slint::VecModel::from(tracks));
                        app.set_tracks(model.into());
                        app.set_album(state.album_to_model());
                        app.set_status_message(format!("Opened: {}", path.display()).into());
                        app.set_selected_track_index(-1);
                        app.set_has_project(true);
                        // Clear waveform
                        app.set_waveform_peaks(Rc::new(slint::VecModel::from(Vec::<WaveformPeak>::new())).into());
                        app.set_waveform_duration("0:00".into());
                    }
                }
                Err(e) => {
                    eprintln!("Failed to open project: {}", e);
                    if let Some(app) = app_weak.upgrade() {
                        show_error_dialog(&app, "Open Failed", &format!("Failed to open project:\n{}", e));
                    }
                }
            }
        }
    });

    let app_weak = app.as_weak();
    let state_clone = state.clone();
    app.on_save_project(move || {
        let has_path = {
            let state = state_clone.borrow();
            if state.project.is_none() {
                return;
            }
            state.project_path.is_some()
        };

        if has_path {
            let state = state_clone.borrow();
            if let Some(ref project) = state.project {
                if let Some(ref path) = state.project_path {
                    match project.save_to(path) {
                        Ok(()) => {
                            if let Some(app) = app_weak.upgrade() {
                                app.set_status_message("Project saved".into());
                            }
                        }
                        Err(e) => {
                            if let Some(app) = app_weak.upgrade() {
                                app.set_status_message(format!("Save failed: {}", e).into());
                            }
                        }
                    }
                }
            }
        } else {
            // No path set, trigger Save As
            if let Some(app) = app_weak.upgrade() {
                app.invoke_save_project_as();
            }
        }
    });

    let app_weak = app.as_weak();
    let state_clone = state.clone();
    app.on_add_tracks(move || {
        // First check if we have a project
        {
            let state = state_clone.borrow();
            if state.project.is_none() {
                if let Some(app) = app_weak.upgrade() {
                    app.set_status_message("Create a project first (File → New Project)".into());
                }
                return;
            }
        }

        let dialog = rfd::FileDialog::new()
            .add_filter("WAV Audio", &["wav", "WAV"])
            .set_title("Select WAV files to add");

        if let Some(files) = dialog.pick_files() {
            let mut state = state_clone.borrow_mut();

            let project = state.project.as_mut().unwrap();
            let mut added = 0;
            let mut needs_transcoding: Vec<(PathBuf, WavInfo)> = Vec::new();

            for path in files {
                match read_wav_info(&path) {
                    Ok(info) => {
                        if info.is_red_book_compliant() {
                            // Get title from filename
                            let title = path.file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or("Unknown")
                                .to_string();

                            let track_num = project.album.track_count() + 1;

                            // Create track with ABSOLUTE path (no copying)
                            let track = Track::new(
                                track_num as u8,
                                title,
                                path.clone(),  // Store original absolute path
                                info.duration,
                            );
                            project.album.add_track(track);
                            added += 1;
                        } else {
                            // Collect non-compliant files for transcoding dialog
                            needs_transcoding.push((path, info));
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to read WAV: {}", e);
                    }
                }
            }

            // Auto-save project after adding tracks
            if added > 0 {
                state.auto_save();
            }

            if let Some(app) = app_weak.upgrade() {
                // Update track list with compliant files added so far
                let tracks: Vec<TrackData> = state.tracks_to_model();
                let model = Rc::new(slint::VecModel::from(tracks));
                app.set_tracks(model.into());
                app.set_album(state.album_to_model());

                if !needs_transcoding.is_empty() {
                    // Build transcode file info for dialog
                    let transcode_infos: Vec<TranscodeFileInfo> = needs_transcoding.iter()
                        .map(|(path, info)| {
                            let filename = path.file_name()
                                .and_then(|s| s.to_str())
                                .unwrap_or("Unknown")
                                .to_string();

                            // Format issues in a user-friendly way
                            let issues = info.format_issues().join(", ")
                                .replace("Hz (needs 44100Hz)", "Hz → 44100Hz")
                                .replace("bits (needs 16 bits)", "-bit → 16-bit")
                                .replace("(needs stereo)", "→ Stereo");

                            TranscodeFileInfo {
                                filename: filename.into(),
                                issues: issues.into(),
                                path: path.to_string_lossy().to_string().into(),
                            }
                        })
                        .collect();

                    // Store pending files and show dialog
                    state.pending_transcode_files = needs_transcoding;

                    let model = Rc::new(slint::VecModel::from(transcode_infos));
                    app.set_transcode_files(model.into());
                    app.set_show_transcode_dialog(true);
                    app.set_is_transcoding(false);
                    app.set_transcode_progress(0.0);
                    app.set_transcode_status("".into());

                    app.set_status_message(format!(
                        "Added {} track(s), {} need conversion",
                        added,
                        state.pending_transcode_files.len()
                    ).into());
                } else {
                    app.set_status_message(format!("Added {} track(s)", added).into());
                }
            }
        }
    });

    // Shared containers for async waveform loading (thread-safe)
    // Stores the pending request: (track_num, path, duration)
    let waveform_pending: Arc<std::sync::Mutex<Option<(u8, PathBuf, std::time::Duration)>>> =
        Arc::new(std::sync::Mutex::new(None));
    // Stores the result: (track_num, waveform_data, duration_str)
    let waveform_result: Arc<std::sync::Mutex<Option<(u8, WaveformData, String)>>> =
        Arc::new(std::sync::Mutex::new(None));
    // Flag indicating if a worker thread is currently running
    let waveform_worker_active: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));

    // Track selection with async waveform loading
    let app_weak = app.as_weak();
    let state_clone = state.clone();
    let engine_clone = audio_engine.clone();
    let wf_pending = waveform_pending.clone();
    let wf_result = waveform_result.clone();
    let wf_worker_active = waveform_worker_active.clone();
    app.on_select_track(move |track_num| {
        let mut state = state_clone.borrow_mut();
        let track_num_u8 = track_num as u8;

        if let Some(app) = app_weak.upgrade() {
            // Find the track index and update metadata editor
            let tracks = app.get_tracks();
            for i in 0..tracks.row_count() {
                if let Some(track) = tracks.row_data(i) {
                    if track.number == track_num {
                        app.set_selected_track_index(i as i32);

                        // Update metadata editor fields
                        app.set_current_track_title(track.title.clone());
                        app.set_current_track_pregap(track.pregap.clone());
                        break;
                    }
                }
            }
        }

        // Check if we were playing before switching tracks
        let was_playing = if let Some(app) = app_weak.upgrade() {
            let playback = app.get_playback();
            playback.is_playing && !playback.is_paused
        } else {
            false
        };

        // Get track info for audio playback and async waveform loading
        let track_info = if let Some(track) = state.get_track(track_num_u8) {
            // Resolve relative path using project_dir
            let path = state.project.as_ref()
                .and_then(|p| p.project_dir.as_ref())
                .map(|dir| track.resolve_source_file(dir))
                .unwrap_or_else(|| track.source_file.clone());
            let duration = track.duration;
            state.current_track_path = Some(path.clone());
            state.current_track_num = Some(track_num_u8);
            Some((path, duration))
        } else {
            None
        };

        // Load track into audio engine
        if let Some((ref path, _)) = track_info {
            if was_playing {
                engine_clone.load_and_play(path.clone());
            } else {
                engine_clone.load(path.clone());
            }
        }

        // Update displayed track number
        state.displayed_track_num = Some(track_num_u8);

        // Reset zoom when switching tracks
        state.zoom_level = 1.0;
        state.scroll_offset = 0.0;

        // Check if waveform is already cached
        if state.get_cached_waveform(track_num_u8).is_some() {
            // Cache hit - update UI immediately
            let peaks = state.get_visible_peaks_for_track(track_num_u8);
            let duration = state.waveform_cache.get(&track_num_u8)
                .map(|c| c.duration_str.clone())
                .unwrap_or_default();

            drop(state); // Release borrow before UI updates

            if let Some(app) = app_weak.upgrade() {
                let model = Rc::new(slint::VecModel::from(peaks));
                app.set_waveform_peaks(model.into());
                app.set_waveform_duration(duration.into());
                app.set_waveform_loading(false);
                app.set_zoom_level(1.0);
                app.set_waveform_scroll_offset(0.0);
                app.set_status_message(format!("Track {} selected", track_num).into());
                app.set_playhead_position(0.0);
            }
            return;
        }

        // Cache miss - start async loading
        if let Some(app) = app_weak.upgrade() {
            app.set_waveform_loading(true);
            app.set_zoom_level(1.0);
            app.set_waveform_scroll_offset(0.0);
            app.set_status_message(format!("Loading waveform for track {}...", track_num).into());
            app.set_playhead_position(0.0);
            // Clear waveform display while loading
            app.set_waveform_peaks(Rc::new(slint::VecModel::from(Vec::<WaveformPeak>::new())).into());
        }

        // Get track info for background thread
        let Some((track_path, track_duration)) = track_info else {
            return;
        };

        // Store the pending request (this cancels any previous request)
        if let Ok(mut pending) = wf_pending.lock() {
            *pending = Some((track_num_u8, track_path.clone(), track_duration));
        }

        // Only spawn a new worker thread if one isn't already running
        if !wf_worker_active.swap(true, Ordering::SeqCst) {
            let pending_clone = wf_pending.clone();
            let result_clone = wf_result.clone();
            let worker_active_clone = wf_worker_active.clone();

            std::thread::spawn(move || {
                loop {
                    // Get the pending request
                    let request = {
                        let mut pending = pending_clone.lock().unwrap();
                        pending.take()
                    };

                    let Some((track_num, track_path, track_duration)) = request else {
                        break; // No more requests
                    };

                    // Extract waveform (this is the slow part)
                    const FULL_PEAKS: usize = WAVEFORM_BINS * 16;
                    if let Ok(waveform_data) = extract_peaks(&track_path, FULL_PEAKS) {
                        let duration_str = format_duration_ms(track_duration);

                        // Check if there's a newer request - if so, discard this result
                        let has_newer_request = {
                            let pending = pending_clone.lock().unwrap();
                            pending.is_some()
                        };

                        if !has_newer_request {
                            // Store the result
                            if let Ok(mut result) = result_clone.lock() {
                                *result = Some((track_num, waveform_data, duration_str));
                            }
                        }
                    }

                    // Check if there's another request pending
                    let has_pending = {
                        let pending = pending_clone.lock().unwrap();
                        pending.is_some()
                    };

                    if !has_pending {
                        break;
                    }
                }
                worker_active_clone.store(false, Ordering::SeqCst);
            });
        }
    });

    let state_clone = state.clone();
    let app_weak = app.as_weak();
    app.on_update_album_title(move |title| {
        let mut state = state_clone.borrow_mut();
        if let Some(ref mut project) = state.project {
            project.album.title = title.to_string();
            if let Some(app) = app_weak.upgrade() {
                app.set_album(state.album_to_model());
            }
            state.auto_save();
        }
    });

    let state_clone = state.clone();
    let app_weak = app.as_weak();
    app.on_update_album_performer(move |performer| {
        let mut state = state_clone.borrow_mut();
        if let Some(ref mut project) = state.project {
            project.album.performer = performer.to_string();
            if let Some(app) = app_weak.upgrade() {
                app.set_album(state.album_to_model());
            }
            state.auto_save();
        }
    });

    let state_clone = state.clone();
    let app_weak = app.as_weak();
    app.on_update_track_title(move |track_num, title| {
        let mut state = state_clone.borrow_mut();
        if let Some(ref mut project) = state.project {
            if let Some(track) = project.album.get_track_mut(track_num as u8) {
                track.title = title.to_string();
                if let Some(app) = app_weak.upgrade() {
                    let tracks: Vec<TrackData> = state.tracks_to_model();
                    let model = Rc::new(slint::VecModel::from(tracks));
                    app.set_tracks(model.into());
                }
                state.auto_save();
            }
        }
    });

    // Playback callbacks
    let engine_clone = audio_engine.clone();
    let app_weak = app.as_weak();
    app.on_play(move || {
        if let Some(app) = app_weak.upgrade() {
            let tracks = app.get_tracks();

            // Don't play if there are no tracks
            if tracks.row_count() == 0 {
                return;
            }

            // Auto-select first track if none is selected
            let selected_index = app.get_selected_track_index();
            if selected_index < 0 {
                // Select the first track (this will trigger on_select_track callback
                // which handles waveform loading asynchronously)
                if let Some(first_track) = tracks.row_data(0) {
                    app.invoke_select_track(first_track.number);
                }
            }

            engine_clone.play();
            let mut playback = app.get_playback();
            playback.is_playing = true;
            playback.is_paused = false;
            app.set_playback(playback);
            app.set_status_message("Playing".into());
        }
    });

    let engine_clone = audio_engine.clone();
    let app_weak = app.as_weak();
    app.on_pause(move || {
        engine_clone.pause();
        if let Some(app) = app_weak.upgrade() {
            let mut playback = app.get_playback();
            playback.is_paused = true;
            app.set_playback(playback);
            app.set_status_message("Paused".into());
        }
    });

    let engine_clone = audio_engine.clone();
    let app_weak = app.as_weak();
    app.on_stop(move || {
        engine_clone.stop();
        if let Some(app) = app_weak.upgrade() {
            let mut playback = app.get_playback();
            playback.is_playing = false;
            playback.is_paused = false;
            playback.position = 0.0;
            app.set_playback(playback);
            app.set_playhead_position(0.0);
            app.set_status_message("Stopped".into());
        }
    });

    // Previous track
    let app_weak = app.as_weak();
    app.on_prev_track(move || {
        if let Some(app) = app_weak.upgrade() {
            let current_index = app.get_selected_track_index();
            if current_index > 0 {
                let tracks = app.get_tracks();
                if let Some(track) = tracks.row_data((current_index - 1) as usize) {
                    app.invoke_select_track(track.number);
                }
            }
        }
    });

    // Next track
    let app_weak = app.as_weak();
    app.on_next_track(move || {
        if let Some(app) = app_weak.upgrade() {
            let current_index = app.get_selected_track_index();
            let tracks = app.get_tracks();
            if current_index < (tracks.row_count() as i32) - 1 {
                if let Some(track) = tracks.row_data((current_index + 1) as usize) {
                    app.invoke_select_track(track.number);
                }
            }
        }
    });

    let engine_clone = audio_engine.clone();
    app.on_seek(move |pos| {
        engine_clone.seek(pos);
    });

    let engine_clone = audio_engine.clone();
    let app_weak = app.as_weak();
    app.on_set_volume(move |vol| {
        engine_clone.set_volume(vol);
        // Update UI to reflect new volume
        if let Some(app) = app_weak.upgrade() {
            let mut playback = app.get_playback();
            playback.volume = vol;
            app.set_playback(playback);
        }
    });

    let engine_clone = audio_engine.clone();
    let app_weak = app.as_weak();
    app.on_waveform_seek(move |pos| {
        engine_clone.seek(pos);
        if let Some(app) = app_weak.upgrade() {
            app.set_playhead_position(pos);
        }
    });

    let state_clone = state.clone();
    let app_weak = app.as_weak();
    app.on_zoom_in(move || {
        let mut state = state_clone.borrow_mut();
        let Some(track_num) = state.displayed_track_num else {
            return;
        };
        if state.waveform_cache.get(&track_num).is_none() {
            return;
        }

        // Zoom in (max 16x)
        let old_zoom = state.zoom_level;
        state.zoom_level = (state.zoom_level * 2.0).min(16.0);

        // Adjust scroll to keep center in view
        let old_view_size = 1.0 / old_zoom;
        let new_view_size = 1.0 / state.zoom_level;
        let center = state.scroll_offset + old_view_size / 2.0;
        state.scroll_offset = (center - new_view_size / 2.0).max(0.0).min(1.0 - new_view_size);

        // Get updated peaks for new zoom level
        let peaks = state.get_visible_peaks();
        let zoom_level = state.zoom_level;
        let scroll_offset = state.scroll_offset;

        if let Some(app) = app_weak.upgrade() {
            let model = Rc::new(slint::VecModel::from(peaks));
            app.set_waveform_peaks(model.into());
            app.set_zoom_level(zoom_level);
            app.set_waveform_scroll_offset(scroll_offset);
        }
    });

    let state_clone = state.clone();
    let app_weak = app.as_weak();
    app.on_zoom_out(move || {
        let mut state = state_clone.borrow_mut();
        let Some(track_num) = state.displayed_track_num else {
            return;
        };
        if state.waveform_cache.get(&track_num).is_none() {
            return;
        }

        // Zoom out (min 1x)
        let old_zoom = state.zoom_level;
        state.zoom_level = (state.zoom_level / 2.0).max(1.0);

        // Adjust scroll to keep center in view
        let old_view_size = 1.0 / old_zoom;
        let new_view_size = 1.0 / state.zoom_level;
        let center = state.scroll_offset + old_view_size / 2.0;
        state.scroll_offset = (center - new_view_size / 2.0).max(0.0).min(1.0 - new_view_size);

        // Get updated peaks for new zoom level
        let peaks = state.get_visible_peaks();
        let zoom_level = state.zoom_level;
        let scroll_offset = state.scroll_offset;

        if let Some(app) = app_weak.upgrade() {
            let model = Rc::new(slint::VecModel::from(peaks));
            app.set_waveform_peaks(model.into());
            app.set_zoom_level(zoom_level);
            app.set_waveform_scroll_offset(scroll_offset);
        }
    });

    let state_clone = state.clone();
    let app_weak = app.as_weak();
    app.on_waveform_scroll(move |delta| {
        let mut state = state_clone.borrow_mut();
        let Some(track_num) = state.displayed_track_num else {
            return;
        };
        if state.waveform_cache.get(&track_num).is_none() || state.zoom_level <= 1.0 {
            return; // No scrolling at 1x zoom
        }

        // Calculate view size and max scroll
        let view_size = 1.0 / state.zoom_level;
        let max_scroll = 1.0 - view_size;

        // Update scroll offset
        state.scroll_offset = (state.scroll_offset + delta * 0.1).max(0.0).min(max_scroll);

        // Get updated peaks for new scroll position
        let peaks = state.get_visible_peaks();
        let scroll_offset = state.scroll_offset;

        if let Some(app) = app_weak.upgrade() {
            let model = Rc::new(slint::VecModel::from(peaks));
            app.set_waveform_peaks(model.into());
            app.set_waveform_scroll_offset(scroll_offset);
        }
    });

    let state_clone = state.clone();
    let app_weak = app.as_weak();
    app.on_export_master(move || {
        // Gather all data we need in a scope, then release the borrow
        let (project_dir, album, skip_validation) = {
            let state = state_clone.borrow();
            let Some(ref project) = state.project else {
                if let Some(app) = app_weak.upgrade() {
                    show_error_dialog(&app, "Export Error", "No project to export. Create or open a project first.");
                }
                return;
            };

            // Require project directory
            let Some(ref project_dir) = project.project_dir else {
                if let Some(app) = app_weak.upgrade() {
                    show_error_dialog(&app, "Export Error", "No project directory. Save the project first.");
                }
                return;
            };

            if project.album.tracks.is_empty() {
                if let Some(app) = app_weak.upgrade() {
                    show_error_dialog(&app, "Export Error", "No tracks to export. Add some tracks first.");
                }
                return;
            }

            // Validate album
            if let Err(e) = project.album.validate() {
                if let Some(app) = app_weak.upgrade() {
                    show_error_dialog(&app, "Validation Error", &format!("{}", e));
                }
                return;
            }

            (project_dir.clone(), project.album.clone(), state.skip_cd_text_validation)
        };

        // Validate CD-TEXT for compatibility issues (unless user already confirmed)
        if !skip_validation {
            let cd_text_validation = validate_cd_text(&album);
            if cd_text_validation.has_warnings() {
                // Store pending action and show warning
                state_clone.borrow_mut().pending_warning_action = PendingWarningAction::Export;
                if let Some(app) = app_weak.upgrade() {
                    app.set_warning_title("CD-TEXT Warning".into());
                    app.set_warning_message(format!(
                        "Some metadata may cause CD-TEXT issues:\n\n{}",
                        cd_text_validation.format_warnings()
                    ).into());
                    app.set_show_warning_dialog(true);
                }
                return;
            }
        }
        // Reset skip flag for next time
        state_clone.borrow_mut().skip_cd_text_validation = false;

        // Generate filenames based on album title
        let base_name = album.sanitized_base_name();

        let wav_path = project_dir.join(format!("{}.wav", base_name));
        let cue_path = project_dir.join(format!("{}.cue", base_name));
        let toc_path = project_dir.join(format!("{}.toc", base_name));

        // Create tracks with resolved paths for concatenation (before spawning thread)
        let resolved_tracks: Vec<Track> = album.tracks.iter().map(|t| {
            let mut resolved = t.clone();
            resolved.source_file = t.resolve_source_file(&project_dir);
            resolved
        }).collect();

        // Show export dialog immediately
        if let Some(app) = app_weak.upgrade() {
            app.set_show_export_dialog(true);
            app.set_is_exporting(true);
            app.set_export_complete(false);
            app.set_export_error(false);
            app.set_export_status("Creating master WAV file...".into());
            app.set_export_output_dir(project_dir.display().to_string().into());
            app.set_status_message("Exporting...".into());
        }

        // Spawn background thread for export work
        let app_weak_thread = app_weak.clone();
        let album_for_thread = album.clone();
        let project_dir_for_status = project_dir.clone();
        std::thread::spawn(move || {
            // Step 1: Concatenate tracks into master WAV
            if let Err(e) = concatenate_tracks(&resolved_tracks, &wav_path) {
                let error_msg = format!("Failed to create master WAV: {}", e);
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(app) = app_weak_thread.upgrade() {
                        app.set_is_exporting(false);
                        app.set_export_error(true);
                        app.set_export_error_message(error_msg.into());
                    }
                });
                return;
            }

            // Step 2: Generate CUE sheet
            let app_weak_step2 = app_weak_thread.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(app) = app_weak_step2.upgrade() {
                    app.set_export_status("Generating CUE sheet...".into());
                }
            });

            let wav_filename = wav_path.file_name().unwrap().to_str().unwrap().to_string();
            if let Err(e) = generate_cue(&album_for_thread, &wav_filename, &cue_path) {
                let error_msg = format!("Failed to generate CUE sheet: {}", e);
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(app) = app_weak_thread.upgrade() {
                        app.set_is_exporting(false);
                        app.set_export_error(true);
                        app.set_export_error_message(error_msg.into());
                    }
                });
                return;
            }

            // Step 3: Generate TOC file for cdrdao
            let app_weak_step3 = app_weak_thread.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(app) = app_weak_step3.upgrade() {
                    app.set_export_status("Generating TOC file...".into());
                }
            });

            if let Err(e) = generate_toc(&album_for_thread, &wav_filename, &toc_path) {
                let error_msg = format!("Failed to generate TOC file: {}", e);
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(app) = app_weak_thread.upgrade() {
                        app.set_is_exporting(false);
                        app.set_export_error(true);
                        app.set_export_error_message(error_msg.into());
                    }
                });
                return;
            }

            // Success! Show created files
            let wav_name = wav_path.file_name().unwrap().to_str().unwrap().to_string();
            let cue_name = cue_path.file_name().unwrap().to_str().unwrap().to_string();
            let toc_name = toc_path.file_name().unwrap().to_str().unwrap().to_string();
            let output_dir = project_dir_for_status.display().to_string();

            let _ = slint::invoke_from_event_loop(move || {
                if let Some(app) = app_weak_thread.upgrade() {
                    app.set_is_exporting(false);
                    app.set_export_complete(true);

                    // Create list of created files
                    let created_files: Vec<slint::SharedString> = vec![
                        wav_name.into(),
                        cue_name.into(),
                        toc_name.into(),
                    ];
                    let files_model: Rc<slint::VecModel<slint::SharedString>> = Rc::new(slint::VecModel::from(created_files));
                    app.set_export_created_files(files_model.into());

                    app.set_status_message(format!("Exported to {}", output_dir).into());
                }
            });
        });
    });

    // on_burn_cd - Opens the burn dialog
    let state_clone = state.clone();
    let app_weak = app.as_weak();
    app.on_burn_cd(move || {
        // Check if cdrdao is available
        if !cdrdao_available() {
            if let Some(app) = app_weak.upgrade() {
                show_error_dialog(&app, "cdrdao Not Found", "cdrdao is required for CD burning.\n\nInstall it with: brew install cdrdao");
            }
            return;
        }

        let state = state_clone.borrow();
        let Some(ref project) = state.project else {
            if let Some(app) = app_weak.upgrade() {
                show_error_dialog(&app, "Burn Error", "No project loaded. Create or open a project first.");
            }
            return;
        };

        if project.album.tracks.is_empty() {
            if let Some(app) = app_weak.upgrade() {
                show_error_dialog(&app, "Burn Error", "No tracks to burn. Add some tracks first.");
            }
            return;
        }

        // Check if TOC file exists in project directory
        let project_dir = project.project_dir.as_ref();
        let toc_exists = project_dir.map(|dir| {
            dir.join(format!("{}.toc", project.sanitized_base_name())).exists()
        }).unwrap_or(false);

        if !toc_exists {
            if let Some(app) = app_weak.upgrade() {
                show_error_dialog(&app, "Export Required", "Please export the project first to generate the TOC file needed for burning.");
            }
            return;
        }

        // On macOS, unmount any mounted optical discs first
        #[cfg(target_os = "macos")]
        {
            if let Some(app) = app_weak.upgrade() {
                app.set_status_message("Detecting CD drives...".into());
            }
            unmount_optical_discs();
        }

        // List available CD drives
        let mut drives = list_drives().unwrap_or_default();

        // On macOS, if no drives found via scanbus, try IORegistry detection
        #[cfg(target_os = "macos")]
        if drives.is_empty() {
            if let Ok(output) = std::process::Command::new("ioreg")
                .args(["-c", "IOCDBlockStorageDevice", "-r", "-l"])
                .output()
            {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if stdout.contains("IOCDBlockStorageDevice") {
                    drives.push(redbookmaster_lib::CdDrive {
                        device: "IOCompactDiscServices".to_string(),
                        vendor: "Apple".to_string(),
                        model: "Optical Drive".to_string(),
                    });
                }
            }
        }

        // Convert drives to Slint format
        let slint_drives: Vec<CdDriveInfo> = drives.iter().map(|d| {
            CdDriveInfo {
                device: d.device.clone().into(),
                vendor: d.vendor.clone().into(),
                model: d.model.clone().into(),
                display_name: format!("{} {}", d.vendor.trim(), d.model.trim()).into(),
            }
        }).collect();

        let drive_names: Vec<slint::SharedString> = slint_drives.iter()
            .map(|d| d.display_name.clone())
            .collect();

        // Speed options
        let speed_options = vec![
            BurnSpeedOption { value: 0, label: "Auto (recommended)".into() },
            BurnSpeedOption { value: 1, label: "1x".into() },
            BurnSpeedOption { value: 2, label: "2x".into() },
            BurnSpeedOption { value: 4, label: "4x".into() },
            BurnSpeedOption { value: 8, label: "8x".into() },
            BurnSpeedOption { value: 16, label: "16x".into() },
            BurnSpeedOption { value: 24, label: "24x".into() },
            BurnSpeedOption { value: 48, label: "48x".into() },
        ];

        let speed_labels: Vec<slint::SharedString> = speed_options.iter()
            .map(|s| s.label.clone())
            .collect();

        if let Some(app) = app_weak.upgrade() {
            // Reset dialog state
            app.set_is_burning(false);
            app.set_burn_complete(false);
            app.set_burn_error(false);
            app.set_burn_progress(0.0);
            app.set_burn_status("".into());
            app.set_burn_log("".into());
            app.set_burn_result_message("".into());

            // Set dialog options
            app.set_burn_selected_drive_index(0);
            app.set_burn_selected_speed_index(0);
            app.set_burn_eject(true);
            app.set_burn_cd_text(false);
            app.set_cd_text_driver_index(0);  // Default to Auto
            app.set_burn_simulate(false);

            // Set CD-TEXT driver options
            let driver_labels: Vec<slint::SharedString> = vec![
                "Auto".into(),
                "Raw Mode".into(),
                "Sub-channel Mode".into(),
                "CUE Sheet Mode (Pioneer)".into(),
            ];
            let driver_labels_model: Rc<slint::VecModel<slint::SharedString>> = Rc::new(slint::VecModel::from(driver_labels));
            app.set_cd_text_driver_labels(driver_labels_model.into());

            // Set drives and speed options
            let drives_model: Rc<slint::VecModel<CdDriveInfo>> = Rc::new(slint::VecModel::from(slint_drives));
            app.set_burn_available_drives(drives_model.into());

            let drive_names_model: Rc<slint::VecModel<slint::SharedString>> = Rc::new(slint::VecModel::from(drive_names));
            app.set_burn_drive_names(drive_names_model.into());

            let speeds_model: Rc<slint::VecModel<BurnSpeedOption>> = Rc::new(slint::VecModel::from(speed_options));
            app.set_burn_speed_options(speeds_model.into());

            let speed_labels_model: Rc<slint::VecModel<slint::SharedString>> = Rc::new(slint::VecModel::from(speed_labels));
            app.set_burn_speed_labels(speed_labels_model.into());

            // Show dialog
            app.set_show_burn_dialog(true);
            app.set_status_message("Configure burn settings...".into());
        }
    });

    // Shared state for burn thread communication
    let burn_log: Arc<std::sync::Mutex<Vec<String>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
    let burn_complete: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    let burn_error: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    let burn_error_message: Arc<std::sync::Mutex<String>> = Arc::new(std::sync::Mutex::new(String::new()));
    let burn_child_process: Arc<std::sync::Mutex<Option<std::process::Child>>> = Arc::new(std::sync::Mutex::new(None));

    // on_start_burn - Starts the burn process in background
    let state_clone = state.clone();
    let app_weak = app.as_weak();
    let burn_log_clone = burn_log.clone();
    let burn_complete_clone = burn_complete.clone();
    let burn_error_clone = burn_error.clone();
    let burn_error_msg_clone = burn_error_message.clone();
    let burn_child_clone = burn_child_process.clone();
    app.on_start_burn(move || {
        // Get burn settings from UI
        let (device, speed, eject, cd_text, cd_text_driver_index, simulate, toc_path) = {
            let state = state_clone.borrow();
            let Some(ref project) = state.project else { return; };
            let Some(ref project_dir) = project.project_dir else { return; };

            let app = match app_weak.upgrade() {
                Some(a) => a,
                None => return,
            };

            let drive_idx = app.get_burn_selected_drive_index() as usize;
            let speed_idx = app.get_burn_selected_speed_index() as usize;
            let eject = app.get_burn_eject();
            let cd_text = app.get_burn_cd_text();
            let cd_text_driver_index = app.get_cd_text_driver_index();
            let simulate = app.get_burn_simulate();

            // Get drive device from available drives
            let drives = app.get_burn_available_drives();
            let device = if drive_idx < drives.row_count() {
                drives.row_data(drive_idx).map(|d| d.device.to_string()).unwrap_or_default()
            } else {
                return;
            };

            // Get speed value from speed options
            let speeds = app.get_burn_speed_options();
            let speed = if speed_idx < speeds.row_count() {
                speeds.row_data(speed_idx).map(|s| s.value as u32).unwrap_or(0)
            } else {
                0
            };

            // Get TOC file path
            let toc_path = project_dir.join(format!("{}.toc", project.sanitized_base_name()));

            (device, speed, eject, cd_text, cd_text_driver_index, simulate, toc_path)
        };

        if !toc_path.exists() {
            if let Some(app) = app_weak.upgrade() {
                app.set_burn_error(true);
                app.set_burn_result_message("TOC file not found. Please export the project first.".into());
            }
            return;
        }

        // Validate CD-TEXT if enabled (unless user already confirmed warning)
        if cd_text && !state_clone.borrow().skip_cd_text_validation {
            let validation = {
                let state = state_clone.borrow();
                if let Some(ref project) = state.project {
                    validate_cd_text(&project.album)
                } else {
                    return;
                }
            };

            if validation.has_warnings() {
                // Store pending action and show warning
                state_clone.borrow_mut().pending_warning_action = PendingWarningAction::Burn;
                if let Some(app) = app_weak.upgrade() {
                    app.set_warning_title("CD-TEXT Warning".into());
                    app.set_warning_message(format!(
                        "Some metadata may cause CD-TEXT issues:\n\n{}",
                        validation.format_warnings()
                    ).into());
                    app.set_show_warning_dialog(true);
                }
                return;
            }
        }
        // Reset skip flag
        state_clone.borrow_mut().skip_cd_text_validation = false;

        // Reset burn state
        if let Ok(mut log) = burn_log_clone.lock() {
            log.clear();
        }
        burn_complete_clone.store(false, Ordering::SeqCst);
        burn_error_clone.store(false, Ordering::SeqCst);
        if let Ok(mut msg) = burn_error_msg_clone.lock() {
            msg.clear();
        }

        // Set UI to burning state
        if let Some(app) = app_weak.upgrade() {
            app.set_is_burning(true);
            app.set_burn_progress(0.0);
            app.set_burn_status("Starting burn...".into());
            app.set_burn_log("".into());
        }

        // On macOS, unmount discs before burning
        #[cfg(target_os = "macos")]
        unmount_optical_discs();

        // Clone shared state for thread
        let burn_log_thread = burn_log_clone.clone();
        let burn_complete_thread = burn_complete_clone.clone();
        let burn_error_thread = burn_error_clone.clone();
        let burn_error_msg_thread = burn_error_msg_clone.clone();
        let burn_child_thread = burn_child_clone.clone();

        // Spawn burn thread
        std::thread::spawn(move || {
            // Determine driver arguments based on CD-TEXT settings
            let driver_args: Option<(&str, &str)> = if cd_text {
                match cd_text_driver_index {
                    0 => None,  // Auto - no driver flag
                    1 => Some(("--driver", "generic-mmc-raw")),
                    2 => Some(("--driver", "generic-mmc:0x10")),
                    3 => Some(("--driver", "generic-mmc:0x20000")),
                    _ => None,
                }
            } else {
                None
            };

            let driver_label = match cd_text_driver_index {
                0 => "Auto",
                1 => "Raw Mode",
                2 => "Sub-channel Mode",
                3 => "CUE Sheet Mode",
                _ => "Auto",
            };

            println!("=== Starting CD burn ===");
            println!("TOC file: {}", toc_path.display());
            println!("Device: {}", device);
            println!("Speed: {} (0=auto)", speed);
            println!("CD-TEXT: {}", if cd_text { format!("Enabled ({})", driver_label) } else { "Disabled".to_string() });
            println!("Simulate: {}", simulate);
            println!("========================");

            // Build cdrdao command
            let mut cmd = std::process::Command::new("cdrdao");
            cmd.arg("write");
            cmd.arg("--device").arg(&device);
            if let Some((flag, value)) = driver_args {
                cmd.arg(flag).arg(value);
            }
            if speed > 0 {
                cmd.arg("--speed").arg(speed.to_string());
            }
            if simulate {
                cmd.arg("--simulate");
            }
            if eject {
                cmd.arg("--eject");
            }
            cmd.arg("-v").arg("2");
            cmd.arg(&toc_path);

            // Set up for streaming output
            cmd.stdout(std::process::Stdio::piped());
            cmd.stderr(std::process::Stdio::piped());

            // Spawn child process
            match cmd.spawn() {
                Ok(mut child) => {
                    // Store child process handle for cancel support
                    if let Ok(mut guard) = burn_child_thread.lock() {
                        *guard = Some(std::mem::replace(&mut child, std::process::Command::new("true").spawn().unwrap()));
                        std::mem::swap(&mut child, guard.as_mut().unwrap());
                    }

                    // Read stderr for progress (cdrdao outputs to stderr)
                    let stderr = child.stderr.take();

                    if let Some(stderr) = stderr {
                        use std::io::BufRead;
                        let reader = std::io::BufReader::new(stderr);

                        for line in reader.lines() {
                            if let Ok(line) = line {
                                println!("[cdrdao] {}", line);

                                // Add to log
                                if let Ok(mut log) = burn_log_thread.lock() {
                                    log.push(line.clone());
                                }
                            }
                        }
                    }

                    // Wait for process to complete
                    match child.wait() {
                        Ok(status) => {
                            if status.success() {
                                println!("=== Burn completed successfully ===");
                                burn_complete_thread.store(true, Ordering::SeqCst);
                            } else {
                                println!("=== Burn failed with status: {} ===", status);
                                burn_error_thread.store(true, Ordering::SeqCst);
                                if let Ok(mut msg) = burn_error_msg_thread.lock() {
                                    *msg = format!("Burn process exited with status: {}", status);
                                }
                            }
                        }
                        Err(e) => {
                            println!("=== Failed to wait for burn process: {} ===", e);
                            burn_error_thread.store(true, Ordering::SeqCst);
                            if let Ok(mut msg) = burn_error_msg_thread.lock() {
                                *msg = format!("Failed to wait for burn process: {}", e);
                            }
                        }
                    }
                }
                Err(e) => {
                    println!("=== Failed to start cdrdao: {} ===", e);
                    burn_error_thread.store(true, Ordering::SeqCst);
                    if let Ok(mut msg) = burn_error_msg_thread.lock() {
                        *msg = format!("Failed to start cdrdao: {}", e);
                    }
                }
            }

            // Clear child process handle
            if let Ok(mut guard) = burn_child_thread.lock() {
                *guard = None;
            }
        });
    });

    // on_cancel_burn - Cancels the burn process
    let app_weak = app.as_weak();
    let burn_child_cancel = burn_child_process.clone();
    let burn_error_cancel = burn_error.clone();
    let burn_error_msg_cancel = burn_error_message.clone();
    app.on_cancel_burn(move || {
        // Kill the child process if running
        if let Ok(mut guard) = burn_child_cancel.lock() {
            if let Some(ref mut child) = *guard {
                println!("=== Cancelling burn process ===");
                let _ = child.kill();
            }
        }

        // Set error state (cancelled)
        burn_error_cancel.store(true, Ordering::SeqCst);
        if let Ok(mut msg) = burn_error_msg_cancel.lock() {
            *msg = "Burn cancelled by user.\n\nNote: The disc may be unusable.".to_string();
        }

        if let Some(app) = app_weak.upgrade() {
            app.set_is_burning(false);
            app.set_burn_error(true);
            app.set_burn_result_message("Burn cancelled by user.\n\nNote: The disc may be unusable.".into());
        }
    });

    // on_close_export_dialog - Closes the export dialog and resets state
    let app_weak = app.as_weak();
    app.on_close_export_dialog(move || {
        if let Some(app) = app_weak.upgrade() {
            app.set_show_export_dialog(false);
            app.set_is_exporting(false);
            app.set_export_complete(false);
            app.set_export_error(false);
            app.set_export_status("".into());
            app.set_export_error_message("".into());
            app.set_export_output_dir("".into());
            // Clear the created files list
            let empty_files: Rc<slint::VecModel<slint::SharedString>> = Rc::new(slint::VecModel::from(Vec::new()));
            app.set_export_created_files(empty_files.into());
        }
    });

    // on_close_burn_dialog - Closes the burn dialog and resets state
    let app_weak = app.as_weak();
    let burn_log_close = burn_log.clone();
    let burn_complete_close = burn_complete.clone();
    let burn_error_close = burn_error.clone();
    app.on_close_burn_dialog(move || {
        // Reset burn state
        if let Ok(mut log) = burn_log_close.lock() {
            log.clear();
        }
        burn_complete_close.store(false, Ordering::SeqCst);
        burn_error_close.store(false, Ordering::SeqCst);

        if let Some(app) = app_weak.upgrade() {
            app.set_show_burn_dialog(false);
            app.set_is_burning(false);
            app.set_burn_complete(false);
            app.set_burn_error(false);
            app.set_burn_progress(0.0);
            app.set_burn_status("".into());
            app.set_burn_log("".into());
            app.set_burn_result_message("".into());
            app.set_status_message("Ready".into());
        }
    });

    let state_clone = state.clone();
    let app_weak = app.as_weak();
    app.on_save_project_as(move || {
        let mut state = state_clone.borrow_mut();
        let Some(ref project) = state.project else {
            if let Some(app) = app_weak.upgrade() {
                app.set_status_message("No project to save".into());
            }
            return;
        };

        // Open save dialog
        let dialog = rfd::FileDialog::new()
            .add_filter("Red Book Master Project", &["rbm"])
            .set_title("Save Project As")
            .set_file_name(
                state.project_path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                    .unwrap_or("untitled.rbm")
            );

        if let Some(path) = dialog.save_file() {
            match project.save_to(&path) {
                Ok(()) => {
                    state.project_path = Some(path.clone());
                    if let Some(app) = app_weak.upgrade() {
                        app.set_status_message(format!("Saved: {}", path.display()).into());
                    }
                }
                Err(e) => {
                    if let Some(app) = app_weak.upgrade() {
                        show_error_dialog(&app, "Save Failed", &format!("Failed to save project:\n{}", e));
                    }
                }
            }
        }
    });

    let state_clone = state.clone();
    let app_weak = app.as_weak();
    app.on_remove_track(move |track_num| {
        let removed = {
            let mut state = state_clone.borrow_mut();
            if let Some(ref mut project) = state.project {
                // Find track index by number
                if let Some(idx) = project.album.tracks.iter().position(|t| t.number == track_num as u8) {
                    project.album.tracks.remove(idx);
                    // Renumber remaining tracks
                    for (i, track) in project.album.tracks.iter_mut().enumerate() {
                        track.number = (i + 1) as u8;
                    }
                    // Invalidate waveform cache since track numbers changed
                    state.waveform_cache.clear();
                    state.displayed_track_num = None;

                    state.auto_save();
                    true
                } else {
                    false
                }
            } else {
                false
            }
        };

        if removed {
            // Gather all UI update data while holding the borrow
            let (tracks, new_count, new_track_num) = {
                let state = state_clone.borrow();
                let tracks = state.tracks_to_model();
                let new_count = state.project.as_ref().map(|p| p.album.tracks.len()).unwrap_or(0) as i32;

                // Get track number for the new selection
                let new_track_num = if let Some(app) = app_weak.upgrade() {
                    let current_idx = app.get_selected_track_index();
                    let new_idx = if current_idx >= new_count { new_count - 1 } else { current_idx };

                    if new_idx >= 0 {
                        state.project.as_ref().and_then(|p| {
                            p.album.tracks.get(new_idx as usize).map(|t| t.number as i32)
                        })
                    } else {
                        None
                    }
                } else {
                    None
                };

                (tracks, new_count, new_track_num)
            };

            // Now update UI without holding borrows
            if let Some(app) = app_weak.upgrade() {
                let model = std::rc::Rc::new(slint::VecModel::from(tracks));
                app.set_tracks(model.into());

                if new_count == 0 {
                    app.set_selected_track_index(-1);
                    app.set_current_track_title("".into());
                    app.set_current_track_pregap("0".into());
                    app.set_waveform_peaks(std::rc::Rc::new(slint::VecModel::from(Vec::<WaveformPeak>::new())).into());
                    app.set_waveform_duration("0:00".into());
                } else if let Some(track_num) = new_track_num {
                    // Use invoke_select_track to handle async waveform loading
                    app.invoke_select_track(track_num);
                }
                app.set_status_message(format!("Removed track {}", track_num).into());
            }
        }
    });

    let state_clone = state.clone();
    let app_weak = app.as_weak();
    app.on_update_track_pregap(move |track_num, pregap| {
        let mut state = state_clone.borrow_mut();
        if let Some(ref mut project) = state.project {
            if let Ok(secs) = pregap.parse::<u64>() {
                if let Some(track) = project.album.get_track_mut(track_num as u8) {
                    track.pregap = std::time::Duration::from_secs(secs);
                    if let Some(app) = app_weak.upgrade() {
                        let tracks: Vec<TrackData> = state.tracks_to_model();
                        let model = Rc::new(slint::VecModel::from(tracks));
                        app.set_tracks(model.into());
                    }
                    state.auto_save();
                }
            }
        }
    });

    // Track reordering via drag-and-drop
    let state_clone = state.clone();
    let app_weak = app.as_weak();
    app.on_reorder_tracks(move |from_index, to_index| {
        let from_idx = from_index as usize;
        let to_idx = to_index as usize;

        // Reorder tracks and get data for UI update
        let (tracks, track_num) = {
            let mut state = state_clone.borrow_mut();
            let Some(ref mut project) = state.project else {
                return;
            };

            let track_count = project.album.tracks.len();
            // to_idx can be track_count (meaning "insert at end")
            if from_idx >= track_count || to_idx > track_count || from_idx == to_idx {
                return;
            }

            // Remove track from old position
            let track = project.album.tracks.remove(from_idx);

            // Calculate insert position:
            // - When dragging DOWN (from < to), the target index shifted down by 1 after removal
            // - When dragging UP (from > to), no adjustment needed
            let insert_idx = if from_idx < to_idx {
                to_idx - 1
            } else {
                to_idx
            };

            project.album.tracks.insert(insert_idx, track);

            // Renumber all tracks
            for (i, track) in project.album.tracks.iter_mut().enumerate() {
                track.number = (i + 1) as u8;
            }

            // Get track number for the moved track (now at insert_idx)
            let track_num = project.album.tracks.get(insert_idx).map(|t| t.number as i32);

            // Prepare UI data
            let tracks = state.tracks_to_model();

            // Invalidate waveform cache since track numbers changed
            state.waveform_cache.clear();
            state.displayed_track_num = None;

            state.auto_save();

            (tracks, track_num)
        };

        // Update UI
        if let Some(app) = app_weak.upgrade() {
            let model = Rc::new(slint::VecModel::from(tracks));
            app.set_tracks(model.into());

            // Use invoke_select_track to handle async waveform loading
            if let Some(track_num) = track_num {
                app.invoke_select_track(track_num);
            }

            app.set_status_message("Track reordered".into());
        }
    });

    // Cancel transcoding dialog
    let app_weak = app.as_weak();
    let state_clone = state.clone();
    app.on_cancel_transcode(move || {
        let mut state = state_clone.borrow_mut();
        state.pending_transcode_files.clear();

        if let Some(app) = app_weak.upgrade() {
            app.set_show_transcode_dialog(false);
            app.set_status_message("Conversion cancelled".into());
        }
    });

    // Dismiss error dialog
    let app_weak = app.as_weak();
    app.on_dismiss_error(move || {
        if let Some(app) = app_weak.upgrade() {
            app.set_show_error_dialog(false);
        }
    });

    // Warning dialog - user chose to proceed anyway
    let state_clone = state.clone();
    let app_weak = app.as_weak();
    app.on_warning_proceed(move || {
        let pending_action = {
            let mut state = state_clone.borrow_mut();
            state.skip_cd_text_validation = true;
            std::mem::replace(&mut state.pending_warning_action, PendingWarningAction::None)
        };

        if let Some(app) = app_weak.upgrade() {
            app.set_show_warning_dialog(false);

            match pending_action {
                PendingWarningAction::Export => {
                    // Invoke export callback again (validation will be skipped)
                    app.invoke_export_master();
                }
                PendingWarningAction::Burn => {
                    // Invoke burn callback again (for future burn validation)
                    app.invoke_start_burn();
                }
                PendingWarningAction::None => {}
            }
        }
    });

    // Warning dialog - user chose to cancel
    let state_clone = state.clone();
    let app_weak = app.as_weak();
    app.on_warning_cancel(move || {
        {
            let mut state = state_clone.borrow_mut();
            state.pending_warning_action = PendingWarningAction::None;
            state.skip_cd_text_validation = false;
        }

        if let Some(app) = app_weak.upgrade() {
            app.set_show_warning_dialog(false);
            app.set_status_message("Operation cancelled".into());
        }
    });

    // Shared container for completed conversions (thread-safe)
    let completed_conversions: Arc<std::sync::Mutex<Vec<(PathBuf, std::time::Duration)>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let conversion_done: Arc<std::sync::atomic::AtomicBool> =
        Arc::new(std::sync::atomic::AtomicBool::new(false));

    // Confirm transcoding - run conversion in background thread
    let app_weak = app.as_weak();
    let state_clone = state.clone();
    let completed_clone = completed_conversions.clone();
    let done_flag = conversion_done.clone();
    app.on_confirm_transcode(move || {
        let mut state = state_clone.borrow_mut();
        let pending_files = std::mem::take(&mut state.pending_transcode_files);

        if pending_files.is_empty() {
            return;
        }

        // Get project_dir for transcoded output
        let project_dir = match state.project.as_ref().and_then(|p| p.project_dir.as_ref()) {
            Some(dir) => dir.clone(),
            None => {
                if let Some(app) = app_weak.upgrade() {
                    app.set_status_message("No project directory - create a project first".into());
                    app.set_show_transcode_dialog(false);
                }
                return;
            }
        };

        // Get current track count for numbering
        let starting_track_num = state.project.as_ref()
            .map(|p| p.album.track_count())
            .unwrap_or(0);

        if let Some(app) = app_weak.upgrade() {
            app.set_is_transcoding(true);
            app.set_transcode_progress(0.0);
        }

        // Clone what we need for the thread
        let app_weak_clone = app_weak.clone();
        let completed_for_thread = completed_clone.clone();
        let done_for_thread = done_flag.clone();

        // Spawn background thread for transcoding
        std::thread::spawn(move || {
            let total = pending_files.len();
            // Store: (relative_path, title, duration)
            let mut converted_paths: Vec<(PathBuf, std::time::Duration)> = Vec::new();

            // Create _transcoded directory in project
            let transcoded_dir = project_dir.join("_transcoded");
            if !transcoded_dir.exists() {
                if let Err(e) = std::fs::create_dir_all(&transcoded_dir) {
                    eprintln!("Failed to create _transcoded directory: {}", e);
                    done_for_thread.store(true, std::sync::atomic::Ordering::SeqCst);
                    return;
                }
            }

            for (i, (input_path, _info)) in pending_files.into_iter().enumerate() {
                // Update progress in UI
                let title = input_path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("file")
                    .to_string();

                let progress = i as f32 / total as f32;
                let status = format!("Converting {}...", title);

                // Update UI from main thread
                let app_weak_status = app_weak_clone.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(app) = app_weak_status.upgrade() {
                        app.set_transcode_progress(progress);
                        app.set_transcode_status(status.into());
                    }
                });

                // Create numbered output filename
                let track_num = starting_track_num + i + 1;
                let sanitized = title.chars()
                    .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' { c } else { '_' })
                    .collect::<String>()
                    .trim()
                    .replace(' ', "_")
                    .to_lowercase();
                let output_filename = format!("{:02}_{}.wav", track_num, sanitized);
                let output_path = transcoded_dir.join(&output_filename);

                // Perform conversion
                match convert_to_red_book(&input_path, &output_path) {
                    Ok(_) => {
                        // Read the converted file to get duration
                        if let Ok(info) = read_wav_info(&output_path) {
                            // Store RELATIVE path (relative to project_dir)
                            let relative_path = PathBuf::from("_transcoded").join(&output_filename);
                            converted_paths.push((relative_path, info.duration));
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to convert {:?}: {}", input_path, e);
                    }
                }
            }

            // Store converted paths and signal completion
            if let Ok(mut completed) = completed_for_thread.lock() {
                *completed = converted_paths;
            }
            done_for_thread.store(true, std::sync::atomic::Ordering::SeqCst);
        });
    });

    // Set up a timer to poll for position updates from the audio engine
    let app_weak = app.as_weak();
    let engine_for_timer = audio_engine.clone();
    let state_for_timer = state.clone();
    let completed_for_timer = completed_conversions.clone();
    let done_for_timer = conversion_done.clone();
    let waveform_result_for_timer = waveform_result.clone();
    // Burn state for timer polling
    let burn_log_timer = burn_log.clone();
    let burn_complete_timer = burn_complete.clone();
    let burn_error_timer = burn_error.clone();
    let burn_error_msg_timer = burn_error_message.clone();
    // Counter for periodic window size check (every ~2 seconds = 40 ticks at 50ms)
    let window_check_counter = Rc::new(std::cell::Cell::new(0u32));
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(50),
        move || {
            // Check for completed waveform loading
            if let Ok(mut result) = waveform_result_for_timer.try_lock() {
                if let Some((track_num, waveform_data, duration_str)) = result.take() {
                    if let Some(app) = app_weak.upgrade() {
                        let mut state = state_for_timer.borrow_mut();

                        // Store in cache
                        state.insert_waveform(track_num, waveform_data, duration_str.clone());

                        // Update UI only if this is still the displayed track
                        if state.displayed_track_num == Some(track_num) {
                            let peaks = state.get_visible_peaks();
                            drop(state); // Release borrow before UI updates

                            let model = Rc::new(slint::VecModel::from(peaks));
                            app.set_waveform_peaks(model.into());
                            app.set_waveform_duration(duration_str.into());
                            app.set_waveform_loading(false);
                            app.set_status_message(format!("Track {} loaded", track_num).into());
                        }
                    }
                }
            }

            // Check for completed transcoding
            if done_for_timer.load(std::sync::atomic::Ordering::SeqCst) {
                done_for_timer.store(false, std::sync::atomic::Ordering::SeqCst);

                let converted_paths = {
                    let mut completed = completed_for_timer.lock().unwrap();
                    std::mem::take(&mut *completed)
                };

                if !converted_paths.is_empty() {
                    if let Some(app) = app_weak.upgrade() {
                        let mut state = state_for_timer.borrow_mut();

                        if let Some(project) = state.project.as_mut() {
                            let mut added = 0;

                            for (relative_path, duration) in converted_paths {
                                // Extract title from filename (relative_path is like "_transcoded/01_song.wav")
                                let title = relative_path.file_stem()
                                    .and_then(|s| s.to_str())
                                    .unwrap_or("Unknown")
                                    .to_string();

                                // Remove track number prefix from title if present (e.g., "01_song" -> "song")
                                let title = if title.len() > 3 && title.chars().take(2).all(|c| c.is_ascii_digit()) && title.chars().nth(2) == Some('_') {
                                    title[3..].to_string()
                                } else {
                                    title
                                };

                                let track = Track::new(
                                    (project.album.track_count() + 1) as u8,
                                    title,
                                    relative_path,  // Already relative path
                                    duration,
                                );
                                project.album.add_track(track);
                                added += 1;
                            }

                            state.auto_save();

                            // Update UI
                            let tracks: Vec<TrackData> = state.tracks_to_model();
                            let model = Rc::new(slint::VecModel::from(tracks));
                            app.set_tracks(model.into());
                            app.set_show_transcode_dialog(false);
                            app.set_is_transcoding(false);
                            app.set_status_message(format!("Converted and added {} track(s)", added).into());
                        }
                    }
                }
            }

            // Check for burn progress updates
            if let Some(app) = app_weak.upgrade() {
                if app.get_is_burning() {
                    // Update log from burn thread
                    if let Ok(log) = burn_log_timer.try_lock() {
                        if !log.is_empty() {
                            let log_text = log.join("\n");
                            app.set_burn_log(log_text.into());

                            // Parse progress from cdrdao output
                            // Look for "Wrote X of Y MB" for overall progress
                            // Look for "Writing track XX" for status
                            let mut found_progress = false;

                            for line in log.iter().rev() {
                                // Check for MB progress: "Wrote 375 of 375 MB"
                                if !found_progress && line.contains(" of ") && line.contains(" MB") {
                                    // Try to parse "Wrote X of Y MB" or "X of Y MB"
                                    if let Some(mb_part) = line.split(" MB").next() {
                                        let parts: Vec<&str> = mb_part.split(" of ").collect();
                                        if parts.len() == 2 {
                                            // Get the last number before "of" (current MB)
                                            let current_str = parts[0].split_whitespace().last().unwrap_or("0");
                                            let total_str = parts[1].trim();
                                            if let (Ok(current), Ok(total)) = (current_str.parse::<f32>(), total_str.parse::<f32>()) {
                                                if total > 0.0 {
                                                    let progress = (current / total).min(1.0);
                                                    app.set_burn_progress(progress);
                                                    found_progress = true;
                                                }
                                            }
                                        }
                                    }
                                }

                                // Check for track status: "Writing track 01 (mode..."
                                if line.contains("Writing track") {
                                    if let Some(track_part) = line.split("Writing track ").nth(1) {
                                        // Extract track number (e.g., "01" from "01 (mode AUDIO...")
                                        let track_num = track_part.split_whitespace().next().unwrap_or("?");
                                        app.set_burn_status(format!("Writing track {}...", track_num).into());
                                    }
                                    if found_progress { break; }
                                } else if line.contains("CD-TEXT lead-in") || line.contains("Writing lead-in") {
                                    app.set_burn_status("Writing lead-in...".into());
                                    if !found_progress { app.set_burn_progress(0.02); }
                                    break;
                                } else if line.contains("Flushing cache") {
                                    app.set_burn_status("Flushing cache...".into());
                                    if !found_progress { app.set_burn_progress(0.98); }
                                    break;
                                } else if line.contains("Starting write") {
                                    app.set_burn_status("Starting write...".into());
                                    if !found_progress { app.set_burn_progress(0.01); }
                                    break;
                                }
                            }
                        }
                    }

                    // Check for completion
                    if burn_complete_timer.load(Ordering::SeqCst) {
                        burn_complete_timer.store(false, Ordering::SeqCst);
                        app.set_is_burning(false);
                        app.set_burn_complete(true);
                        app.set_burn_progress(1.0);
                        app.set_burn_result_message("Successfully burned CD!".into());
                        app.set_status_message("CD burned successfully!".into());
                    }

                    // Check for error
                    if burn_error_timer.load(Ordering::SeqCst) {
                        burn_error_timer.store(false, Ordering::SeqCst);
                        app.set_is_burning(false);
                        app.set_burn_error(true);

                        let error_msg = if let Ok(msg) = burn_error_msg_timer.lock() {
                            if msg.is_empty() {
                                "Burn failed. Check console for details.".to_string()
                            } else {
                                msg.clone()
                            }
                        } else {
                            "Burn failed. Check console for details.".to_string()
                        };

                        app.set_burn_result_message(error_msg.into());
                        app.set_status_message("CD burn failed".into());
                    }
                }
            }

            // Process events from audio engine
            while let Some(event) = engine_for_timer.try_recv_event() {
                let Some(app) = app_weak.upgrade() else { return };

                match event {
                    PlayerEvent::Loaded { duration_ms } => {
                        // Reset to stopped state when a new track is loaded
                        let mut playback = app.get_playback();
                        playback.duration = duration_ms as f32 / 1000.0;
                        playback.position = 0.0;
                        playback.is_playing = false;
                        playback.is_paused = false;
                        app.set_playback(playback);
                        app.set_playhead_position(0.0);
                    }
                    PlayerEvent::Playing => {
                        let mut playback = app.get_playback();
                        playback.is_playing = true;
                        playback.is_paused = false;
                        app.set_playback(playback);
                    }
                    PlayerEvent::Paused => {
                        let mut playback = app.get_playback();
                        playback.is_paused = true;
                        app.set_playback(playback);
                    }
                    PlayerEvent::Stopped => {
                        // User-initiated stop
                        let mut playback = app.get_playback();
                        playback.is_playing = false;
                        playback.is_paused = false;
                        playback.position = 0.0;
                        app.set_playback(playback);
                        app.set_playhead_position(0.0);
                    }
                    PlayerEvent::TrackFinished => {
                        // Track finished naturally - try to play next track
                        let tracks = app.get_tracks();
                        let current_index = app.get_selected_track_index();
                        let next_index = current_index + 1;

                        if next_index < tracks.row_count() as i32 {
                            // There's a next track - select and play it
                            if let Some(next_track) = tracks.row_data(next_index as usize) {
                                let track_num = next_track.number;

                                // Use invoke_select_track which handles async waveform loading
                                // The select_track callback will see was_playing=true because
                                // we'll still be in "playing" state until after this
                                app.invoke_select_track(track_num);

                                // Start playing after the track is loaded
                                engine_for_timer.play();
                            }
                        } else {
                            // This was the last track - stop playback
                            let mut playback = app.get_playback();
                            playback.is_playing = false;
                            playback.is_paused = false;
                            playback.position = 0.0;
                            app.set_playback(playback);
                            app.set_playhead_position(0.0);
                            app.set_status_message("Playback finished".into());
                        }
                    }
                    PlayerEvent::Position(pos_ms) => {
                        let playback = app.get_playback();
                        let duration_ms = (playback.duration * 1000.0) as u64;
                        if duration_ms > 0 {
                            let position_ratio = pos_ms as f32 / duration_ms as f32;
                            app.set_playhead_position(position_ratio);

                            // Update playback position
                            let mut playback = playback;
                            playback.position = pos_ms as f32 / 1000.0;
                            app.set_playback(playback);
                        }
                    }
                    PlayerEvent::Error(msg) => {
                        app.set_status_message(format!("Error: {}", msg).into());
                    }
                }
            }

            // Periodic window size check (every ~2 seconds)
            let count = window_check_counter.get() + 1;
            window_check_counter.set(count);
            if count % 40 == 0 {
                if let Some(app) = app_weak.upgrade() {
                    let size = app.window().size();
                    let scale = app.window().scale_factor();
                    let logical_width = (size.width as f32 / scale) as u32;
                    let logical_height = (size.height as f32 / scale) as u32;

                    let mut state = state_for_timer.borrow_mut();
                    let current_size = (logical_width, logical_height);
                    let size_changed = state.last_saved_window_size != Some(current_size);

                    if size_changed {
                        state.preferences.window_width = Some(logical_width);
                        state.preferences.window_height = Some(logical_height);
                        state.save_preferences();
                    }
                }
            }
        },
    );

    // Keep timer alive by moving it into a variable that lives until app.run() completes
    let _timer = timer;

    app.run()
}

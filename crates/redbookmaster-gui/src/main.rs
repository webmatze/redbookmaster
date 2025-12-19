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
use redbookmaster_lib::{generate_cue, generate_toc};
use redbookmaster_lib::{Cdrdao, BurnOptions, cdrdao_available, list_drives};
use slint::Model;

use player::{AudioEngine, PlayerEvent};

slint::include_modules!();

/// Number of waveform peaks to display
const WAVEFORM_BINS: usize = 500;

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
}

/// Cached waveform data
struct WaveformCache {
    /// Full waveform data for zooming
    waveform_data: WaveformData,
    duration_str: String,
}

impl AppState {
    fn new() -> Self {
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
        }
    }

    fn new_project(&mut self) {
        let album = Album::new("Untitled Album".to_string(), "".to_string());
        let project = Project::new(album);
        self.project = Some(project);
        self.project_path = None;
        self.waveform_cache.clear();
        self.displayed_track_num = None;
        self.current_track_path = None;
        self.current_track_num = None;
        self.zoom_level = 1.0;
        self.scroll_offset = 0.0;
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
        let mut state = state_clone.borrow_mut();
        state.new_project();

        if let Some(app) = app_weak.upgrade() {
            let tracks: Vec<TrackData> = state.tracks_to_model();
            let model = Rc::new(slint::VecModel::from(tracks));
            app.set_tracks(model.into());
            app.set_album(state.album_to_model());
            app.set_status_message("New project created".into());
            app.set_selected_track_index(-1);
            // Clear waveform
            app.set_waveform_peaks(Rc::new(slint::VecModel::from(Vec::<WaveformPeak>::new())).into());
            app.set_waveform_duration("0:00".into());
        }
    });

    let app_weak = app.as_weak();
    let state_clone = state.clone();
    app.on_open_project(move || {
        let dialog = rfd::FileDialog::new()
            .add_filter("Red Book Master Project", &["rbm"])
            .set_title("Open Project");

        if let Some(path) = dialog.pick_file() {
            match Project::load(&path) {
                Ok(project) => {
                    let mut state = state_clone.borrow_mut();
                    state.project = Some(project);
                    state.project_path = Some(path.clone());
                    state.waveform_cache.clear();
                    state.displayed_track_num = None;

                    if let Some(app) = app_weak.upgrade() {
                        let tracks: Vec<TrackData> = state.tracks_to_model();
                        let model = Rc::new(slint::VecModel::from(tracks));
                        app.set_tracks(model.into());
                        app.set_album(state.album_to_model());
                        app.set_status_message(format!("Opened: {}", path.display()).into());
                        app.set_selected_track_index(-1);
                        // Clear waveform
                        app.set_waveform_peaks(Rc::new(slint::VecModel::from(Vec::<WaveformPeak>::new())).into());
                        app.set_waveform_duration("0:00".into());
                    }
                }
                Err(e) => {
                    eprintln!("Failed to open project: {}", e);
                    if let Some(app) = app_weak.upgrade() {
                        app.set_status_message(format!("Error: {}", e).into());
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
        let dialog = rfd::FileDialog::new()
            .add_filter("WAV Audio", &["wav", "WAV"])
            .set_title("Select WAV files to add");

        if let Some(files) = dialog.pick_files() {
            let mut state = state_clone.borrow_mut();

            // Create a new project if none exists
            if state.project.is_none() {
                state.new_project();
            }

            let project = state.project.as_mut().unwrap();
            let mut added = 0;
            let mut needs_transcoding: Vec<(PathBuf, WavInfo)> = Vec::new();

            for path in files {
                match read_wav_info(&path) {
                    Ok(info) => {
                        if info.is_red_book_compliant() {
                            // Add compliant files immediately
                            let title = path.file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or("Unknown")
                                .to_string();

                            let track = Track::new(
                                (project.album.track_count() + 1) as u8,
                                title,
                                path,
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
            let path = track.source_file.clone();
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
        let state = state_clone.borrow();
        let Some(ref project) = state.project else {
            if let Some(app) = app_weak.upgrade() {
                app.set_status_message("No project to export".into());
            }
            return;
        };

        if project.album.tracks.is_empty() {
            if let Some(app) = app_weak.upgrade() {
                app.set_status_message("No tracks to export".into());
            }
            return;
        }

        // Validate album
        if let Err(e) = project.album.validate() {
            if let Some(app) = app_weak.upgrade() {
                app.set_status_message(format!("Validation failed: {}", e).into());
            }
            return;
        }

        // Open folder selection dialog
        let dialog = rfd::FileDialog::new()
            .set_title("Select Export Directory");

        let Some(output_dir) = dialog.pick_folder() else {
            return;
        };

        if let Some(app) = app_weak.upgrade() {
            app.set_status_message("Exporting...".into());
        }

        // Generate filenames based on album title
        let base_name = project.album.title.replace(|c: char| !c.is_alphanumeric() && c != ' ', "")
            .replace(' ', "_")
            .to_lowercase();
        let base_name = if base_name.is_empty() { "master".to_string() } else { base_name };

        let wav_path = output_dir.join(format!("{}.wav", base_name));
        let cue_path = output_dir.join(format!("{}.cue", base_name));
        let toc_path = output_dir.join(format!("{}.toc", base_name));

        // Step 1: Concatenate tracks into master WAV
        if let Some(app) = app_weak.upgrade() {
            app.set_status_message("Creating master WAV file...".into());
        }

        if let Err(e) = concatenate_tracks(&project.album.tracks, &wav_path) {
            if let Some(app) = app_weak.upgrade() {
                app.set_status_message(format!("Export failed: {}", e).into());
            }
            return;
        }

        // Step 2: Generate CUE sheet
        if let Some(app) = app_weak.upgrade() {
            app.set_status_message("Generating CUE sheet...".into());
        }

        let wav_filename = wav_path.file_name().unwrap().to_str().unwrap();
        if let Err(e) = generate_cue(&project.album, wav_filename, &cue_path) {
            if let Some(app) = app_weak.upgrade() {
                app.set_status_message(format!("CUE generation failed: {}", e).into());
            }
            return;
        }

        // Step 3: Generate TOC file for cdrdao
        if let Some(app) = app_weak.upgrade() {
            app.set_status_message("Generating TOC file...".into());
        }

        if let Err(e) = generate_toc(&project.album, wav_filename, &toc_path) {
            if let Some(app) = app_weak.upgrade() {
                app.set_status_message(format!("TOC generation failed: {}", e).into());
            }
            return;
        }

        // Success!
        if let Some(app) = app_weak.upgrade() {
            app.set_status_message(format!("Exported to {}", output_dir.display()).into());
        }
    });

    let state_clone = state.clone();
    let app_weak = app.as_weak();
    app.on_burn_cd(move || {
        // Check if cdrdao is available
        if !cdrdao_available() {
            if let Some(app) = app_weak.upgrade() {
                app.set_status_message("cdrdao not installed. Install it with: brew install cdrdao".into());
            }
            return;
        }

        let state = state_clone.borrow();
        let Some(ref project) = state.project else {
            if let Some(app) = app_weak.upgrade() {
                app.set_status_message("No project loaded".into());
            }
            return;
        };

        if project.album.tracks.is_empty() {
            if let Some(app) = app_weak.upgrade() {
                app.set_status_message("No tracks to burn".into());
            }
            return;
        }

        // On macOS, unmount any mounted optical discs first
        #[cfg(target_os = "macos")]
        {
            if let Some(app) = app_weak.upgrade() {
                app.set_status_message("Unmounting disc...".into());
            }
            // Find and unmount optical discs using diskutil
            if let Ok(output) = std::process::Command::new("diskutil").args(["list"]).output() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    // Look for external/optical drives (typically disk1, disk2, etc.)
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

        // List available CD drives
        let mut drives = list_drives().unwrap_or_default();

        // On macOS, if no drives found via scanbus, try IORegistry detection
        #[cfg(target_os = "macos")]
        if drives.is_empty() {
            if let Some(app) = app_weak.upgrade() {
                app.set_status_message("Detecting CD drive via IORegistry...".into());
            }
            // Try to detect via ioreg
            if let Ok(output) = std::process::Command::new("ioreg")
                .args(["-c", "IOCDBlockStorageDevice", "-r", "-l"])
                .output()
            {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if stdout.contains("IOCDBlockStorageDevice") {
                    // Found an optical drive - use IOKit device path
                    drives.push(redbookmaster_lib::CdDrive {
                        device: "IOCompactDiscServices".to_string(),
                        vendor: "Apple".to_string(),
                        model: "Optical Drive".to_string(),
                    });
                }
            }
        }

        if drives.is_empty() {
            if let Some(app) = app_weak.upgrade() {
                app.set_status_message("No CD drives found. Please close any disc dialogs and try again.".into());
            }
            return;
        }

        // Ask user to select TOC file
        let dialog = rfd::FileDialog::new()
            .set_title("Select TOC File to Burn")
            .add_filter("TOC files", &["toc"]);

        let Some(toc_path) = dialog.pick_file() else {
            return;
        };

        if !toc_path.exists() {
            if let Some(app) = app_weak.upgrade() {
                app.set_status_message("TOC file not found. Please export first.".into());
            }
            return;
        }

        // Use first available drive
        let drive = &drives[0];

        // Ask about CD-TEXT mode
        let cdtext_confirm = rfd::MessageDialog::new()
            .set_title("CD-TEXT Mode")
            .set_description("Enable CD-TEXT?\n\nCD-TEXT embeds track/album titles on the disc, but uses a driver mode that can fail on some drives.\n\nIf burning fails, try again with CD-TEXT disabled.")
            .set_buttons(rfd::MessageButtons::YesNo)
            .show();

        let use_cdtext = cdtext_confirm == rfd::MessageDialogResult::Yes;

        // Confirm burn
        let confirm = rfd::MessageDialog::new()
            .set_title("Burn CD")
            .set_description(&format!(
                "Burn to {} {}?\n\nDevice: {}\nCD-TEXT: {}\n\nMake sure:\n• A blank CD-R is inserted\n• Any system disc dialogs are closed\n\nClick OK to start burning.",
                drive.vendor, drive.model, drive.device,
                if use_cdtext { "Enabled" } else { "Disabled" }
            ))
            .set_buttons(rfd::MessageButtons::OkCancel)
            .show();

        if confirm != rfd::MessageDialogResult::Ok {
            if let Some(app) = app_weak.upgrade() {
                app.set_status_message("Burn cancelled".into());
            }
            return;
        }

        // Unmount again right before burning (in case macOS re-mounted)
        #[cfg(target_os = "macos")]
        {
            if let Ok(output) = std::process::Command::new("diskutil").args(["list"]).output() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
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

        if let Some(app) = app_weak.upgrade() {
            app.set_status_message("Burning CD... (this may take several minutes)".into());
        }

        // Create burn options
        let burn_options = BurnOptions {
            device: drive.device.clone(),
            speed: 0, // Auto
            simulate: false,
            eject: true,
            force_raw_driver: use_cdtext, // Only use raw driver for CD-TEXT
        };

        let burner = Cdrdao::new(burn_options.clone());

        println!("=== Starting CD burn ===");
        println!("TOC file: {}", toc_path.display());
        println!("Device: {}", burn_options.device);
        println!("Speed: {} (0=auto)", burn_options.speed);
        println!("CD-TEXT: {}", if burn_options.force_raw_driver { "Enabled (raw driver)" } else { "Disabled" });
        println!("========================");
        println!("TIP: If burn fails, try running with sudo:");
        println!("  sudo cargo run -p redbookmaster-gui");
        println!("========================");

        // Burn!
        match burner.burn(&toc_path) {
            Ok(()) => {
                println!("=== Burn completed successfully ===");
                if let Some(app) = app_weak.upgrade() {
                    app.set_status_message("CD burned successfully!".into());
                }
            }
            Err(e) => {
                let error_str = e.to_string();
                println!("=== Burn failed ===");
                println!("{}", error_str);
                println!();
                println!("SUGGESTIONS:");
                if error_str.contains("Write data failed") {
                    println!("  1. Try burning with CD-TEXT disabled");
                    println!("  2. Try running with sudo: sudo cargo run -p redbookmaster-gui");
                    println!("  3. Try a different CD-R disc");
                    println!("  4. Try a slower burn speed");
                }
                if error_str.contains("Device already in use") || error_str.contains("Cannot grab") {
                    println!("  - Close any Finder windows showing the disc");
                    println!("  - Run: diskutil unmountDisk /dev/disk2 (or similar)");
                }
                println!("===================");

                if let Some(app) = app_weak.upgrade() {
                    if error_str.contains("Device already in use") || error_str.contains("Cannot grab") {
                        app.set_status_message("Drive is busy. Close any disc dialogs and try again.".into());
                    } else if error_str.contains("Write data failed") {
                        app.set_status_message("Write failed. Try with CD-TEXT disabled or run with sudo. See console.".into());
                    } else {
                        // Show first 150 chars of error in status bar
                        let short_error = if error_str.len() > 150 {
                            format!("{}... (see console)", &error_str[..150])
                        } else {
                            error_str
                        };
                        app.set_status_message(format!("Burn failed: {}", short_error).into());
                    }
                }
            }
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
                        app.set_status_message(format!("Save failed: {}", e).into());
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
            let mut converted_paths: Vec<(PathBuf, std::time::Duration)> = Vec::new();

            for (i, (input_path, _info)) in pending_files.into_iter().enumerate() {
                // Update progress in UI
                let filename = input_path.file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("file")
                    .to_string();

                let progress = i as f32 / total as f32;
                let status = format!("Converting {}...", filename);

                // Update UI from main thread
                let app_weak_status = app_weak_clone.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(app) = app_weak_status.upgrade() {
                        app.set_transcode_progress(progress);
                        app.set_transcode_status(status.into());
                    }
                });

                // Create output path in _converted subdirectory
                let parent = input_path.parent().unwrap_or(std::path::Path::new("."));
                let converted_dir = parent.join("_converted");
                if !converted_dir.exists() {
                    if let Err(e) = std::fs::create_dir_all(&converted_dir) {
                        eprintln!("Failed to create _converted directory: {}", e);
                        continue;
                    }
                }

                let output_filename = input_path.file_name().unwrap_or_default();
                let output_path = converted_dir.join(output_filename);

                // Perform conversion
                match convert_to_red_book(&input_path, &output_path) {
                    Ok(_) => {
                        // Read the converted file to get duration
                        if let Ok(info) = read_wav_info(&output_path) {
                            converted_paths.push((output_path, info.duration));
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

                        // Ensure project exists
                        if state.project.is_none() {
                            state.new_project();
                        }

                        let project = state.project.as_mut().unwrap();
                        let mut added = 0;

                        for (path, duration) in converted_paths {
                            let title = path.file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or("Unknown")
                                .to_string();

                            let track = Track::new(
                                (project.album.track_count() + 1) as u8,
                                title,
                                path,
                                duration,
                            );
                            project.album.add_track(track);
                            added += 1;
                        }

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
        },
    );

    // Keep timer alive by moving it into a variable that lives until app.run() completes
    let _timer = timer;

    app.run()
}

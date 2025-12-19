// Red Book Master GUI
// Professional CD mastering application with Slint UI

mod player;

use std::rc::Rc;
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;

use redbookmaster_lib::{Album, Project, Track, extract_peaks, WaveformData};
use redbookmaster_lib::core::track::format_duration_ms;
use redbookmaster_lib::audio::concat::concatenate_tracks;
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
    current_waveform: Option<WaveformCache>,
    current_track_path: Option<PathBuf>,
    current_track_num: Option<u8>,
    /// Current zoom level (1.0 = full view, 2.0 = 2x zoom, etc.)
    zoom_level: f32,
    /// Scroll offset for zoomed view (0.0 to 1.0 - zoom_range)
    scroll_offset: f32,
}

/// Cached waveform data
struct WaveformCache {
    track_number: u8,
    /// Full waveform data for zooming
    waveform_data: WaveformData,
    duration_str: String,
}

impl AppState {
    fn new() -> Self {
        Self {
            project: None,
            project_path: None,
            current_waveform: None,
            current_track_path: None,
            current_track_num: None,
            zoom_level: 1.0,
            scroll_offset: 0.0,
        }
    }

    fn new_project(&mut self) {
        let album = Album::new("Untitled Album".to_string(), "".to_string());
        let project = Project::new(album);
        self.project = Some(project);
        self.project_path = None;
        self.current_waveform = None;
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
    fn extract_waveform(&mut self, track_num: u8) -> Option<&WaveformCache> {
        // Check if we already have this waveform cached
        if let Some(ref cache) = self.current_waveform {
            if cache.track_number == track_num {
                return self.current_waveform.as_ref();
            }
        }

        // Get the track
        let track = self.get_track(track_num)?;
        let path = &track.source_file;
        let duration = track.duration;

        // Extract more peaks than needed to allow zooming (16x max zoom)
        const FULL_PEAKS: usize = WAVEFORM_BINS * 16;
        match extract_peaks(path, FULL_PEAKS) {
            Ok(waveform_data) => {
                // Reset zoom when loading new track
                self.zoom_level = 1.0;
                self.scroll_offset = 0.0;

                self.current_waveform = Some(WaveformCache {
                    track_number: track_num,
                    waveform_data,
                    duration_str: format_duration_ms(duration),
                });

                self.current_waveform.as_ref()
            }
            Err(e) => {
                eprintln!("Failed to extract waveform: {}", e);
                None
            }
        }
    }

    /// Get peaks for current zoom level and scroll offset
    fn get_visible_peaks(&self) -> Vec<WaveformPeak> {
        let Some(ref cache) = self.current_waveform else {
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
                    state.current_waveform = None;

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

            for path in files {
                match redbookmaster_lib::read_wav_info(&path) {
                    Ok(info) => {
                        if info.is_red_book_compliant() {
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
                            eprintln!("File not Red Book compliant: {:?}", path);
                            // TODO: Show conversion dialog
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to read WAV: {}", e);
                    }
                }
            }

            if let Some(app) = app_weak.upgrade() {
                let tracks: Vec<TrackData> = state.tracks_to_model();
                let model = Rc::new(slint::VecModel::from(tracks));
                app.set_tracks(model.into());
                app.set_album(state.album_to_model());
                app.set_status_message(format!("Added {} track(s)", added).into());
            }
        }
    });

    // Track selection with waveform extraction
    let app_weak = app.as_weak();
    let state_clone = state.clone();
    let engine_clone = audio_engine.clone();
    app.on_select_track(move |track_num| {
        let mut state = state_clone.borrow_mut();

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

            // Show loading state
            app.set_waveform_loading(true);
            app.set_status_message(format!("Loading waveform for track {}...", track_num).into());
        }

        // Get track path for audio playback
        if let Some(track) = state.get_track(track_num as u8) {
            let path = track.source_file.clone();
            state.current_track_path = Some(path.clone());
            state.current_track_num = Some(track_num as u8);

            // Load track into audio engine
            engine_clone.load(path);
        }

        // Extract waveform
        if state.extract_waveform(track_num as u8).is_some() {
            let peaks = state.get_visible_peaks();
            let duration = state.current_waveform.as_ref().map(|c| c.duration_str.clone()).unwrap_or_default();

            if let Some(app) = app_weak.upgrade() {
                let model = Rc::new(slint::VecModel::from(peaks));
                app.set_waveform_peaks(model.into());
                app.set_waveform_duration(duration.into());
                app.set_waveform_loading(false);
                app.set_status_message(format!("Track {} selected", track_num).into());

                // Reset playhead position
                app.set_playhead_position(0.0);
            }
        } else {
            if let Some(app) = app_weak.upgrade() {
                app.set_waveform_loading(false);
                app.set_status_message("Failed to load waveform".into());
            }
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
    let state_clone = state.clone();
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
                // Select the first track
                if let Some(first_track) = tracks.row_data(0) {
                    let track_num = first_track.number as u8;

                    // Update metadata editor fields
                    app.set_current_track_title(first_track.title.clone());
                    app.set_current_track_pregap(first_track.pregap.clone());

                    // Get track path and load into engine
                    let mut state = state_clone.borrow_mut();
                    if let Some(track) = state.get_track(track_num) {
                        let path = track.source_file.clone();
                        state.current_track_path = Some(path.clone());
                        state.current_track_num = Some(track_num);
                        engine_clone.load(path);
                    }

                    // Extract waveform
                    if state.extract_waveform(track_num).is_some() {
                        let peaks = state.get_visible_peaks();
                        let duration = state.current_waveform.as_ref().map(|c| c.duration_str.clone()).unwrap_or_default();

                        let model = Rc::new(slint::VecModel::from(peaks));
                        app.set_waveform_peaks(model.into());
                        app.set_waveform_duration(duration.into());
                    }

                    app.set_selected_track_index(0);
                    app.set_playhead_position(0.0);
                    drop(state);

                    // Small delay to allow track to load before playing
                    // The audio engine will handle the play command once loaded
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

    let engine_clone = audio_engine.clone();
    app.on_seek(move |pos| {
        engine_clone.seek(pos);
    });

    let engine_clone = audio_engine.clone();
    app.on_set_volume(move |vol| {
        engine_clone.set_volume(vol);
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
        if state.current_waveform.is_none() {
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
        if state.current_waveform.is_none() {
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
        if state.current_waveform.is_none() || state.zoom_level <= 1.0 {
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
                    state.current_waveform = None;
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
            let (tracks, new_count, track_data, waveform_data) = {
                let mut state = state_clone.borrow_mut();
                let tracks = state.tracks_to_model();
                let new_count = state.project.as_ref().map(|p| p.album.tracks.len()).unwrap_or(0) as i32;

                // Get track metadata for the new selection
                let track_data = if let Some(app) = app_weak.upgrade() {
                    let current_idx = app.get_selected_track_index();
                    let new_idx = if current_idx >= new_count { new_count - 1 } else { current_idx };

                    if new_idx >= 0 {
                        state.project.as_ref().and_then(|p| {
                            p.album.tracks.get(new_idx as usize).map(|t| {
                                (new_idx, t.title.clone(), t.pregap.as_secs(), t.number)
                            })
                        })
                    } else {
                        None
                    }
                } else {
                    None
                };

                // Extract waveform if we have a track
                let waveform_data = if let Some((_, _, _, track_num)) = track_data {
                    if state.extract_waveform(track_num).is_some() {
                        let peaks = state.get_visible_peaks();
                        let duration = state.current_waveform.as_ref().map(|c| c.duration_str.clone()).unwrap_or_default();
                        Some((peaks, duration))
                    } else {
                        None
                    }
                } else {
                    None
                };

                (tracks, new_count, track_data, waveform_data)
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
                } else if let Some((new_idx, title, pregap, _)) = track_data {
                    app.set_selected_track_index(new_idx);
                    app.set_current_track_title(title.into());
                    app.set_current_track_pregap(pregap.to_string().into());
                    app.set_playhead_position(0.0);

                    if let Some((peaks, duration)) = waveform_data {
                        app.set_waveform_peaks(std::rc::Rc::new(slint::VecModel::from(peaks)).into());
                        app.set_waveform_duration(duration.into());
                    }
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

        // First pass: reorder tracks and get metadata
        let (tracks, new_selected_idx, track_metadata) = {
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

            // Get metadata for the moved track (now at insert_idx)
            let track_metadata = project.album.tracks.get(insert_idx).map(|t| {
                (t.title.clone(), t.pregap.as_secs(), t.number)
            });

            // Prepare UI data
            let tracks = state.tracks_to_model();

            // Invalidate waveform cache since track numbers changed
            state.current_waveform = None;

            (tracks, insert_idx as i32, track_metadata)
        };

        // Second pass: extract waveform (needs separate borrow)
        let waveform_data = if let Some((_, _, track_num)) = track_metadata {
            let mut state = state_clone.borrow_mut();
            if state.extract_waveform(track_num).is_some() {
                let peaks = state.get_visible_peaks();
                let duration = state.current_waveform.as_ref().map(|c| c.duration_str.clone()).unwrap_or_default();
                Some((peaks, duration))
            } else {
                None
            }
        } else {
            None
        };

        // Update UI
        if let Some(app) = app_weak.upgrade() {
            let model = Rc::new(slint::VecModel::from(tracks));
            app.set_tracks(model.into());
            app.set_selected_track_index(new_selected_idx);

            // Update metadata editor
            if let Some((title, pregap, _)) = track_metadata {
                app.set_current_track_title(title.into());
                app.set_current_track_pregap(pregap.to_string().into());
            }

            // Update waveform
            if let Some((peaks, duration)) = waveform_data {
                app.set_waveform_peaks(Rc::new(slint::VecModel::from(peaks)).into());
                app.set_waveform_duration(duration.into());
            }

            app.set_playhead_position(0.0);
            app.set_status_message("Track reordered".into());
        }
    });

    // Set up a timer to poll for position updates from the audio engine
    let app_weak = app.as_weak();
    let engine_for_timer = audio_engine.clone();
    let state_for_timer = state.clone();
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(50),
        move || {
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
                                let track_num = next_track.number as u8;

                                // Update metadata editor fields
                                app.set_current_track_title(next_track.title.clone());
                                app.set_current_track_pregap(next_track.pregap.clone());

                                // Load the next track
                                let mut state = state_for_timer.borrow_mut();
                                if let Some(track) = state.get_track(track_num) {
                                    let path = track.source_file.clone();
                                    state.current_track_path = Some(path.clone());
                                    state.current_track_num = Some(track_num);
                                    engine_for_timer.load(path);
                                }

                                // Extract waveform
                                if state.extract_waveform(track_num).is_some() {
                                    let peaks = state.get_visible_peaks();
                                    let duration = state.current_waveform.as_ref().map(|c| c.duration_str.clone()).unwrap_or_default();

                                    let model = Rc::new(slint::VecModel::from(peaks));
                                    app.set_waveform_peaks(model.into());
                                    app.set_waveform_duration(duration.into());
                                }

                                app.set_selected_track_index(next_index);
                                app.set_playhead_position(0.0);
                                drop(state);

                                // Start playing the next track
                                engine_for_timer.play();
                                let mut playback = app.get_playback();
                                playback.is_playing = true;
                                playback.is_paused = false;
                                app.set_playback(playback);
                                app.set_status_message(format!("Playing track {}", track_num).into());
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

// Red Book Master GUI
// Professional CD mastering application with Slint UI

mod player;

use std::rc::Rc;
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;

use redbookmaster_lib::{Album, Project, Track, extract_peaks};
use redbookmaster_lib::core::track::format_duration_ms;
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
}

/// Cached waveform data
struct WaveformCache {
    track_number: u8,
    peaks: Vec<WaveformPeak>,
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

    /// Extract waveform for a track
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

        // Extract peaks
        match extract_peaks(path, WAVEFORM_BINS) {
            Ok(waveform_data) => {
                let peaks: Vec<WaveformPeak> = waveform_data.peaks.iter().map(|&(min, max)| {
                    WaveformPeak { min, max }
                }).collect();

                self.current_waveform = Some(WaveformCache {
                    track_number: track_num,
                    peaks,
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
            } else {
                // No path set, trigger Save As
                drop(state);
                // TODO: Implement save as dialog
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
            // Find the track index
            let tracks = app.get_tracks();
            for i in 0..tracks.row_count() {
                if let Some(track) = tracks.row_data(i) {
                    if track.number == track_num {
                        app.set_selected_track_index(i as i32);
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
        if let Some(cache) = state.extract_waveform(track_num as u8) {
            if let Some(app) = app_weak.upgrade() {
                let peaks = cache.peaks.clone();
                let duration = cache.duration_str.clone();

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

                    // Get track path and load into engine
                    let mut state = state_clone.borrow_mut();
                    if let Some(track) = state.get_track(track_num) {
                        let path = track.source_file.clone();
                        state.current_track_path = Some(path.clone());
                        state.current_track_num = Some(track_num);
                        engine_clone.load(path);
                    }

                    // Extract waveform
                    if let Some(cache) = state.extract_waveform(track_num) {
                        let peaks = cache.peaks.clone();
                        let duration = cache.duration_str.clone();

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

    app.on_zoom_in(|| {
        println!("Zoom in");
    });

    app.on_zoom_out(|| {
        println!("Zoom out");
    });

    app.on_export_master(|| {
        println!("Export clicked");
        // TODO: Implement export dialog
    });

    app.on_burn_cd(|| {
        println!("Burn CD clicked");
        // TODO: Implement burn dialog
    });

    app.on_save_project_as(|| {
        println!("Save As clicked");
        // TODO: Implement save as dialog
    });

    app.on_remove_track(|_track_num| {
        println!("Remove track clicked");
        // TODO: Implement track removal
    });

    app.on_update_track_pregap(|track_num, pregap| {
        println!("Update pregap for track {}: {}", track_num, pregap);
        // TODO: Implement pregap update
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

                                // Load the next track
                                let mut state = state_for_timer.borrow_mut();
                                if let Some(track) = state.get_track(track_num) {
                                    let path = track.source_file.clone();
                                    state.current_track_path = Some(path.clone());
                                    state.current_track_num = Some(track_num);
                                    engine_for_timer.load(path);
                                }

                                // Extract waveform
                                if let Some(cache) = state.extract_waveform(track_num) {
                                    let peaks = cache.peaks.clone();
                                    let duration = cache.duration_str.clone();

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

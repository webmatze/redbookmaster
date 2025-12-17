use colored::Colorize;
use inquire::{Confirm, Select, Text};
use std::path::PathBuf;

use crate::core::{Album, Project};

/// Display the main menu when launching without arguments
pub fn main_menu() {
    println!();
    println!("{}", "Welcome to Red Book Master!".bold().cyan());
    println!("{}", "─".repeat(40).dimmed());
    println!();

    let options = vec![
        "Create new project",
        "Open existing project",
        "Exit",
    ];

    let selection = Select::new("What would you like to do?", options)
        .with_help_message("Use arrow keys to navigate, Enter to select")
        .prompt();

    match selection {
        Ok("Create new project") => new_project(),
        Ok("Open existing project") => open_project_dialog(),
        Ok("Exit") | Err(_) => {
            println!("Goodbye!");
        }
        _ => {}
    }
}

/// Start the new project wizard
pub fn new_project() {
    println!();
    println!("{}", "Create New Project".bold().green());
    println!("{}", "─".repeat(40).dimmed());
    println!();

    // Get album title
    let title = match Text::new("Album title:")
        .with_help_message("Enter the title of your album")
        .prompt()
    {
        Ok(t) if !t.trim().is_empty() => t.trim().to_string(),
        Ok(_) => {
            println!("{} Album title cannot be empty", "✗".red());
            return;
        }
        Err(_) => return,
    };

    // Get performer/artist
    let performer = match Text::new("Artist/Performer:")
        .with_help_message("Enter the main artist or band name")
        .prompt()
    {
        Ok(p) if !p.trim().is_empty() => p.trim().to_string(),
        Ok(_) => {
            println!("{} Artist name cannot be empty", "✗".red());
            return;
        }
        Err(_) => return,
    };

    // Optional songwriter
    let songwriter = match Text::new("Songwriter (optional):")
        .with_help_message("Press Enter to skip")
        .prompt()
    {
        Ok(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        _ => None,
    };

    // Optional catalog number (MCN/UPC)
    let _catalog = match Text::new("Catalog number (MCN/UPC, optional):")
        .with_help_message("13-digit UPC/EAN code, press Enter to skip")
        .prompt()
    {
        Ok(c) if !c.trim().is_empty() => {
            // Validate MCN format
            match crate::core::metadata::Mcn::new(c.trim()) {
                Ok(mcn) => Some(mcn),
                Err(e) => {
                    println!("{} Invalid catalog number: {}. Skipping.", "⚠".yellow(), e);
                    None
                }
            }
        }
        _ => None,
    };

    // Create the album
    let mut album = Album::new(title.clone(), performer);
    album.songwriter = songwriter;
    // album.catalog = catalog; // Uncomment when ready

    // Create project
    let mut project = Project::new(album);

    println!();
    println!("{} Created new project: {}", "✓".green(), title.bold());

    // Ask to add tracks
    let add_tracks = Confirm::new("Add WAV files now?")
        .with_default(true)
        .prompt();

    if matches!(add_tracks, Ok(true)) {
        add_tracks_wizard(&mut project);
    }

    // Save project
    save_project_dialog(&mut project);

    // Enter project menu
    project_menu(project);
}

/// Convert a non-compliant WAV file and add it as a track
fn convert_and_add_track(
    project: &mut Project,
    source_path: &std::path::Path,
    title: &str,
    info: &crate::audio::WavInfo,
) -> Result<(), String> {
    use indicatif::{ProgressBar, ProgressStyle};

    println!();
    println!("{} Converting to Red Book format...", "⟳".cyan());
    println!(
        "  Input: {}Hz, {}-bit, {} channel(s)",
        info.sample_rate, info.bits_per_sample, info.channels
    );
    println!("  Target: 44100Hz, 16-bit, stereo");

    // Create output filename in same directory as source
    let output_path = source_path.with_file_name(format!(
        "{}_converted.wav",
        source_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("track")
    ));

    // Show progress spinner
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg}")
            .unwrap(),
    );
    pb.set_message("Converting audio...");
    pb.enable_steady_tick(std::time::Duration::from_millis(100));

    // Perform conversion
    let result = crate::audio::convert_to_red_book(source_path, &output_path);

    pb.finish_and_clear();

    match result {
        Ok(conversion) => {
            println!("{} Conversion complete: {}", "✓".green(), conversion.summary());

            // Read the converted file info
            match crate::audio::wav::read_wav_info(&output_path) {
                Ok(new_info) => {
                    let track = crate::core::Track::new(
                        (project.album.track_count() + 1) as u8,
                        title.to_string(),
                        output_path.clone(),
                        new_info.duration,
                    );

                    project.album.add_track(track);
                    println!(
                        "{} Added: {} ({})",
                        "✓".green(),
                        title,
                        crate::core::track::format_duration_ms(new_info.duration)
                    );
                    println!("  Converted file: {:?}", output_path);

                    Ok(())
                }
                Err(e) => Err(format!("Failed to read converted file: {}", e)),
            }
        }
        Err(e) => Err(format!("{}", e)),
    }
}

/// Add tracks wizard
pub fn add_tracks_wizard(project: &mut Project) {
    println!();
    println!("{}", "Add Tracks".bold().green());
    println!("{}", "─".repeat(40).dimmed());
    println!();

    println!("Enter paths to WAV files (one per line).");
    println!("Press Enter on an empty line when done.");
    println!();

    loop {
        let path_input = match Text::new("WAV file path:")
            .with_help_message("Enter path or press Enter to finish")
            .prompt()
        {
            Ok(p) => p,
            Err(_) => break,
        };

        if path_input.trim().is_empty() {
            break;
        }

        let path = PathBuf::from(path_input.trim());

        if !path.exists() {
            println!("{} File not found: {:?}", "✗".red(), path);
            continue;
        }

        if !path.extension().map_or(false, |e| e.eq_ignore_ascii_case("wav")) {
            println!("{} Not a WAV file: {:?}", "✗".red(), path);
            continue;
        }

        // Try to read WAV file info
        match crate::audio::wav::read_wav_info(&path) {
            Ok(info) => {
                // Auto-generate track title from filename
                let title = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(|s| clean_track_title(s))
                    .unwrap_or_else(|| format!("Track {}", project.album.track_count() + 1));

                let track = crate::core::Track::new(
                    (project.album.track_count() + 1) as u8,
                    title.clone(),
                    path.clone(),
                    info.duration,
                );

                // Check format compliance
                if !info.is_red_book_compliant() {
                    println!(
                        "{} File is not Red Book compliant: {}",
                        "⚠".yellow(),
                        info.format_issues().join(", ")
                    );

                    let action = Select::new(
                        "What would you like to do?",
                        vec!["Convert automatically", "Skip this file", "Abort"],
                    )
                    .prompt();

                    match action {
                        Ok("Convert automatically") => {
                            match convert_and_add_track(project, &path, &title, &info) {
                                Ok(()) => continue,
                                Err(e) => {
                                    println!("{} Conversion failed: {}", "✗".red(), e);
                                    continue;
                                }
                            }
                        }
                        Ok("Skip this file") => continue,
                        Ok("Abort") | Err(_) => break,
                        _ => continue,
                    }
                } else {
                    // File is already compliant
                    project.album.add_track(track);
                    println!(
                        "{} Added: {} ({})",
                        "✓".green(),
                        title,
                        crate::core::track::format_duration_ms(info.duration)
                    );
                }
            }
            Err(e) => {
                println!("{} Failed to read WAV file: {}", "✗".red(), e);
            }
        }
    }

    println!();
    println!(
        "Added {} track(s), total duration: {}",
        project.album.track_count(),
        project.album.format_duration()
    );
}

/// Clean up a filename to use as track title
fn clean_track_title(filename: &str) -> String {
    // Remove common prefixes like "01 - ", "01_", "Track 01 - ", etc.
    let re_patterns = [
        r"^\d{1,2}[\s._-]+",           // "01 - ", "01_", "01."
        r"^Track\s*\d{1,2}[\s._-]*",   // "Track 01 - "
    ];

    let mut title = filename.to_string();

    for pattern in &re_patterns {
        if let Ok(re) = regex::Regex::new(pattern) {
            title = re.replace(&title, "").to_string();
        }
    }

    // Replace underscores with spaces
    title = title.replace('_', " ");

    // Trim and capitalize first letter
    title = title.trim().to_string();
    if let Some(first) = title.get_mut(0..1) {
        first.make_ascii_uppercase();
    }

    title
}

/// Save project dialog
pub fn save_project_dialog(project: &mut Project) {
    let default_name = project.default_filename();

    let filename = match Text::new("Save project as:")
        .with_default(&default_name)
        .with_help_message("Project will be saved with .rbm extension")
        .prompt()
    {
        Ok(f) => f,
        Err(_) => return,
    };

    let mut path = PathBuf::from(filename.trim());

    // Ensure .rbm extension
    if path.extension().map_or(true, |e| !e.eq_ignore_ascii_case("rbm")) {
        path.set_extension("rbm");
    }

    match project.save_as(&path) {
        Ok(()) => {
            println!("{} Project saved to: {:?}", "✓".green(), path);
        }
        Err(e) => {
            println!("{} Failed to save project: {}", "✗".red(), e);
        }
    }
}

/// Open project dialog
fn open_project_dialog() {
    let path_input = match Text::new("Project file path:")
        .with_help_message("Enter the path to a .rbm project file")
        .prompt()
    {
        Ok(p) => p,
        Err(_) => return,
    };

    let path = PathBuf::from(path_input.trim());

    match Project::load(&path) {
        Ok(project) => {
            println!("{} Loaded project: {}", "✓".green(), project.album.title);
            project_menu(project);
        }
        Err(e) => {
            println!("{} Failed to load project: {}", "✗".red(), e);
        }
    }
}

/// Project menu - main workflow after loading/creating a project
pub fn project_menu(mut project: Project) {
    loop {
        println!();
        println!("{}", format!("Project: {}", project.album.title).bold().cyan());
        println!(
            "{} tracks, {} total",
            project.album.track_count(),
            project.album.format_duration()
        );
        println!("{}", "─".repeat(40).dimmed());

        let options = vec![
            "View tracks",
            "Add tracks",
            "Edit track metadata",
            "Edit album metadata",
            "Reorder tracks",
            "Configure gaps",
            "Play preview",
            "Export master",
            "Burn CD",
            "Validate",
            "Save",
            "Exit",
        ];

        let selection = Select::new("Choose an action:", options)
            .with_help_message("Use arrow keys to navigate")
            .prompt();

        match selection {
            Ok("View tracks") => view_tracks(&project),
            Ok("Add tracks") => add_tracks_wizard(&mut project),
            Ok("Edit track metadata") => edit_track_metadata(&mut project),
            Ok("Edit album metadata") => edit_album_metadata(&mut project),
            Ok("Reorder tracks") => reorder_tracks(&mut project),
            Ok("Configure gaps") => configure_gaps(&mut project),
            Ok("Play preview") => play_preview(&project),
            Ok("Export master") => export_master(&project),
            Ok("Burn CD") => burn_cd(&project),
            Ok("Validate") => validate_project(&project),
            Ok("Save") => {
                match project.save() {
                    Ok(()) => println!("{} Project saved", "✓".green()),
                    Err(e) => {
                        println!("{} Save failed: {}. Use Save As.", "✗".red(), e);
                        save_project_dialog(&mut project);
                    }
                }
            }
            Ok("Exit") | Err(_) => {
                println!("Goodbye!");
                break;
            }
            _ => {}
        }
    }
}

/// View all tracks in the project
fn view_tracks(project: &Project) {
    println!();
    println!("{}", "Track List".bold().green());
    println!("{}", "─".repeat(60).dimmed());

    if project.album.tracks.is_empty() {
        println!("No tracks yet. Use 'Add tracks' to add WAV files.");
        return;
    }

    for track in &project.album.tracks {
        let duration = crate::core::track::format_duration_ms(track.duration);
        let pregap = if track.pregap.as_secs() > 0 {
            format!(" [gap: {}s]", track.pregap.as_secs())
        } else {
            String::new()
        };

        println!(
            "{:2}. {} ({}){}",
            track.number,
            track.title,
            duration,
            pregap.dimmed()
        );

        if let Some(ref isrc) = track.isrc {
            println!("    ISRC: {}", isrc);
        }
    }

    println!("{}", "─".repeat(60).dimmed());
    println!(
        "Total: {} tracks, {}",
        project.album.track_count(),
        project.album.format_duration()
    );
}

/// Edit track metadata
fn edit_track_metadata(project: &mut Project) {
    if project.album.tracks.is_empty() {
        println!("No tracks to edit.");
        return;
    }

    // List tracks for selection
    let track_options: Vec<String> = project
        .album
        .tracks
        .iter()
        .map(|t| format!("{}. {}", t.number, t.title))
        .collect();

    let selection = Select::new("Select track to edit:", track_options.clone()).prompt();

    let track_num = match selection {
        Ok(s) => {
            // Extract track number from selection
            s.split('.').next().and_then(|n| n.trim().parse::<u8>().ok())
        }
        Err(_) => return,
    };

    if let Some(num) = track_num {
        if let Some(track) = project.album.get_track_mut(num) {
            // Edit title
            if let Ok(new_title) = Text::new("Title:")
                .with_default(&track.title)
                .prompt()
            {
                track.title = new_title;
            }

            // Edit performer
            let current_performer = track.performer.as_deref().unwrap_or("");
            if let Ok(new_performer) = Text::new("Performer (optional):")
                .with_default(current_performer)
                .prompt()
            {
                track.performer = if new_performer.trim().is_empty() {
                    None
                } else {
                    Some(new_performer)
                };
            }

            // Edit ISRC
            let current_isrc = track.isrc.as_ref().map(|i| i.formatted()).unwrap_or_default();
            if let Ok(new_isrc) = Text::new("ISRC (optional):")
                .with_default(&current_isrc)
                .with_help_message("Format: CC-XXX-YY-NNNNN")
                .prompt()
            {
                track.isrc = if new_isrc.trim().is_empty() {
                    None
                } else {
                    match crate::core::Isrc::new(&new_isrc) {
                        Ok(isrc) => Some(isrc),
                        Err(e) => {
                            println!("{} Invalid ISRC: {}", "⚠".yellow(), e);
                            track.isrc.clone()
                        }
                    }
                };
            }

            println!("{} Track updated", "✓".green());
        }
    }
}

/// Edit album metadata
fn edit_album_metadata(project: &mut Project) {
    println!();
    println!("{}", "Edit Album Metadata".bold().green());
    println!("{}", "─".repeat(40).dimmed());

    // Edit title
    if let Ok(new_title) = Text::new("Album title:")
        .with_default(&project.album.title)
        .prompt()
    {
        project.album.title = new_title;
    }

    // Edit performer
    if let Ok(new_performer) = Text::new("Artist/Performer:")
        .with_default(&project.album.performer)
        .prompt()
    {
        project.album.performer = new_performer;
    }

    // Edit songwriter
    let current_songwriter = project.album.songwriter.as_deref().unwrap_or("");
    if let Ok(new_songwriter) = Text::new("Songwriter (optional):")
        .with_default(current_songwriter)
        .prompt()
    {
        project.album.songwriter = if new_songwriter.trim().is_empty() {
            None
        } else {
            Some(new_songwriter)
        };
    }

    // Edit catalog number
    let current_catalog = project.album.catalog.as_ref().map(|c| c.to_string()).unwrap_or_default();
    if let Ok(new_catalog) = Text::new("Catalog number (MCN/UPC, optional):")
        .with_default(&current_catalog)
        .prompt()
    {
        project.album.catalog = if new_catalog.trim().is_empty() {
            None
        } else {
            match crate::core::Mcn::new(&new_catalog) {
                Ok(mcn) => Some(mcn),
                Err(e) => {
                    println!("{} Invalid catalog number: {}", "⚠".yellow(), e);
                    project.album.catalog.clone()
                }
            }
        };
    }

    println!("{} Album metadata updated", "✓".green());
}

/// Reorder tracks interactively
fn reorder_tracks(project: &mut Project) {
    if project.album.tracks.len() < 2 {
        println!("Need at least 2 tracks to reorder.");
        return;
    }

    println!();
    println!("{}", "Reorder Tracks".bold().green());
    println!("Select a track to move, then select its new position.");
    println!();

    view_tracks(project);

    // Select track to move
    let track_options: Vec<String> = project
        .album
        .tracks
        .iter()
        .map(|t| format!("{}. {}", t.number, t.title))
        .collect();

    let from_selection = Select::new("Select track to move:", track_options.clone()).prompt();

    let from_idx = match from_selection {
        Ok(s) => s.split('.').next().and_then(|n| n.trim().parse::<usize>().ok()).map(|n| n - 1),
        Err(_) => return,
    };

    // Select new position
    let to_selection = Select::new("Move to position:", track_options).prompt();

    let to_idx = match to_selection {
        Ok(s) => s.split('.').next().and_then(|n| n.trim().parse::<usize>().ok()).map(|n| n - 1),
        Err(_) => return,
    };

    if let (Some(from), Some(to)) = (from_idx, to_idx) {
        if project.album.move_track(from, to) {
            println!("{} Track moved", "✓".green());
        } else {
            println!("{} Failed to move track", "✗".red());
        }
    }
}

/// Configure track gaps
fn configure_gaps(project: &mut Project) {
    if project.album.tracks.is_empty() {
        println!("No tracks to configure.");
        return;
    }

    println!();
    println!("{}", "Configure Track Gaps".bold().green());
    println!("{}", "─".repeat(40).dimmed());
    println!();
    println!("Pregap: Silence BEFORE the track");
    println!("Note: Track 1 requires minimum 2 second pregap (Red Book spec)");
    println!();

    // List tracks for selection
    let track_options: Vec<String> = project
        .album
        .tracks
        .iter()
        .map(|t| {
            format!(
                "{}. {} (pregap: {}s)",
                t.number,
                t.title,
                t.pregap.as_secs()
            )
        })
        .collect();

    let selection = Select::new("Select track to configure:", track_options).prompt();

    let track_num = match selection {
        Ok(s) => s.split('.').next().and_then(|n| n.trim().parse::<u8>().ok()),
        Err(_) => return,
    };

    if let Some(num) = track_num {
        if let Some(track) = project.album.get_track_mut(num) {
            let min_pregap = if num == 1 { 2 } else { 0 };
            let current_pregap = track.pregap.as_secs();

            let pregap_str = match Text::new("Pregap (seconds):")
                .with_default(&current_pregap.to_string())
                .with_help_message(&format!("Minimum: {} seconds", min_pregap))
                .prompt()
            {
                Ok(s) => s,
                Err(_) => return,
            };

            if let Ok(pregap) = pregap_str.trim().parse::<u64>() {
                if pregap < min_pregap {
                    println!(
                        "{} Pregap must be at least {} seconds for track {}",
                        "✗".red(),
                        min_pregap,
                        num
                    );
                } else {
                    track.pregap = std::time::Duration::from_secs(pregap);
                    println!("{} Gap updated", "✓".green());
                }
            } else {
                println!("{} Invalid number", "✗".red());
            }
        }
    }
}

/// Play preview - audio playback interface
fn play_preview(project: &Project) {
    if project.album.tracks.is_empty() {
        println!("No tracks to play.");
        return;
    }

    println!();
    println!("{}", "Play Preview".bold().green());
    println!("{}", "─".repeat(40).dimmed());

    // Build track selection list
    let mut options: Vec<String> = project
        .album
        .tracks
        .iter()
        .map(|t| {
            format!(
                "{}. {} ({})",
                t.number,
                t.title,
                crate::core::track::format_duration_ms(t.duration)
            )
        })
        .collect();

    options.push("Play all tracks".to_string());
    options.push("Play transition between tracks".to_string());
    options.push("Cancel".to_string());

    let selection = match Select::new("What would you like to play?", options).prompt() {
        Ok(s) => s,
        Err(_) => return,
    };

    if selection == "Cancel" {
        return;
    }

    if selection == "Play all tracks" {
        play_all_tracks(project);
        return;
    }

    if selection == "Play transition between tracks" {
        play_transition(project);
        return;
    }

    // Play specific track
    let track_num = selection
        .split('.')
        .next()
        .and_then(|n| n.trim().parse::<usize>().ok());

    if let Some(num) = track_num {
        if let Some(track) = project.album.tracks.get(num - 1) {
            play_single_track(track);
        }
    }
}

/// Play a single track with progress display
fn play_single_track(track: &crate::core::Track) {
    use indicatif::{ProgressBar, ProgressStyle};
    use std::io::{self, Read};

    println!();
    println!(
        "Playing: {} {}",
        format!("Track {}", track.number).cyan(),
        track.title.bold()
    );
    println!("{}", "─".repeat(50).dimmed());

    // Check if file exists
    if !track.source_file.exists() {
        println!("{} File not found: {:?}", "✗".red(), track.source_file);
        return;
    }

    // Initialize player
    let mut player = match crate::audio::Player::new() {
        Ok(p) => p,
        Err(e) => {
            println!("{} Failed to initialize audio: {}", "✗".red(), e);
            return;
        }
    };

    // Start playback
    let duration = match player.play(&track.source_file) {
        Ok(d) => d,
        Err(e) => {
            println!("{} Failed to play: {}", "✗".red(), e);
            return;
        }
    };

    let duration_secs = duration.as_secs();
    let duration_str = crate::core::track::format_duration_ms(duration);

    // Create progress bar
    let pb = ProgressBar::new(duration_secs);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} {msg}")
            .unwrap()
            .progress_chars("█▓░"),
    );

    println!();
    println!("Controls: [Space] Pause/Resume  [Q] Stop  [+/-] Volume");
    println!();

    // Set terminal to raw mode for keyboard input
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let stdin_fd = io::stdin().as_raw_fd();
        let mut termios = termios::Termios::from_fd(stdin_fd).ok();

        if let Some(ref mut t) = termios {
            let original = t.clone();
            t.c_lflag &= !(termios::ICANON | termios::ECHO);
            t.c_cc[termios::VMIN] = 0;
            t.c_cc[termios::VTIME] = 1;
            let _ = termios::tcsetattr(stdin_fd, termios::TCSANOW, t);

            // Playback loop
            let start = std::time::Instant::now();
            let mut paused_time = std::time::Duration::ZERO;
            let mut pause_start: Option<std::time::Instant> = None;

            loop {
                // Check if playback finished
                if player.is_finished() {
                    pb.finish_with_message("Done!");
                    break;
                }

                // Calculate elapsed time (accounting for pauses)
                let elapsed = if player.is_paused() {
                    if pause_start.is_none() {
                        pause_start = Some(std::time::Instant::now());
                    }
                    start.elapsed().saturating_sub(paused_time)
                } else {
                    if let Some(ps) = pause_start.take() {
                        paused_time += ps.elapsed();
                    }
                    start.elapsed().saturating_sub(paused_time)
                };

                let elapsed_secs = elapsed.as_secs().min(duration_secs);
                pb.set_position(elapsed_secs);

                let status = if player.is_paused() { "⏸ PAUSED" } else { "▶ Playing" };
                let elapsed_str = crate::core::track::format_duration_ms(elapsed);
                pb.set_message(format!("{} {}/{}", status, elapsed_str, duration_str));

                // Check for keyboard input
                let mut buf = [0u8; 1];
                if io::stdin().read(&mut buf).unwrap_or(0) > 0 {
                    match buf[0] {
                        b' ' => player.toggle_pause(),
                        b'q' | b'Q' => {
                            player.stop();
                            pb.finish_with_message("Stopped");
                            break;
                        }
                        b'+' | b'=' => {
                            let vol = (player.volume() + 0.1).min(1.0);
                            player.set_volume(vol);
                        }
                        b'-' | b'_' => {
                            let vol = (player.volume() - 0.1).max(0.0);
                            player.set_volume(vol);
                        }
                        _ => {}
                    }
                }

                std::thread::sleep(std::time::Duration::from_millis(100));
            }

            // Restore terminal
            let _ = termios::tcsetattr(stdin_fd, termios::TCSANOW, &original);
        } else {
            // Fallback: simple blocking playback
            simple_playback(&player, duration);
        }
    }

    #[cfg(not(unix))]
    {
        simple_playback(&player, duration);
    }

    println!();
}

/// Simple blocking playback without keyboard controls
fn simple_playback(player: &crate::audio::Player, duration: std::time::Duration) {
    use indicatif::{ProgressBar, ProgressStyle};

    let duration_secs = duration.as_secs();
    let pb = ProgressBar::new(duration_secs);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len}s")
            .unwrap()
            .progress_chars("█▓░"),
    );

    println!("Playing... (Press Ctrl+C to stop)");

    let start = std::time::Instant::now();
    while !player.is_finished() {
        let elapsed = start.elapsed().as_secs().min(duration_secs);
        pb.set_position(elapsed);
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    pb.finish_with_message("Done!");
}

/// Play all tracks in sequence
fn play_all_tracks(project: &Project) {
    println!();
    println!(
        "{} {}",
        "Playing album:".green(),
        project.album.title.bold()
    );
    println!("{}", "─".repeat(50).dimmed());
    println!("Press Ctrl+C to stop");
    println!();

    // Initialize player once
    let mut player = match crate::audio::Player::new() {
        Ok(p) => p,
        Err(e) => {
            println!("{} Failed to initialize audio: {}", "✗".red(), e);
            return;
        }
    };

    for track in &project.album.tracks {
        println!(
            "\n{} {} - {}",
            format!("Track {}:", track.number).cyan(),
            track.title.bold(),
            crate::core::track::format_duration_ms(track.duration)
        );

        if !track.source_file.exists() {
            println!("{} File not found, skipping...", "⚠".yellow());
            continue;
        }

        match player.play(&track.source_file) {
            Ok(_) => {
                // Wait for track to finish
                player.wait_until_end();
            }
            Err(e) => {
                println!("{} Failed to play: {}", "✗".red(), e);
            }
        }
    }

    println!();
    println!("{} Playback complete!", "✓".green());
}

/// Play transition between two tracks
fn play_transition(project: &Project) {
    if project.album.tracks.len() < 2 {
        println!("Need at least 2 tracks to play a transition.");
        return;
    }

    println!();
    println!("{}", "Play Transition".bold().green());
    println!("This will play the last few seconds of one track");
    println!("followed by the gap and start of the next track.");
    println!();

    // Select which transition to play
    let transition_options: Vec<String> = (1..project.album.tracks.len())
        .map(|i| {
            let t1 = &project.album.tracks[i - 1];
            let t2 = &project.album.tracks[i];
            format!(
                "Track {} -> Track {} ({} -> {})",
                t1.number, t2.number, t1.title, t2.title
            )
        })
        .collect();

    let selection = match Select::new("Select transition to preview:", transition_options).prompt() {
        Ok(s) => s,
        Err(_) => return,
    };

    // Parse selection to get track indices
    let track_idx = selection
        .split("->")
        .next()
        .and_then(|s| s.trim().strip_prefix("Track "))
        .and_then(|n| n.trim().parse::<usize>().ok())
        .map(|n| n - 1);

    let Some(idx) = track_idx else {
        println!("{} Failed to parse selection", "✗".red());
        return;
    };

    let track1 = &project.album.tracks[idx];
    let track2 = &project.album.tracks[idx + 1];

    println!();
    println!(
        "Playing: {} {} -> {} {}",
        format!("Track {}", track1.number).cyan(),
        track1.title,
        format!("Track {}", track2.number).cyan(),
        track2.title
    );

    // Initialize player
    let mut player = match crate::audio::Player::new() {
        Ok(p) => p,
        Err(e) => {
            println!("{} Failed to initialize audio: {}", "✗".red(), e);
            return;
        }
    };

    // Play last 5 seconds of track 1
    let preview_duration = std::time::Duration::from_secs(5);
    let skip_to = track1.duration.saturating_sub(preview_duration);

    println!("  Playing end of track {}...", track1.number);

    if track1.source_file.exists() {
        if let Ok(_) = player.play_from(&track1.source_file, skip_to) {
            player.wait_until_end();
        }
    }

    // Play gap (silence)
    if track2.pregap.as_millis() > 0 {
        println!(
            "  [Gap: {} seconds]",
            track2.pregap.as_secs_f32()
        );
        std::thread::sleep(track2.pregap.min(std::time::Duration::from_secs(3)));
    }

    // Play first 5 seconds of track 2
    println!("  Playing start of track {}...", track2.number);

    if track2.source_file.exists() {
        if let Ok(duration) = player.play(&track2.source_file) {
            // Only play first 5 seconds
            let play_time = duration.min(preview_duration);
            std::thread::sleep(play_time);
            player.stop();
        }
    }

    println!();
    println!("{} Transition preview complete!", "✓".green());
}

/// Export master - generate CUE sheet, WAV, and TOC files
fn export_master(project: &Project) {
    use indicatif::{ProgressBar, ProgressStyle};

    println!();
    println!("{}", "Export Master".bold().green());
    println!("{}", "─".repeat(40).dimmed());
    println!();

    // Check for empty project
    if project.album.tracks.is_empty() {
        println!("{} No tracks to export. Add tracks first.", "✗".red());
        return;
    }

    // Validate first
    match project.album.validate() {
        Ok(()) => {
            println!("{} Project is Red Book compliant", "✓".green());
        }
        Err(e) => {
            println!("{} Validation error: {}", "✗".red(), e);
            println!("Please fix the issues before exporting.");
            return;
        }
    }

    // Check that all source files exist and are Red Book compliant
    println!();
    println!("Checking source files...");
    for track in &project.album.tracks {
        if !track.source_file.exists() {
            println!(
                "{} Source file missing: {:?}",
                "✗".red(),
                track.source_file
            );
            return;
        }

        match crate::audio::wav::read_wav_info(&track.source_file) {
            Ok(info) => {
                if !info.is_red_book_compliant() {
                    println!(
                        "{} Track {} is not Red Book compliant: {}",
                        "✗".red(),
                        track.number,
                        info.format_issues().join(", ")
                    );
                    println!("Please convert the file first using 'Add tracks'.");
                    return;
                }
            }
            Err(e) => {
                println!(
                    "{} Cannot read track {}: {}",
                    "✗".red(),
                    track.number,
                    e
                );
                return;
            }
        }
    }
    println!("{} All source files OK", "✓".green());

    // Choose export format
    println!();
    let format_options = vec![
        "Single WAV + CUE (recommended for burning)",
        "Multi-file CUE (references original WAV files)",
    ];

    let format_selection = match Select::new("Export format:", format_options).prompt() {
        Ok(s) => s,
        Err(_) => return,
    };

    let single_wav_mode = format_selection.starts_with("Single");

    // Get output directory
    let default_dir = project
        .file_path
        .as_ref()
        .and_then(|p| p.parent())
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| ".".to_string());

    let output_dir = match Text::new("Output directory:")
        .with_default(&default_dir)
        .with_help_message("Directory where export files will be saved")
        .prompt()
    {
        Ok(d) => PathBuf::from(d.trim()),
        Err(_) => return,
    };

    // Create output directory if needed
    if !output_dir.exists() {
        if let Err(e) = std::fs::create_dir_all(&output_dir) {
            println!("{} Failed to create directory: {}", "✗".red(), e);
            return;
        }
    }

    // Generate base filename
    let base_name = project.name();
    let cue_path = output_dir.join(format!("{}.cue", base_name));
    let toc_path = output_dir.join(format!("{}.toc", base_name));
    let wav_path = output_dir.join(format!("{}.wav", base_name));

    println!();
    println!("{}", "Exporting...".bold());

    if single_wav_mode {
        // Export single concatenated WAV + CUE + TOC

        // 1. Concatenate tracks into single WAV
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.green} {msg}")
                .unwrap(),
        );
        pb.set_message("Concatenating audio tracks...");
        pb.enable_steady_tick(std::time::Duration::from_millis(100));

        match crate::audio::concat::concatenate_tracks(&project.album.tracks, &wav_path) {
            Ok(()) => {
                pb.finish_and_clear();
                println!("{} Created: {}", "✓".green(), wav_path.display());
            }
            Err(e) => {
                pb.finish_and_clear();
                println!("{} Failed to concatenate audio: {}", "✗".red(), e);
                return;
            }
        }

        // 2. Generate CUE sheet
        let wav_filename = wav_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("master.wav");

        match crate::export::cue::generate_cue(&project.album, wav_filename, &cue_path) {
            Ok(()) => {
                println!("{} Created: {}", "✓".green(), cue_path.display());
            }
            Err(e) => {
                println!("{} Failed to generate CUE: {}", "✗".red(), e);
                return;
            }
        }

        // 3. Generate TOC file
        match crate::export::toc::generate_toc(&project.album, wav_filename, &toc_path) {
            Ok(()) => {
                println!("{} Created: {}", "✓".green(), toc_path.display());
            }
            Err(e) => {
                println!("{} Failed to generate TOC: {}", "✗".red(), e);
                return;
            }
        }
    } else {
        // Multi-file mode - just generate CUE referencing original files
        match crate::export::cue::generate_cue_multi(&project.album, &cue_path) {
            Ok(()) => {
                println!("{} Created: {}", "✓".green(), cue_path.display());
            }
            Err(e) => {
                println!("{} Failed to generate CUE: {}", "✗".red(), e);
                return;
            }
        }
    }

    // Summary
    println!();
    println!("{}", "Export complete!".bold().green());
    println!();
    println!("Files created:");
    if single_wav_mode {
        println!("  - {} (concatenated audio)", wav_path.display());
    }
    println!("  - {} (CUE sheet with CD-TEXT)", cue_path.display());
    if single_wav_mode {
        println!("  - {} (cdrdao TOC file)", toc_path.display());
    }
    println!();

    if single_wav_mode {
        println!("To burn with cdrdao:");
        println!(
            "  cdrdao write --device /dev/cdrom {}",
            toc_path.display()
        );
    } else {
        println!("Note: Multi-file CUE requires all source WAV files to be present.");
        println!("For CD burning, use 'Single WAV + CUE' format.");
    }
}

/// Burn CD using cdrdao
fn burn_cd(project: &Project) {
    use crate::burn::cdrdao::{self, BurnOptions, Cdrdao, CdDrive};

    println!();
    println!("{}", "Burn CD".bold().green());
    println!("{}", "─".repeat(40).dimmed());
    println!();

    // Check if cdrdao is available
    if !cdrdao::is_available() {
        println!("{} cdrdao is not installed.", "✗".red());
        println!();
        println!("cdrdao is required for CD burning. Install it with:");
        println!("  macOS:  brew install cdrdao");
        println!("  Ubuntu: sudo apt install cdrdao");
        println!("  Fedora: sudo dnf install cdrdao");
        return;
    }

    // Show cdrdao version
    if let Some(version) = cdrdao::version() {
        println!("{} Found: {}", "✓".green(), version);
    }

    // Check for empty project
    if project.album.tracks.is_empty() {
        println!("{} No tracks to burn. Add tracks first.", "✗".red());
        return;
    }

    // Validate project
    match project.album.validate() {
        Ok(()) => {
            println!("{} Project is Red Book compliant", "✓".green());
        }
        Err(e) => {
            println!("{} Validation error: {}", "✗".red(), e);
            println!("Please fix the issues before burning.");
            return;
        }
    }

    // Check if we have an exported TOC file
    let toc_path = project
        .file_path
        .as_ref()
        .and_then(|p| p.parent())
        .map(|dir| dir.join(format!("{}.toc", project.name())))
        .filter(|p| p.exists());

    let toc_file = match toc_path {
        Some(path) => {
            println!("{} Found TOC file: {}", "✓".green(), path.display());
            path
        }
        None => {
            println!("{} No TOC file found.", "⚠".yellow());
            println!("Please export the master first using 'Export master' -> 'Single WAV + CUE'.");
            println!();

            let export_now = Confirm::new("Export master now?")
                .with_default(true)
                .prompt();

            if matches!(export_now, Ok(true)) {
                export_master(project);

                // Check again for TOC file
                let new_toc = project
                    .file_path
                    .as_ref()
                    .and_then(|p| p.parent())
                    .map(|dir| dir.join(format!("{}.toc", project.name())))
                    .filter(|p| p.exists());

                match new_toc {
                    Some(path) => path,
                    None => {
                        println!("{} Export cancelled or failed.", "✗".red());
                        return;
                    }
                }
            } else {
                return;
            }
        }
    };

    println!();

    // List available CD drives
    println!("Scanning for CD drives...");
    let drives = match cdrdao::list_drives() {
        Ok(d) if !d.is_empty() => d,
        Ok(_) => {
            println!("{} No CD drives found.", "✗".red());
            println!("Make sure a CD burner is connected and recognized by the system.");
            return;
        }
        Err(e) => {
            println!("{} Failed to scan drives: {}", "⚠".yellow(), e);
            println!("You can manually specify the device path.");

            // Allow manual device entry
            let device = match Text::new("CD drive device path:")
                .with_default("/dev/sr0")
                .with_help_message("e.g., /dev/sr0, /dev/cdrom, or SCSI address like 0,0,0")
                .prompt()
            {
                Ok(d) => d.trim().to_string(),
                Err(_) => return,
            };

            vec![CdDrive {
                device,
                vendor: "Unknown".to_string(),
                model: "Manual entry".to_string(),
            }]
        }
    };

    // Select drive
    let drive = if drives.len() == 1 {
        println!("{} Using drive: {} {}", "✓".green(), drives[0].vendor, drives[0].model);
        &drives[0]
    } else {
        let drive_options: Vec<String> = drives
            .iter()
            .map(|d| format!("{} - {} {}", d.device, d.vendor, d.model))
            .collect();

        let selection = match Select::new("Select CD drive:", drive_options.clone()).prompt() {
            Ok(s) => s,
            Err(_) => return,
        };

        let idx = drive_options.iter().position(|o| o == &selection).unwrap_or(0);
        &drives[idx]
    };

    // Select burn speed
    let speed_options = vec![
        "Auto (let drive decide)",
        "1x",
        "2x",
        "4x",
        "8x",
        "16x",
        "24x",
        "48x",
    ];

    let speed_selection = match Select::new("Burn speed:", speed_options)
        .with_help_message("Lower speeds are more reliable")
        .prompt()
    {
        Ok(s) => s,
        Err(_) => return,
    };

    let speed: u32 = match speed_selection {
        "Auto (let drive decide)" => 0,
        s => s.trim_end_matches('x').parse().unwrap_or(0),
    };

    // Ask about simulation
    let simulate_first = match Confirm::new("Simulate burn first (dry run)?")
        .with_default(true)
        .with_help_message("Recommended to test before actual burning")
        .prompt()
    {
        Ok(s) => s,
        Err(_) => return,
    };

    // Create burn options
    let options = BurnOptions {
        device: drive.device.clone(),
        speed,
        simulate: false,
        eject: true,
        force_raw_driver: true,
    };

    let burner = Cdrdao::new(options);

    // Simulation
    if simulate_first {
        println!();
        println!("{}", "Simulating burn...".bold());
        println!("This will test the burn process without writing to the disc.");
        println!();

        match burner.simulate(&toc_file) {
            Ok(()) => {
                println!();
                println!("{} Simulation successful!", "✓".green().bold());
            }
            Err(e) => {
                println!();
                println!("{} Simulation failed: {}", "✗".red(), e);
                println!("Please check your disc and try again.");
                return;
            }
        }

        // Confirm actual burn
        println!();
        let proceed = match Confirm::new("Proceed with actual burn?")
            .with_default(true)
            .prompt()
        {
            Ok(p) => p,
            Err(_) => return,
        };

        if !proceed {
            println!("Burn cancelled.");
            return;
        }
    }

    // Insert disc reminder
    println!();
    println!("{}", "Ready to Burn".bold().yellow());
    println!();
    println!("Make sure you have:");
    println!("  - A blank CD-R or CD-RW disc inserted");
    println!("  - Enough space for {} of audio", project.album.format_duration());
    println!();

    let ready = match Confirm::new("Ready to burn?")
        .with_default(true)
        .prompt()
    {
        Ok(r) => r,
        Err(_) => return,
    };

    if !ready {
        println!("Burn cancelled.");
        return;
    }

    // Actual burn
    println!();
    println!("{}", "Burning CD...".bold());
    println!("This may take several minutes. Do not eject the disc!");
    println!();

    match burner.burn(&toc_file) {
        Ok(()) => {
            println!();
            println!("{}", "═".repeat(40).green());
            println!("{}", "  CD burned successfully!".bold().green());
            println!("{}", "═".repeat(40).green());
            println!();
            println!("Your Red Book audio CD is ready.");
            println!("Album: {}", project.album.title);
            println!("Tracks: {}", project.album.track_count());
            println!("Duration: {}", project.album.format_duration());
        }
        Err(e) => {
            println!();
            println!("{} Burn failed: {}", "✗".red().bold(), e);
            println!();
            println!("Possible causes:");
            println!("  - Disc is not blank or is damaged");
            println!("  - Drive does not support the requested speed");
            println!("  - Insufficient buffer or system resources");
            println!("  - Disc was ejected during burn");
        }
    }
}

/// Validate project
fn validate_project(project: &Project) {
    println!();
    println!("{}", "Validating Project".bold().green());
    println!("{}", "─".repeat(40).dimmed());
    println!();

    match project.album.validate() {
        Ok(()) => {
            println!("{} All checks passed!", "✓".green().bold());
            println!();
            println!("Summary:");
            println!("  Tracks: {} (max 99)", project.album.track_count());
            println!("  Duration: {} (max 79:57)", project.album.format_duration());

            let compliant_tracks = project
                .album
                .tracks
                .iter()
                .filter(|t| t.duration >= crate::core::track::MIN_TRACK_DURATION)
                .count();

            println!("  Track duration: {}/{} tracks >= 4 seconds", compliant_tracks, project.album.track_count());

            if project.album.catalog.is_some() {
                println!("  Catalog (MCN): {}", "✓".green());
            }

            let isrc_count = project.album.tracks.iter().filter(|t| t.isrc.is_some()).count();
            println!("  ISRC codes: {}/{} tracks", isrc_count, project.album.track_count());
        }
        Err(e) => {
            println!("{} Validation failed: {}", "✗".red(), e);
        }
    }
}

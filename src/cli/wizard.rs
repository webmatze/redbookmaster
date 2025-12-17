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
                            println!("{} Auto-conversion not yet implemented", "⚠".yellow());
                            // TODO: Implement conversion
                            continue;
                        }
                        Ok("Skip this file") => continue,
                        Ok("Abort") | Err(_) => break,
                        _ => continue,
                    }
                }

                project.album.add_track(track);
                println!(
                    "{} Added: {} ({})",
                    "✓".green(),
                    title,
                    crate::core::track::format_duration_ms(info.duration)
                );
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

/// Play preview (placeholder)
fn play_preview(project: &Project) {
    if project.album.tracks.is_empty() {
        println!("No tracks to play.");
        return;
    }

    println!();
    println!("{}", "Play Preview".bold().green());
    println!("{}", "─".repeat(40).dimmed());
    println!();
    println!("{} Audio playback not yet implemented", "⚠".yellow());
    println!("Tracks in this project:");

    for track in &project.album.tracks {
        println!(
            "  {}. {} - {:?}",
            track.number, track.title, track.source_file
        );
    }
}

/// Export master (placeholder)
fn export_master(project: &Project) {
    println!();
    println!("{}", "Export Master".bold().green());
    println!("{}", "─".repeat(40).dimmed());
    println!();

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

    println!("{} Export not yet implemented", "⚠".yellow());
    println!();
    println!("Would export:");
    println!("  - {}.cue (CUE sheet with CD-TEXT)", project.name());
    println!("  - {}.wav (concatenated audio)", project.name());
    println!("  - {}.toc (cdrdao TOC file)", project.name());
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

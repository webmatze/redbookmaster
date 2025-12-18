//! Red Book Master Library
//!
//! Core functionality for creating Red Book compatible CD masters.
//! This library provides:
//! - Album and track data structures
//! - WAV file reading, validation, and conversion
//! - Audio concatenation for master creation
//! - CUE and TOC file export
//! - CD burning via cdrdao

pub mod core;
pub mod audio;
pub mod export;
pub mod burn;

// Re-export commonly used types
pub use core::{Album, Track, Project, CdText, Isrc, Mcn};
pub use audio::{WavInfo, read_wav_info, Player, PlayerError};
pub use audio::{convert_to_red_book, convert_wav, ConvertOptions, ConversionResult, ConvertError};
pub use audio::{WaveformData, extract_peaks, peaks_to_svg_path, WaveformError};
pub use export::cue::{generate_cue, generate_cue_multi, CueError};
pub use export::toc::{generate_toc, TocError};
pub use burn::cdrdao::{Cdrdao, BurnOptions, CdDrive, CdrdaoError, find_cdrdao, is_available as cdrdao_available};

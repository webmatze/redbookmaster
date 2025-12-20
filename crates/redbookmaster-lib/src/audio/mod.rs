pub mod wav;
pub mod player;
pub mod concat;
pub mod convert;
pub mod waveform;

pub use wav::{WavInfo, read_wav_info};
pub use player::{Player, PlayerError};
pub use convert::{convert_to_red_book, convert_wav, ConvertOptions, ConversionResult, ConvertError};
pub use waveform::{WaveformData, extract_peaks, peaks_to_svg_path, WaveformError};

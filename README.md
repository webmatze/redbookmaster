# Red Book Master

A CLI tool for creating Red Book compatible CD masters with CUE sheet output.

## Features

- **Interactive wizard** - Guided step-by-step experience for creating CD masters
- **Native file browser** - Select WAV files using your system's file dialog
- **Red Book compliant** - Enforces CD-DA specifications (16-bit, 44.1kHz stereo)
- **Automatic format conversion** - Converts non-compliant WAV files (resampling, bit depth, channels)
- **Audio preview** - Play tracks, albums, and track transitions before burning
- **Full metadata support** - CD-TEXT, ISRC codes, MCN/UPC catalog numbers
- **CUE sheet export** - Industry-standard format for CD mastering
- **TOC file export** - Compatible with cdrdao for disc-at-once burning
- **CD burning** - Direct burning via cdrdao with CD-TEXT support
- **Project files** - Save and load projects in .rbm format

## Installation

### Prerequisites

- Rust 1.70 or later
- Audio output device (for playback features)
- cdrdao (optional, for CD burning)

### Build from source

```bash
git clone https://github.com/webmatze/redbookmaster.git
cd redbookmaster
cargo build --release
```

The binary will be at `target/release/redbookmaster`.

### Install via Cargo

```bash
cargo install --path .
```

## Usage

### Interactive Mode (Recommended)

Launch the interactive wizard:

```bash
redbookmaster
```

This opens a guided menu where you can:
- Create new projects
- Add and manage tracks
- Edit metadata (album, track, ISRC, MCN)
- Configure track gaps
- Preview audio playback
- Export master files
- Burn CDs

### Command Line

```bash
# Create a new project
redbookmaster new

# Open an existing project
redbookmaster open my-album.rbm

# Add tracks to current project
redbookmaster add track1.wav track2.wav

# List tracks
redbookmaster list

# Edit track metadata
redbookmaster edit 1

# Reorder tracks
redbookmaster reorder

# Configure track gaps
redbookmaster gaps

# Play album preview
redbookmaster play

# Play specific track
redbookmaster play 3

# Play transition between tracks
redbookmaster transition 2

# Validate Red Book compliance
redbookmaster validate

# Export CUE/WAV master
redbookmaster export

# Burn to CD (requires cdrdao)
redbookmaster burn

# Show project info
redbookmaster info
```

## Workflow Example

```
$ redbookmaster

Welcome to Red Book Master!

? What would you like to do?
> Create new project

? Album title: My Awesome Album
? Artist/Performer: The Band
? Add WAV files now? Yes
? WAV file path: /path/to/track1.wav
✓ Added: Track 1 (3:42)
? WAV file path: /path/to/track2.wav
⚠ File is not Red Book compliant: Sample rate is 48000Hz (need 44100Hz)
? What would you like to do?
> Convert automatically
⟳ Converting to Red Book format...
✓ Conversion complete: 48000Hz -> 44100Hz
✓ Added: Track 2 (4:15)

? Save project as: my-awesome-album.rbm
✓ Project saved

# Later...
? Choose an action:
> Export master

? Export format:
> Single WAV + CUE (recommended for burning)

? Output directory: ./master
✓ Created: ./master/my-awesome-album.wav
✓ Created: ./master/my-awesome-album.cue
✓ Created: ./master/my-awesome-album.toc
✓ Project saved with export location

? Choose an action:
> Burn CD

✓ Found exported master:
  TOC: ./master/my-awesome-album.toc
  WAV: ./master/my-awesome-album.wav
✓ Found: cdrdao version 1.2.5
...
```

## Audio Format Conversion

Red Book Master automatically handles non-compliant WAV files:

| Input Format | Conversion |
|--------------|------------|
| 48kHz, 96kHz, etc. | Resampled to 44.1kHz (high-quality FFT resampling) |
| 24-bit, 32-bit | Converted to 16-bit with TPDF dithering |
| Mono | Duplicated to stereo |
| 32-bit float | Converted to 16-bit integer |

When adding a non-compliant file, you'll be prompted to:
- **Convert automatically** - Creates a Red Book compliant copy
- **Skip this file** - Continue without adding
- **Abort** - Stop adding tracks

## Red Book Specifications

The tool enforces the following CD-DA (Red Book) specifications:

| Specification | Requirement |
|---------------|-------------|
| Audio format | 16-bit, 44.1kHz, stereo PCM |
| Maximum tracks | 99 |
| Maximum duration | 79:57 |
| Minimum track duration | 4 seconds |
| Track 1 pregap | 2 seconds (default) |
| ISRC format | CC-XXX-YY-NNNNN (country-registrant-year-designation) |
| MCN format | 13 digits (UPC/EAN with check digit) |

## Project Structure

Projects are saved as `.rbm` files (JSON format) containing:

- Album metadata (title, performer, songwriter, catalog number)
- Track list with individual metadata (title, performer, ISRC)
- Gap configurations (pregap, postgap per track)
- Export directory location (remembered for burning)
- CD-TEXT information

## Output Formats

### Single WAV + CUE (Recommended)

Concatenates all tracks into a single WAV file with proper gaps, plus:
- `.cue` - CUE sheet with CD-TEXT
- `.toc` - cdrdao TOC file for burning

Best for CD burning and replication.

### Multi-file CUE

Generates a CUE sheet referencing original WAV files. Useful for software playback but requires all source files to be present.

## External Dependencies

### cdrdao (for CD burning)

cdrdao is required only for the "Burn CD" feature.

```bash
# macOS (Homebrew)
brew install cdrdao

# Ubuntu/Debian
sudo apt install cdrdao

# Fedora
sudo dnf install cdrdao

# Arch Linux
sudo pacman -S cdrdao
```

Red Book Master automatically searches for cdrdao in:
- System PATH
- `/opt/homebrew/bin/cdrdao` (Homebrew on Apple Silicon)
- `/usr/local/bin/cdrdao` (Homebrew on Intel Mac)
- `/usr/bin/cdrdao` (Linux)

## Rust Dependencies

| Crate | Purpose |
|-------|---------|
| clap | Command-line argument parsing |
| inquire | Interactive prompts and menus |
| hound | WAV file reading and writing |
| rodio | Audio playback |
| rubato | High-quality audio resampling |
| rfd | Native file dialogs |
| serde / serde_json | Project file serialization |
| colored | Terminal colors |
| thiserror | Error handling |
| chrono | Timestamps |
| indicatif | Progress bars |
| regex | Filename pattern matching |

## Playback Controls

During audio preview:

| Key | Action |
|-----|--------|
| Space | Pause/Resume |
| `q` | Stop playback |
| `n` | Next track (album mode) |
| `+` / `-` | Volume up/down |

## Tips

1. **Use lower burn speeds** - 4x or 8x is recommended for audio CDs
2. **Simulate first** - Always simulate burns before actual writing
3. **Check track gaps** - Default is 2s for track 1, 0 for others
4. **Preview transitions** - Use transition preview to check gaps between tracks
5. **Save often** - Projects auto-save after export, but save manually after edits

## Troubleshooting

### "cdrdao is not installed"

Even if cdrdao is in your PATH, the program searches specific locations. Install via Homebrew on macOS or your system package manager on Linux.

### Audio playback not working

Ensure you have a working audio output device. The program uses the system default audio output.

### Conversion taking too long

Large files (especially high sample rates) take longer to convert. The FFT-based resampler prioritizes quality over speed.

## License

MIT

## Contributing

Contributions are welcome! Please feel free to submit issues and pull requests.

## Acknowledgments

- [hound](https://github.com/ruuda/hound) - WAV file handling
- [rodio](https://github.com/RustAudio/rodio) - Audio playback
- [rubato](https://github.com/HEnquist/rubato) - High-quality resampling
- [cdrdao](http://cdrdao.sourceforge.net/) - CD burning

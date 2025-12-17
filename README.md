# Red Book Master

A CLI tool for creating Red Book compatible CD masters with CUE sheet output.

## Features

- **Interactive wizard** - Guided step-by-step experience for creating CD masters
- **Red Book compliant** - Enforces CD-DA specifications (16-bit, 44.1kHz stereo)
- **Full metadata support** - CD-TEXT, ISRC codes, MCN/UPC catalog numbers
- **CUE sheet export** - Industry-standard format for CD mastering
- **TOC file export** - Compatible with cdrdao for disc-at-once burning
- **WAV validation** - Validates audio format compliance before mastering
- **Project files** - Save and load projects in .rbm format

## Installation

```bash
cargo install --path .
```

Or build from source:

```bash
cargo build --release
```

## Usage

### Interactive Mode

Launch the interactive wizard:

```bash
redbookmaster
```

### Command Line

```bash
# Create a new project
redbookmaster new

# Open an existing project
redbookmaster open my-album.rbm

# View project info
redbookmaster info

# Validate Red Book compliance
redbookmaster validate

# Export CUE/WAV master
redbookmaster export

# Burn to CD (requires cdrdao)
redbookmaster burn
```

## Red Book Specifications

The tool enforces the following CD-DA (Red Book) specifications:

- **Audio format**: 16-bit, 44.1kHz, stereo PCM
- **Maximum tracks**: 99
- **Maximum duration**: 79:57
- **Minimum track duration**: 4 seconds
- **Track 1 pregap**: 2 seconds minimum
- **ISRC format**: CC-XXX-YY-NNNNN (12 characters)
- **MCN format**: 13 digits (UPC/EAN)

## Project Structure

Projects are saved as `.rbm` files (JSON format) containing:

- Album metadata (title, performer, songwriter, catalog number)
- Track list with individual metadata
- Gap configurations
- CD-TEXT information

## Output Formats

### CUE Sheet (.cue)

Standard CUE sheet format with CD-TEXT support, compatible with most CD burning software.

### TOC File (.toc)

cdrdao-compatible TOC file for disc-at-once burning with full CD-TEXT and ISRC support.

## External Dependencies

- **cdrdao** (optional) - Required for CD burning functionality

Install cdrdao:

```bash
# macOS
brew install cdrdao

# Ubuntu/Debian
sudo apt install cdrdao

# Fedora
sudo dnf install cdrdao
```

## License

MIT

# lrcphile

A CLI tool for automatically downloading lyrics for your music collection from the LRCLIB database.

## Features

- Fetches synchronized (.lrc) and plain text (.txt) lyrics
- Supports batch processing of directories with progress tracking
- Recursive directory scanning
- Handles instrumental tracks
- Preserves existing lyrics files unless specified otherwise
- Supports common audio formats (MP3, FLAC, WAV, OGG, M4A, AAC, OPUS, WMA, APE, DSF, DFF)

## Installation

Clone the repository:
```bash
git clone https://github.com/khalil-cheddadi/lrcphile.git
```

Build and install:

```bash
cd lrcphile
cargo install --path .
```

## Usage

Process your entire music library (defaults to system music directory):
```bash
lrcphile
```

Process a single audio file:
```bash
lrcphile /path/to/song.mp3
```

Process a specific directory:
```bash
lrcphile /path/to/music/
```

Process recursively and override existing lyrics files:
```bash
lrcphile -r -o
```

Use a different LRCLIB instance:
```bash
lrcphile --url https://my-lrclib.example.com
```

### Options

- `[PATH]`: Path to audio file or directory (defaults to system music directory)
- `-r, --recursive`: Recursively process subdirectories
- `-o, --override`: Override existing lyrics files
- `-u, --url <URL>`: URL for the lyrics database instance (default: https://lrclib.net)

## Requirements

Audio files must have proper metadata (title, artist, album) for lyrics lookup to work.

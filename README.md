# lrcphile

A CLI tool for automatically downloading lyrics for your music collection from the LRCLIB database.

## Features

- Fetches synchronized (.lrc) and plain text (.txt) lyrics
- Supports batch processing of directories
- Recursive directory scanning
- Handles instrumental tracks
- Preserves existing lyrics files (with override option)
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

Process a single audio file:
```bash
lrcphile /path/to/song.mp3
```

Process a directory:
```bash
lrcphile /path/to/music/
```

Process recursively with options:
```bash
lrcphile -r -o /path/to/music/
```

### Options

- `-r, --recursive`: Recursively process subdirectories
- `-o, --override-files`: Override existing lyrics files
- `-s, --silent`: Suppress non-important messages

## Requirements

Audio files must have proper metadata (title, artist, album) for lyrics lookup to work.

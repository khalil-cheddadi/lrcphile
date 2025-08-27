use clap::Parser;
use lofty::{file::AudioFile, prelude::TaggedFileExt, probe::Probe, tag::Accessor};
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "lrcphile")]
#[command(about = "CLI liblrc Client")]
#[command(version = "0.1.0")]
struct Cli {
    /// Path to the audio file or directory
    #[arg(help = "Path to the audio file or directory")]
    path: PathBuf,

    /// Automatically override existing lyrics files without prompting
    #[arg(short, long, help = "Override existing lyrics files")]
    override_files: bool,

    /// Recursively process subdirectories
    #[arg(short, long, help = "Recursively process subdirectories")]
    recursive: bool,
}

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
struct LyricsResponse {
    id: u64,
    #[serde(rename = "trackName")]
    track_name: String,
    #[serde(rename = "artistName")]
    artist_name: String,
    #[serde(rename = "albumName")]
    album_name: String,
    duration: f64,
    instrumental: bool,
    #[serde(rename = "plainLyrics")]
    plain_lyrics: Option<String>,
    #[serde(rename = "syncedLyrics")]
    synced_lyrics: Option<String>,
}

#[derive(Debug)]
struct TrackMetadata {
    track_name: String,
    artist_name: String,
    album_name: String,
    duration: f64,
}

impl std::fmt::Display for TrackMetadata {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let minutes = (self.duration as u32) / 60;
        let seconds = (self.duration as u32) % 60;
        write!(
            f,
            "{} - {} by {} ({}:{:02})",
            self.track_name, self.album_name, self.artist_name, minutes, seconds,
        )
    }
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    if cli.path.is_file() {
        process_file(&cli.path, cli.override_files).await;
    } else if cli.path.is_dir() {
        process_directory(&cli.path, cli.override_files, cli.recursive).await;
    } else {
        eprintln!(
            "Path does not exist or is not a file or directory: {}",
            cli.path.display()
        );
        std::process::exit(1);
    }
}

async fn process_directory(dir_path: &PathBuf, override_files: bool, recursive: bool) {
    match scan_directory(dir_path) {
        Ok((audio_files, subdirs)) => {
            // Process audio files in current directory
            if !audio_files.is_empty() {
                for file_path in audio_files {
                    println!("Processing: {}", file_path.display());
                    process_file(&file_path, override_files).await;
                }
            }

            // If recursive, process subdirectories
            if recursive {
                for subdir in subdirs {
                    println!("\nEntering directory: {}", subdir.display());
                    Box::pin(process_directory(&subdir, override_files, recursive)).await;
                }
            }
        }
        Err(e) => {
            eprintln!("Error reading directory {}: {}", dir_path.display(), e);
        }
    }
}

async fn process_file(file_path: &PathBuf, override_files: bool) {
    let metadata_result = read_metadata(file_path).await;
    match metadata_result {
        Ok(metadata) => {
            // Check if lyrics files already exist
            println!("Found: {}", metadata);
            let is_instrumental;
            let lrc_exists = match get_lyrics_file_path(file_path, "lrc") {
                Ok(path) => {
                    is_instrumental = is_instrumental_lrc_file(&path);
                    path.exists()
                }
                Err(e) => {
                    eprintln!("Error determining LRC file path: {}", e);
                    return;
                }
            };
            let txt_exists = match get_lyrics_file_path(file_path, "txt") {
                Ok(path) => path.exists(),
                Err(e) => {
                    eprintln!("Error determining TXT file path: {}", e);
                    return;
                }
            };

            let should_fetch = if is_instrumental {
                false
            } else if lrc_exists || txt_exists {
                override_files
            } else {
                true
            };

            if should_fetch {
                match fetch_lyrics(&metadata).await {
                    Ok(lyrics_result) => {
                        let header = generate_header(
                            &metadata.track_name,
                            &metadata.artist_name,
                            &metadata.album_name,
                            metadata.duration,
                        );
                        if lyrics_result.instrumental {
                            // Create LRC file with instrumental tag to avoid refetching
                            let instrumental_lrc = format!("{}\n[instrumental]", header);
                            match save_lyrics_file(file_path, &instrumental_lrc, "lrc") {
                                Ok(_) => {
                                    println!("Marked as Instrumental");
                                }
                                Err(e) => {
                                    println!("Failed to save instrumental LRC file: {}", e);
                                }
                            }
                        } else if let Some(synced_lyrics) = &lyrics_result.synced_lyrics {
                            // Save synced lyrics to a .lrc file
                            let lrc_with_header = format!("{}\n{}", header, synced_lyrics);
                            match save_lyrics_file(file_path, &lrc_with_header, "lrc") {
                                Ok(lrc_path) => {
                                    println!("Saved synced lyrics to: {}", lrc_path.display());
                                }
                                Err(e) => {
                                    println!("Failed to save LRC file: {}", e);
                                }
                            }
                        } else if let Some(plain_lyrics) = &lyrics_result.plain_lyrics {
                            // Only save plain lyrics to a .txt file
                            let txt_with_header = format!("{}\n{}", header, plain_lyrics);
                            match save_lyrics_file(file_path, &txt_with_header, "txt") {
                                Ok(txt_path) => {
                                    println!("Saved plain lyrics to: {}", txt_path.display());
                                }
                                Err(e) => {
                                    println!("Failed to save TXT file: {}", e);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        println!("Failed to fetch lyrics: {}", e);
                    }
                }
            } else {
                println!(
                    "Skipped fetching for {} - keeping existing files",
                    &metadata.track_name
                );
            }
        }
        Err(e) => {
            println!("Error reading metadata for {}: {}", file_path.display(), e);
        }
    }
    println!("");
}

fn scan_directory(
    dir_path: &PathBuf,
) -> Result<(Vec<PathBuf>, Vec<PathBuf>), Box<dyn std::error::Error>> {
    let audio_extensions = [
        "mp3", "flac", "wav", "ogg", "m4a", "aac", "opus", "wma", "ape", "dsf", "dff",
    ];

    let mut audio_files = Vec::new();
    let mut subdirs = Vec::new();

    for entry in fs::read_dir(dir_path)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_file() {
            if let Some(extension) = path.extension() {
                if let Some(ext_str) = extension.to_str() {
                    if audio_extensions.contains(&ext_str.to_lowercase().as_str()) {
                        audio_files.push(path);
                    }
                }
            }
        } else if path.is_dir() {
            subdirs.push(path);
        }
    }

    audio_files.sort();
    subdirs.sort();

    Ok((audio_files, subdirs))
}

async fn read_metadata(file_path: &PathBuf) -> Result<TrackMetadata, Box<dyn std::error::Error>> {
    let tagged_file = Probe::open(file_path)?.read()?;

    // Return metadata for potential lyrics fetching
    if let Some(tag) = tagged_file.primary_tag() {
        let title = tag.title().map(|s| s.to_string());
        let artist = tag.artist().map(|s| s.to_string());
        let album = tag.album().map(|s| s.to_string());
        let duration = tagged_file.properties().duration().as_secs() as f64;

        if let (Some(title), Some(artist), Some(album)) = (title, artist, album) {
            return Ok(TrackMetadata {
                track_name: title,
                artist_name: artist,
                album_name: album,
                duration,
            });
        }
    }

    Err("Missing required metadata (title, artist, or album)".into())
}

async fn fetch_lyrics(
    metadata: &TrackMetadata,
) -> Result<LyricsResponse, Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();

    let url = format!(
        "https://lrclib.net/api/get?track_name={}&artist_name={}&album_name={}&duration={}",
        urlencoding::encode(&metadata.track_name),
        urlencoding::encode(&metadata.artist_name),
        urlencoding::encode(&metadata.album_name),
        metadata.duration,
    );

    let response = client
        .get(&url)
        .header(
            "User-Agent",
            "lrcphile v0.1.0 (https://github.com/basketingballs/lrcphile)",
        )
        .send()
        .await?;

    if response.status().is_success() {
        let lyrics_response: LyricsResponse = response.json().await?;
        Ok(lyrics_response)
    } else if response.status() == 404 {
        Err("Track not found in LRCLIB database".into())
    } else {
        Err(format!("API request failed with status: {}", response.status()).into())
    }
}

fn get_lyrics_file_path(
    audio_file_path: &PathBuf,
    extension: &str,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let audio_dir = audio_file_path
        .parent()
        .ok_or("Could not determine parent directory")?;

    let file_stem = audio_file_path
        .file_stem()
        .ok_or("Could not determine file name")?;

    let mut lyrics_path = audio_dir.to_path_buf();
    lyrics_path.push(format!("{}.{}", file_stem.to_string_lossy(), extension));

    Ok(lyrics_path)
}

fn is_instrumental_lrc_file(lrc_path: &PathBuf) -> bool {
    if let Ok(content) = fs::read_to_string(lrc_path) {
        content.contains("[by: lrcphile]") && content.contains("[instrumental]")
    } else {
        false
    }
}

fn save_lyrics_file(
    audio_file_path: &PathBuf,
    lyrics: &str,
    extension: &str,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let file_path = get_lyrics_file_path(audio_file_path, extension)?;
    // Write the lyrics to the file
    fs::write(&file_path, lyrics)?;
    Ok(file_path)
}

fn generate_header(title: &str, artist: &str, album: &str, duration: f64) -> String {
    let minutes = (duration as u32) / 60;
    let seconds = (duration as u32) % 60;
    let length = format!("{}:{:02}", minutes, seconds);

    let header = format!(
        "[ti: {}]\n[ar: {}]\n[al: {}]\n[au: {}]\n[length: {}]\n[by: lrcphile]",
        title, artist, album, artist, length
    );

    header
}

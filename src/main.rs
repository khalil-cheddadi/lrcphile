use clap::Parser;
use colored::Colorize;
use directories::UserDirs;
use lofty::{file::AudioFile, prelude::TaggedFileExt, probe::Probe, tag::Accessor};
use serde::Deserialize;
use std::{fs, path::PathBuf};

#[derive(Parser)]
#[command(name = "lrcphile")]
#[command(about = "CLI liblrc Client")]
#[command(version = "0.1.0")]
struct Cli {
    /// Path to the audio file or directory (defaults to music directory)
    #[arg(help = "Path to the audio file or directory (defaults to music directory)")]
    path: Option<PathBuf>,

    /// Automatically override existing lyrics files without prompting
    #[arg(short, long = "override", help = "Override existing lyrics files")]
    override_files: bool,

    /// Recursively process subdirectories
    #[arg(short, long, help = "Recursively process subdirectories")]
    recursive: bool,

    /// Suppress output messages
    #[arg(short, long, help = "Suppress none important messages")]
    silent: bool,

    /// URL for lyrics database instance
    #[arg(
        short,
        long,
        default_value = "https://lrclib.net",
        help = "URL for the lyrics database instance (e.g., self-hosted LRCLIB)"
    )]
    url: String,
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

impl LyricsResponse {
    fn generate_header(&self) -> String {
        let minutes = (self.duration as u32) / 60;
        let seconds = (self.duration as u32) % 60;
        let length = format!("{}:{:02}", minutes, seconds);

        format!(
            "[ti: {}]\n[ar: {}]\n[al: {}]\n[length: {}]\n[by: lrcphile]",
            self.track_name, self.artist_name, self.album_name, length
        )
    }
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
            "{} - {} by {} ({})",
            self.track_name.bright_white().bold(),
            self.album_name.bright_cyan(),
            self.artist_name.bright_yellow(),
            format!("{}:{:02}", minutes, seconds).bright_magenta()
        )
    }
}

impl TrackMetadata {
    async fn fetch_lyrics(
        self,
        url: &str,
    ) -> Result<Option<LyricsResponse>, Box<dyn std::error::Error>> {
        let client = reqwest::Client::new();

        let api_url = format!(
            "{}/api/get?track_name={}&artist_name={}&album_name={}&duration={}",
            url.trim_end_matches('/'),
            urlencoding::encode(&self.track_name),
            urlencoding::encode(&self.artist_name),
            urlencoding::encode(&self.album_name),
            self.duration,
        );

        let response = client
            .get(&api_url)
            .header(
                "User-Agent",
                "lrcphile v0.1.0 (https://github.com/khalil-cheddadi/lrcphile)",
            )
            .send()
            .await?;

        if response.status().is_success() {
            let lyrics_response: LyricsResponse = response.json().await?;
            Ok(Some(lyrics_response))
        } else if response.status() == 404 {
            Ok(None)
        } else {
            Err(format!("API request failed with status: {}", response.status()).into())
        }
    }
}

#[tokio::main]
async fn main() {
    let args = Cli::parse();

    let path = match &args.path {
        Some(p) => p.clone(),
        None => UserDirs::new()
            .expect("Failed to get user directories")
            .audio_dir()
            .unwrap()
            .to_path_buf(),
    };

    if path.is_file() {
        process_file(&path, &args).await;
    } else if path.is_dir() {
        process_directory(&path, &args).await;
    } else {
        eprintln!(
            "{} {}",
            "Error:".red().bold(),
            format!(
                "Path does not exist or is not a file or directory: {}",
                path.display()
            )
            .red()
        );
        std::process::exit(1);
    }
}

async fn process_directory(dir_path: &PathBuf, args: &Cli) {
    match scan_directory(&dir_path) {
        Ok((audio_files, subdirs)) => {
            // Process audio files in current directory
            if !audio_files.is_empty() {
                for file_path in audio_files {
                    process_file(&file_path, args).await;
                }
            }
            // If recursive, process subdirectories
            if args.recursive {
                for subdir in subdirs {
                    Box::pin(process_directory(&subdir, args)).await;
                }
            }
        }
        Err(e) => {
            eprintln!(
                "{} {}",
                "Error:".red().bold(),
                format!("Error reading directory {}: {}", dir_path.display(), e).red()
            );
        }
    }
}

async fn process_file(file_path: &PathBuf, args: &Cli) {
    let metadata_result = read_metadata(file_path).await;
    match metadata_result {
        Ok(metadata) => {
            // Check if lyrics files already exist
            let is_instrumental;
            let lrc_exists = match get_lyrics_file_path(file_path, "lrc") {
                Ok(path) => {
                    is_instrumental = is_instrumental_lrc_file(&path);
                    path.exists()
                }
                Err(e) => {
                    eprintln!(
                        "{} {}",
                        "Error:".red().bold(),
                        format!("Error determining LRC file path: {}", e).red()
                    );
                    return;
                }
            };
            let txt_exists = match get_lyrics_file_path(file_path, "txt") {
                Ok(path) => path.exists(),
                Err(e) => {
                    eprintln!(
                        "{} {}",
                        "Error:".red().bold(),
                        format!("Error determining TXT file path: {}", e).red()
                    );
                    return;
                }
            };

            let should_fetch = if is_instrumental {
                false
            } else if lrc_exists || txt_exists {
                args.override_files
            } else {
                true
            };
            if should_fetch || !args.silent {
                print!(
                    "{} {} ",
                    "Found:".green().bold(),
                    metadata.to_string().white()
                );
            }

            if should_fetch {
                match metadata.fetch_lyrics(&args.url).await {
                    Ok(Some(lyrics_result)) => {
                        let header = lyrics_result.generate_header();
                        if lyrics_result.instrumental {
                            // Create LRC file with instrumental tag to avoid refetching
                            let instrumental_lrc = format!("{}\n[instrumental]", header);
                            match save_lyrics_file(file_path, &instrumental_lrc, "lrc") {
                                Ok(_) => {
                                    println!("{}", "Marked as Instrumental".yellow().bold());
                                }
                                Err(e) => {
                                    eprintln!(
                                        "{} {}",
                                        "Failed:".red().bold(),
                                        format!("Failed to save instrumental LRC file: {}", e)
                                            .red()
                                    );
                                }
                            }
                        } else if let Some(synced_lyrics) = &lyrics_result.synced_lyrics {
                            // Save synced lyrics to a .lrc file
                            let lrc_with_header = format!("{}\n{}", header, synced_lyrics);
                            match save_lyrics_file(file_path, &lrc_with_header, "lrc") {
                                Ok(lrc_path) => {
                                    println!(
                                        "{} {}",
                                        "Saved Synced Lyrics to:".green().bold(),
                                        lrc_path.display().to_string().white()
                                    );
                                }
                                Err(e) => {
                                    eprintln!(
                                        "{} {}",
                                        "Failed:".red().bold(),
                                        format!("Failed to save LRC file: {}", e).red()
                                    );
                                }
                            }
                        } else if let Some(plain_lyrics) = &lyrics_result.plain_lyrics {
                            // Only save plain lyrics to a .txt file
                            let txt_with_header = format!("{}\n{}", header, plain_lyrics);
                            match save_lyrics_file(file_path, &txt_with_header, "txt") {
                                Ok(txt_path) => {
                                    println!(
                                        "{} {}",
                                        "Saved Plain Lyrics to:".green().bold(),
                                        txt_path.display().to_string().white()
                                    );
                                }
                                Err(e) => {
                                    eprintln!(
                                        "{} {}",
                                        "Failed:".red().bold(),
                                        format!("Failed to save TXT file: {}", e).red()
                                    );
                                }
                            }
                        }
                    }
                    Ok(None) => {
                        println!("{}", "Track not found in LRCLIB database".red());
                    }
                    Err(e) => {
                        eprintln!(
                            "{} {}",
                            "Failed:".red().bold(),
                            format!("Failed to fetch lyrics: {}", e).red()
                        );
                    }
                }
            } else {
                if !args.silent {
                    println!(
                        "{}",
                        if is_instrumental {
                            "Track Marked as instrumental, Skipping".bold().yellow()
                        } else {
                            "Existing lyrics found, Skipping".bold().yellow()
                        }
                    );
                }
            }
        }
        Err(e) => {
            eprintln!(
                "{} {}",
                "Error:".red().bold(),
                format!("Error reading metadata for {}: {}", file_path.display(), e).red()
            );
        }
    }
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
        let track_name = tag.title().map(|s| s.to_string());
        let artist_name = tag.artist().map(|s| s.to_string());
        let album_name = tag.album().map(|s| s.to_string());
        let duration = tagged_file.properties().duration().as_secs() as f64;

        if let (Some(track_name), Some(artist_name), Some(album_name)) =
            (track_name, artist_name, album_name)
        {
            return Ok(TrackMetadata {
                track_name,
                artist_name,
                album_name,
                duration,
            });
        }
    }

    Err("Missing required metadata (title, artist, or album)".into())
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
    file_path: &PathBuf,
    lyrics: &str,
    extension: &str,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let file_path = get_lyrics_file_path(file_path, extension)?;
    // Write the lyrics to the file
    fs::write(&file_path, lyrics)?;
    Ok(file_path)
}

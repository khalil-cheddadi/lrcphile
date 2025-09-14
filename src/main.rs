use clap::Parser;
use colored::Colorize;
use directories::UserDirs;
use futures::stream::{self, StreamExt};
use indicatif::{ProgressBar, ProgressStyle};
use lofty::{file::AudioFile, prelude::TaggedFileExt, probe::Probe, tag::Accessor};
use serde::Deserialize;
use std::{fs, path::PathBuf, sync::Arc};
use tokio::sync::Mutex;

#[derive(Parser, Clone)]
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

#[derive(Debug, Clone)]
struct ProcessingStats {
    success: usize,
    failed: usize,
    skipped: usize,
    total: usize,
}

impl ProcessingStats {
    fn new(total: usize) -> Self {
        Self {
            success: 0,
            failed: 0,
            skipped: 0,
            total,
        }
    }

    fn increment_success(&mut self) {
        self.success += 1;
    }

    fn increment_failed(&mut self) {
        self.failed += 1;
    }

    fn increment_skipped(&mut self) {
        self.skipped += 1;
    }

    fn display_summary(&self) {
        println!("\n{}", "Processing Summary:".bright_cyan().bold());
        println!(
            "  {} {} {}",
            "Processed:".white(),
            self.total.to_string().bright_white().bold(),
            "files".white()
        );
        println!(
            "  {} {} {}",
            "Successful:".green(),
            self.success.to_string().bright_green().bold(),
            "files".green()
        );
        println!(
            "  {} {} {}",
            "Failed:".red(),
            self.failed.to_string().bright_red().bold(),
            "files".red()
        );
        println!(
            "  {} {} {}",
            "Skipped (existing/instrumental):".yellow(),
            self.skipped.to_string().bright_yellow().bold(),
            "files".yellow()
        );
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
        process_file(&path, &args, None).await;
    } else if path.is_dir() {
        match process_directory(&path, args.recursive) {
            Ok(audio_files) => {
                println!(
                    "{} {}",
                    "Found:".green().bold(),
                    format!("{} audio files", audio_files.len()).bright_cyan()
                );

                if audio_files.len() == 0 {
                    println!("{}", "No audio files found.".yellow());
                    return;
                }

                // Create progress bar
                let progress = ProgressBar::new(audio_files.len() as u64);
                progress.set_style(
                    ProgressStyle::default_bar()
                        .template("[{bar:40}] {pos}/{len} {msg}")
                        .unwrap()
                        .progress_chars("# "),
                );
                progress.set_message("Processing audio files...");

                let stats = Arc::new(Mutex::new(ProcessingStats::new(audio_files.len())));

                // Process files concurrently with a limit of 4
                let concurrent_limit = 4;
                stream::iter(audio_files)
                    .map(|file_path| {
                        let args_clone = args.clone();
                        let progress_clone = progress.clone();
                        let stats_clone = stats.clone();
                        async move {
                            process_file(&file_path, &args_clone, Some(stats_clone)).await;
                            progress_clone.inc(1);
                        }
                    })
                    .buffer_unordered(concurrent_limit)
                    .collect::<Vec<_>>()
                    .await;

                progress.finish_with_message("Processing complete!");

                let final_stats = stats.lock().await;
                final_stats.display_summary();
            }
            Err(e) => {
                eprintln!(
                    "{} {}",
                    "Error:".red().bold(),
                    format!("Error collecting tracks from {}: {}", path.display(), e).red()
                );
                std::process::exit(1);
            }
        }
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

fn process_directory(
    dir_path: &PathBuf,
    recursive: bool,
) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let mut all_tracks = Vec::new();
    let audio_extensions = [
        "mp3", "flac", "wav", "ogg", "m4a", "aac", "opus", "wma", "ape", "dsf", "dff",
    ];
    for entry in fs::read_dir(dir_path)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_file() {
            if let Some(extension) = path.extension() {
                if let Some(ext_str) = extension.to_str() {
                    if audio_extensions.contains(&ext_str.to_lowercase().as_str()) {
                        all_tracks.push(path);
                    }
                }
            }
        } else if path.is_dir() && recursive {
            match process_directory(&path, recursive) {
                Ok(sub_tracks) => all_tracks.extend(sub_tracks),
                Err(e) => {
                    eprintln!(
                        "{} {}",
                        "Warning:".yellow().bold(),
                        format!("Error reading subdirectory {}: {}", path.display(), e).yellow()
                    );
                }
            }
        }
    }

    all_tracks.sort();

    Ok(all_tracks)
}

async fn process_file(file_path: &PathBuf, args: &Cli, stats: Option<Arc<Mutex<ProcessingStats>>>) {
    let metadata_result = read_metadata(file_path).await;
    let stats = stats.unwrap_or(Arc::new(Mutex::new(ProcessingStats::new(0))));
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

            if !should_fetch {
                stats.lock().await.increment_skipped();
            } else {
                match metadata.fetch_lyrics(&args.url).await {
                    Ok(Some(lyrics_result)) => {
                        let header = lyrics_result.generate_header();
                        if lyrics_result.instrumental {
                            // Create LRC file with instrumental tag to avoid refetching
                            let instrumental_lrc = format!("{}\n[instrumental]", header);
                            match save_lyrics_file(file_path, &instrumental_lrc, "lrc") {
                                Ok(_) => {
                                    stats.lock().await.increment_success();
                                }
                                Err(e) => {
                                    eprintln!(
                                        "{} {}",
                                        "Failed:".red().bold(),
                                        format!("Failed to save instrumental LRC file: {}", e)
                                            .red()
                                    );
                                    stats.lock().await.increment_failed();
                                }
                            }
                        } else if let Some(synced_lyrics) = &lyrics_result.synced_lyrics {
                            // Save synced lyrics to a .lrc file
                            let lrc_with_header = format!("{}\n{}", header, synced_lyrics);
                            match save_lyrics_file(file_path, &lrc_with_header, "lrc") {
                                Ok(_) => {
                                    stats.lock().await.increment_success();
                                }
                                Err(e) => {
                                    eprintln!(
                                        "{} {}",
                                        "Failed:".red().bold(),
                                        format!("Failed to save LRC file: {}", e).red()
                                    );
                                    stats.lock().await.increment_failed();
                                }
                            }
                        } else if let Some(plain_lyrics) = &lyrics_result.plain_lyrics {
                            // Only save plain lyrics to a .txt file
                            let txt_with_header = format!("{}\n{}", header, plain_lyrics);
                            match save_lyrics_file(file_path, &txt_with_header, "txt") {
                                Ok(_) => {
                                    stats.lock().await.increment_success();
                                }
                                Err(e) => {
                                    eprintln!(
                                        "{} {}",
                                        "Failed:".red().bold(),
                                        format!("Failed to save TXT file: {}", e).red()
                                    );
                                    stats.lock().await.increment_failed();
                                }
                            }
                        }
                    }
                    Ok(None) => {
                        stats.lock().await.increment_failed();
                    }
                    Err(e) => {
                        eprintln!(
                            "{} {}",
                            "Failed:".red().bold(),
                            format!("Failed to fetch lyrics: {}", e).red()
                        );
                        stats.lock().await.increment_failed();
                    }
                }
            }
        }
        Err(_) => {
            stats.lock().await.increment_failed();
        }
    }
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
    // Write the lyrics to the file
    let file_path = get_lyrics_file_path(file_path, extension)?;
    fs::write(&file_path, lyrics)?;
    Ok(file_path)
}

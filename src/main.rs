use clap::{Parser, Subcommand};
use directories::{BaseDirs, ProjectDirs};
use notify_rust::Notification;
use serde_json::Value;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::str::FromStr;
use urlquerystring::StackQueryParams;

const DESKTOP_TEMPLATE: &str = include_str!("../resources/localvid.desktop");

#[derive(Parser)]
#[command(name = "localvid")]
#[command(
    about = "A program which enhances mpv's integration with youtube, designed to be registered as a url scheme on linux."
)]
#[command(args_conflicts_with_subcommands = true)]
struct Cli {
    #[arg(required = true)]
    uri: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    #[command(about = "Create desktop entry and register this program's url scheme.")]
    Init,
}

fn get_info(video: &str) -> Result<Value, Box<dyn std::error::Error>> {
    let out = Command::new("yt-dlp")
        .args(["--dump-json", video])
        .output()?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!("command failed: {}", stderr).into());
    }

    let info: Value = serde_json::from_slice(&out.stdout)?;

    return Ok(info);
}

fn fetch_convert_subs(
    video_dir: &str,
    url: &str,
    mpv_cmd: &mut Command,
) -> Result<(), Box<dyn std::error::Error>> {
    let out = Command::new("yt-dlp")
        .args([
            "--skip-download",
            "--write-subs",
            "--sub-format",
            "srv3",
            "-o",
            "%(title)s.%(ext)s",
            "-P",
            video_dir,
            &url,
        ])
        .output()
        .expect("Something failed.");

    if !out.status.success() {
        // This will tell you EXACTLY why yt-dlp is grumpy
        let stderr = String::from_utf8_lossy(&out.stderr);
        eprintln!("command failed: {}", stderr);
    } else {
        println!("Subtitles downloaded successfully to {:?}", video_dir);
    }

    // find srv3 file in directory
    let entries: Vec<PathBuf> = fs::read_dir(video_dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("srv3"))
        .collect();
    let ass_entries: Vec<PathBuf> = entries.iter().map(|e| e.with_extension("ass")).collect();

    // generate .ass file
    for (i, e) in entries.iter().enumerate() {
        let str = fs::read_to_string(e)?;
        let srv = srv3_ttml::TimedText::from_str(&str)?;
        let ass = srv3tovtt_crate::to_ass(&srv)?;
        fs::write(&ass_entries[i], ass)?;
        fs::remove_file(e)?;
    }

    for path in ass_entries {
        mpv_cmd.arg(format!("--sub-files-append={}", path.to_string_lossy()));
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    register_scheme();
    match cli.command {
        Some(Commands::Init) => {
            if let Err(e) = setup_desktop_entry() {
                eprintln!("Failed to set up desktop entry: {}", e);
            }
            Ok(())
        }
        None => {
            let uri = cli.uri.expect("URI is required when not using 'init'");
            video(&uri)?;
            Ok(())
        }
    }
}

fn video(uri: &str) -> Result<(), Box<dyn std::error::Error>> {
    let params = StackQueryParams::new(uri);
    let video_id: &str = params.get("v").expect("Couldn't find video ID");
    let url: String = format!("https://youtube.com/watch?v={}", video_id);

    if let Some(base_dirs) = BaseDirs::new() {
        base_dirs.executable_dir();
    }

    let proj_dirs = ProjectDirs::from("", "", "localvid").unwrap();

    // get video info
    let vi: Value = get_info(&url)?;

    let video_dir = &format!(
        "{}/{}",
        proj_dirs.cache_dir().display(),
        vi["title"]
            .as_str()
            .expect("somethings fucked with the title")
    );

    let mut mpv_cmd = Command::new("mpv");

    // check for srv3 subtitles
    if vi["subtitles"].as_object().map_or(false, |o| !o.is_empty()) {
        fetch_convert_subs(&video_dir, &url, &mut mpv_cmd)?;
    }
    mpv_cmd.arg(url);

    Notification::new()
        .summary("Localvid launching mpv")
        .body(&format!("Opening {} in MPV for viewing...", vi["title"]))
        .show()?;

    mpv_cmd.status().expect("Failed to execute mpv");
    Ok(())
}

fn register_scheme() {
    Command::new("xdg-mime")
        .args(["default", "localvid.desktop", "x-scheme-handler/localvid"])
        .output()
        .expect("failed to set url handler");
}

fn setup_desktop_entry() -> std::io::Result<()> {
    let base_dirs = BaseDirs::new().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "Home directory not found")
    })?;

    let mut app_dir = base_dirs.data_local_dir().to_path_buf();
    app_dir.push("applications");

    fs::create_dir_all(&app_dir)?;

    let desktop_file_path = app_dir.join("localvid.desktop");

    if !desktop_file_path.exists() {
        let current_exe = env::current_exe()?;
        let exe_str = current_exe.to_str().unwrap_or("");

        let content = DESKTOP_TEMPLATE.replace("EXEC_PATH", exe_str);

        fs::write(desktop_file_path, content)?;
        println!("Successfully installed desktop entry.");
    }

    Ok(())
}

//! Local short-form video studio and storyboard renderer (`create_short_video`).
//!
//! Mirrors Keel Mobile's `ShortVideo` / `ShortVideoMedia` / `ShortVideoDraft` /
//! `ShortVideoTimeline` / `ShortVideoManifest` architecture and upgrades it for
//! Desktop with FFmpeg Ken Burns motion, optional local video B-roll clips,
//! local neural TTS narration (`audio::run_tts`, e.g. `audio.cpp` VoxCPM2 /
//! Qwen3-TTS or Piper), safe-zone caption overlays, and `-14 LUFS` loudness
//! normalization.

use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use tokio::task::AbortHandle;

pub const DEFAULT_VOICE_ID: &str = "local";
pub const MAX_SCENES: usize = 12;
pub const MAX_NARRATION_CHARS: usize = 280;
pub const MAX_OVERLAY_CHARS: usize = 80;
pub const MAX_TITLE_CHARS: usize = 80;
pub const MAX_SCENES_CHARS: usize = 6_000;
pub const MAX_ESTIMATED_SECONDS: f64 = 90.0;
pub const MAX_SCRIPT_DESCRIPTION_CHARS: usize = 600;
pub const MAX_SCRIPT_WORDS_PER_LINE: usize = 24;
pub const CHARS_PER_SECOND: f64 = 14.0;
pub const NARRATION_PAD_SECONDS: f64 = 0.35;
pub const MINIMUM_CLIP_SECONDS: f64 = 1.5;
pub const FRAMES_PER_SECOND: u32 = 30;
pub const MAX_VIDEO_BYTES: usize = 50 * 1024 * 1024;
const ZOOM_END: f64 = 1.12;
const PAN_ZOOM: f64 = 1.10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AspectRatio {
    #[default]
    #[serde(rename = "9:16")]
    Vertical,
    #[serde(rename = "1:1")]
    Square,
    #[serde(rename = "16:9")]
    Landscape,
}

impl AspectRatio {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Vertical => "9:16",
            Self::Square => "1:1",
            Self::Landscape => "16:9",
        }
    }

    pub fn parse(raw: &str) -> Result<Self> {
        match raw.trim() {
            "9:16" => Ok(Self::Vertical),
            "1:1" => Ok(Self::Square),
            "16:9" => Ok(Self::Landscape),
            _ => bail!("aspect_ratio must be 9:16, 1:1, or 16:9."),
        }
    }

    pub fn pixel_size(self) -> (u32, u32) {
        match self {
            Self::Vertical => (1080, 1920),
            Self::Square => (1080, 1080),
            Self::Landscape => (1920, 1080),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CameraMotion {
    #[default]
    None,
    SlowZoom,
    PanLeft,
    PanRight,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CropRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl CameraMotion {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::SlowZoom => "slow_zoom",
            Self::PanLeft => "pan_left",
            Self::PanRight => "pan_right",
        }
    }

    pub fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "none" | "" => Ok(Self::None),
            "slow_zoom" => Ok(Self::SlowZoom),
            "pan_left" => Ok(Self::PanLeft),
            "pan_right" => Ok(Self::PanRight),
            _ => bail!("motion must be none, slow_zoom, pan_left, or pan_right."),
        }
    }

    pub fn ease(t: f64) -> f64 {
        let clamped = t.clamp(0.0, 1.0);
        clamped * clamped * (3.0 - 2.0 * clamped)
    }

    /// Computes the aspect-fill crop rectangle in source pixels at `progress` (0..=1).
    pub fn crop_rect(
        self,
        image_width: f64,
        image_height: f64,
        canvas_width: f64,
        canvas_height: f64,
        progress: f64,
    ) -> CropRect {
        let t = Self::ease(progress);
        let canvas_aspect = canvas_width / canvas_height.max(1.0);
        let mut base_width = image_width;
        let mut base_height = image_width / canvas_aspect;
        if base_height > image_height {
            base_height = image_height;
            base_width = image_height * canvas_aspect;
        }
        let zoom = match self {
            Self::None => 1.0,
            Self::SlowZoom => 1.0 + (ZOOM_END - 1.0) * t,
            Self::PanLeft | Self::PanRight => PAN_ZOOM,
        };
        let width = base_width / zoom;
        let height = base_height / zoom;
        let mut x = (image_width - width) / 2.0;
        let y = (image_height - height) / 2.0;
        if matches!(self, Self::PanLeft | Self::PanRight) {
            let room = (image_width - width).max(0.0);
            let reach = (room / 2.0).min(width * 0.10);
            let (from, to) = if self == Self::PanLeft {
                (x + reach, x - reach)
            } else {
                (x - reach, x + reach)
            };
            x = from + (to - from) * t;
        }
        CropRect {
            x,
            y,
            width,
            height,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoScene {
    pub image_path: String,
    pub narration: String,
    #[serde(default)]
    pub overlay: Option<String>,
    #[serde(default)]
    pub motion: CameraMotion,
}

impl VideoScene {
    pub fn estimated_seconds(&self) -> f64 {
        let spoken = self.narration.chars().count().max(1);
        (spoken as f64 / CHARS_PER_SECOND).max(MINIMUM_CLIP_SECONDS)
    }

    pub fn is_video_clip(&self) -> bool {
        is_video_clip_path(&self.image_path)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoArguments {
    pub path: String,
    pub scenes: Vec<VideoScene>,
    pub voice: String,
    pub aspect_ratio: AspectRatio,
    pub title: Option<String>,
}

impl VideoArguments {
    pub fn estimated_seconds(&self) -> f64 {
        self.scenes.iter().map(VideoScene::estimated_seconds).sum()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderClip {
    pub image_path: String,
    pub narration: String,
    #[serde(default)]
    pub overlay: Option<String>,
    #[serde(default)]
    pub motion: CameraMotion,
    pub duration: f64,
    pub start: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderPlan {
    pub path: String,
    pub title: Option<String>,
    pub voice: String,
    pub aspect_ratio: AspectRatio,
    pub width: u32,
    pub height: u32,
    pub clips: Vec<RenderClip>,
    pub total_seconds: f64,
}

impl RenderPlan {
    pub fn from_args(args: &VideoArguments) -> Self {
        let (width, height) = args.aspect_ratio.pixel_size();
        let mut cursor = 0.0;
        let mut clips = Vec::with_capacity(args.scenes.len());
        for scene in &args.scenes {
            let duration = scene.estimated_seconds();
            clips.push(RenderClip {
                image_path: scene.image_path.clone(),
                narration: scene.narration.clone(),
                overlay: scene.overlay.clone(),
                motion: scene.motion,
                duration,
                start: cursor,
            });
            cursor += duration;
        }
        Self {
            path: args.path.clone(),
            title: args.title.clone(),
            voice: args.voice.clone(),
            aspect_ratio: args.aspect_ratio,
            width,
            height,
            clips,
            total_seconds: cursor,
        }
    }

    pub fn retimed(&self, narration_seconds: &[Option<f64>]) -> Self {
        let mut cursor = 0.0;
        let mut clips = Vec::with_capacity(self.clips.len());
        for (index, clip) in self.clips.iter().enumerate() {
            let spoken = narration_seconds.get(index).copied().flatten();
            let duration = spoken
                .map(|s| (s + NARRATION_PAD_SECONDS).max(MINIMUM_CLIP_SECONDS))
                .unwrap_or(clip.duration);
            clips.push(RenderClip {
                image_path: clip.image_path.clone(),
                narration: clip.narration.clone(),
                overlay: clip.overlay.clone(),
                motion: clip.motion,
                duration,
                start: cursor,
            });
            cursor += duration;
        }
        Self {
            path: self.path.clone(),
            title: self.title.clone(),
            voice: self.voice.clone(),
            aspect_ratio: self.aspect_ratio,
            width: self.width,
            height: self.height,
            clips,
            total_seconds: cursor,
        }
    }

    pub fn frame_count(seconds: f64) -> usize {
        ((seconds * FRAMES_PER_SECOND as f64).round() as isize).max(1) as usize
    }

    /// The scene playing at `time` seconds; the last scene after the end.
    pub fn clip_index_at(&self, time: f64) -> usize {
        if self.clips.is_empty() {
            return 0;
        }
        self.clips
            .iter()
            .rposition(|clip| clip.start <= time + 1e-6)
            .unwrap_or(0)
    }

    /// Jump to the start of this scene, or the previous scene within `restart_window`.
    pub fn previous_start(&self, time: f64, restart_window: f64) -> f64 {
        if self.clips.is_empty() {
            return 0.0;
        }
        let idx = self.clip_index_at(time);
        if time - self.clips[idx].start > restart_window || idx == 0 {
            self.clips[idx].start
        } else {
            self.clips[idx - 1].start
        }
    }

    /// Next scene's start time, or `None` on the last scene.
    pub fn next_start(&self, time: f64) -> Option<f64> {
        let idx = self.clip_index_at(time);
        self.clips.get(idx + 1).map(|c| c.start)
    }

    /// Each scene's share of the timeline (0..=1), for proportional clip-strip widths.
    pub fn clip_fractions(&self) -> Vec<f64> {
        if self.total_seconds <= 0.0 {
            return vec![0.0; self.clips.len()];
        }
        self.clips
            .iter()
            .map(|c| c.duration / self.total_seconds)
            .collect()
    }
}

/// Versioned sidecar saved next to `<video>.mp4` as `<video>.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShortVideoManifest {
    pub version: u32,
    pub path: String,
    pub title: Option<String>,
    pub voice: String,
    pub aspect_ratio: AspectRatio,
    pub scenes: Vec<RenderClip>,
}

impl ShortVideoManifest {
    pub fn from_plan(plan: &RenderPlan) -> Self {
        Self {
            version: 1,
            path: plan.path.clone(),
            title: plan.title.clone(),
            voice: plan.voice.clone(),
            aspect_ratio: plan.aspect_ratio,
            scenes: plan.clips.clone(),
        }
    }

    pub fn to_plan(&self) -> RenderPlan {
        let (width, height) = self.aspect_ratio.pixel_size();
        let total_seconds = self
            .scenes
            .last()
            .map(|c| c.start + c.duration)
            .unwrap_or(0.0);
        RenderPlan {
            path: self.path.clone(),
            title: self.title.clone(),
            voice: self.voice.clone(),
            aspect_ratio: self.aspect_ratio,
            width,
            height,
            clips: self.scenes.clone(),
            total_seconds,
        }
    }

    pub fn storyboard_block(&self) -> String {
        self.scenes
            .iter()
            .map(|scene| {
                let mut lines = vec![
                    format!("image: {}", scene.image_path),
                    format!("say: {}", scene.narration),
                ];
                if let Some(overlay) = &scene.overlay {
                    lines.push(format!("overlay: {overlay}"));
                }
                if scene.motion != CameraMotion::None {
                    lines.push(format!("motion: {}", scene.motion.as_str()));
                }
                lines.join("\n")
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

/// Peak level per bar from audio samples (-1..=1), normalized so the loudest bar is 1.
pub fn waveform_bars(samples: &[f32], bars: usize) -> Vec<f32> {
    let bars = bars.clamp(1, 2_000);
    if samples.is_empty() {
        return vec![0.0; bars];
    }
    let mut peaks = vec![0.0_f32; bars];
    let len = samples.len();
    for (idx, sample) in samples.iter().enumerate() {
        let bar = ((idx * bars) / len).min(bars - 1);
        peaks[bar] = peaks[bar].max(sample.abs());
    }
    let loudest = peaks.iter().copied().fold(0.0_f32, f32::max);
    if loudest <= 0.001 {
        return vec![0.0; bars];
    }
    peaks.into_iter().map(|p| p / loudest).collect()
}

/// One spoken line and optional short on-screen caption generated by the local AI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoScriptLine {
    pub narration: String,
    #[serde(default)]
    pub caption: Option<String>,
}

/// Builds a subject-first prompt asking the local model for one spoken sentence
/// and short caption per chosen photo/clip, plus optional revision instructions.
pub fn build_video_script_prompt(
    description: &str,
    media_paths: &[String],
    previous: &[VideoScriptLine],
    change: Option<&str>,
) -> Result<(String, String)> {
    let count = media_paths.len();
    if count == 0 {
        bail!("Add at least one photo or video clip first.");
    }
    if count > MAX_SCENES {
        bail!("Use up to {MAX_SCENES} scenes.");
    }
    let about = description.split_whitespace().collect::<Vec<_>>().join(" ");
    if about.is_empty() || about.chars().count() > MAX_SCRIPT_DESCRIPTION_CHARS {
        bail!(
            "Describe what the video is about in 1 to {MAX_SCRIPT_DESCRIPTION_CHARS} characters."
        );
    }
    let plural = if count == 1 { "" } else { "s" };
    let system = format!(
        "You write the voice-over for a short video made from {count} photo/clip{plural} shown in order.\n\
The user's sentence says what the video is about: that is its subject and its tone. Every line is about that subject. If the sentence names someone (a person, a pet, a place), say the name.\n\
Match the tone the user asks for: funny if they say funny, warm if warm, a pitch only if they are selling something.\n\
Write exactly {count} line{plural}, numbered 1 to {count}, one per scene, telling one short story: line 1 is a hook that makes people keep watching, the middle lines each add something new, and the last line closes it off.\n\
Each line is one spoken sentence of at most 14 words, in plain, everyday words.\n\
Start every line with different words: no two lines may begin the same way.\n\
Never use buzzwords like \"unlock\", \"revolutionize\", \"seamless\", \"the future of\".\n\
End every line with | and a short on-screen caption of 2 to 4 words.\n\
Write nothing else: no title, no notes, no emoji. Each line looks like this:\n\
1. <one spoken sentence> | <CAPTION>"
    );

    let mut user = format!("The video is about: {about}\n\nScenes in order:\n");
    for (idx, path) in media_paths.iter().enumerate() {
        let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
        user.push_str(&format!("{}. {}\n", idx + 1, name));
    }
    if !previous.is_empty() {
        user.push_str("\nThe lines so far:\n");
        for (idx, line) in previous.iter().enumerate() {
            if let Some(cap) = &line.caption {
                user.push_str(&format!("{}. {} | {}\n", idx + 1, line.narration, cap));
            } else {
                user.push_str(&format!("{}. {}\n", idx + 1, line.narration));
            }
        }
    }
    if let Some(ch) = change {
        let cleaned = ch.split_whitespace().collect::<Vec<_>>().join(" ");
        if !cleaned.is_empty() {
            let capped: String = cleaned.chars().take(MAX_SCRIPT_DESCRIPTION_CHARS).collect();
            user.push_str(&format!(
                "\nChange: {capped}\nWrite all {count} lines again with this change."
            ));
        }
    }
    Ok((system, user.trim_end().to_string()))
}

fn clean_script_piece(raw: &str) -> String {
    raw.trim()
        .trim_matches(['"', '“', '”', '*', '_', ' '])
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Parses numbered `1. <narration> | <CAPTION>` lines from a small model's reply.
pub fn parse_video_script_reply(raw: &str, scene_count: usize) -> Result<Vec<VideoScriptLine>> {
    if scene_count == 0 {
        bail!("Need at least one scene.");
    }
    let body = raw
        .rsplit_once("</think>")
        .map(|(_, after)| after)
        .unwrap_or(raw);
    let mut found: HashMap<usize, VideoScriptLine> = HashMap::new();
    for line in body.lines() {
        let trimmed = line.trim().trim_start_matches(['*', '-', '#', '>', ' ']);
        let digit_len = trimmed.bytes().take_while(|b| b.is_ascii_digit()).count();
        if digit_len == 0 {
            continue;
        }
        let Ok(number) = trimmed[..digit_len].parse::<usize>() else {
            continue;
        };
        if !(1..=scene_count).contains(&number) || found.contains_key(&number) {
            continue;
        }
        let rest = &trimmed[digit_len..];
        let Some(first_mark) = rest.chars().next() else {
            continue;
        };
        if !matches!(first_mark, '.' | ')' | ':') {
            continue;
        }
        let after_mark = rest[first_mark.len_utf8()..].trim();
        let (raw_said, raw_cap) = match after_mark.split_once('|') {
            Some((a, b)) => (a.trim(), Some(b.trim())),
            None => (after_mark, None),
        };
        // Strip accidental one-word labels like "Test: ..." or "Note: ...".
        let said_without_label = match raw_said.split_once(':') {
            Some((prefix, tail))
                if !prefix.contains(' ')
                    && prefix.chars().count() <= 12
                    && !prefix.chars().any(|c| c.is_ascii_digit()) =>
            {
                tail.trim()
            }
            _ => raw_said,
        };
        let spoken = clean_script_piece(said_without_label);
        if spoken.is_empty() {
            continue;
        }
        let words: Vec<&str> = spoken.split(' ').collect();
        let capped_narration = if words.len() > MAX_SCRIPT_WORDS_PER_LINE {
            format!("{}.", words[..MAX_SCRIPT_WORDS_PER_LINE].join(" "))
        } else {
            spoken
        };
        let narration: String = capped_narration.chars().take(MAX_NARRATION_CHARS).collect();
        let caption = raw_cap.and_then(|c| {
            let cleaned = clean_script_piece(c);
            let short = cleaned.split(' ').take(4).collect::<Vec<_>>().join(" ");
            let capped: String = short.chars().take(MAX_OVERLAY_CHARS).collect();
            (!capped.is_empty()).then_some(capped)
        });
        found.insert(number, VideoScriptLine { narration, caption });
    }
    let mut lines = Vec::with_capacity(scene_count);
    for idx in 1..=scene_count {
        let Some(line) = found.remove(&idx) else {
            bail!("The AI did not write a line for every scene. Try again, or describe it more simply.");
        };
        lines.push(line);
    }
    Ok(strip_shared_opener(lines))
}

/// Drops a repeated opening phrase shared by all 3+ lines from lines 2..N.
fn strip_shared_opener(lines: Vec<VideoScriptLine>) -> Vec<VideoScriptLine> {
    if lines.len() < 3 {
        return lines;
    }
    let plain = |w: &str| {
        w.to_ascii_lowercase()
            .trim_matches(|c: char| !c.is_alphanumeric())
            .to_string()
    };
    let first_words: Vec<&str> = lines[0].narration.split(' ').collect();
    let max_shared = 3usize.min(first_words.len().saturating_sub(1));
    let mut shared = 0usize;
    while shared < max_shared {
        let target = plain(first_words[shared]);
        let all_match = lines.iter().all(|l| {
            let w: Vec<&str> = l.narration.split(' ').collect();
            w.len() > shared + 1 && plain(w[shared]) == target
        });
        if all_match {
            shared += 1;
        } else {
            break;
        }
    }
    if !(shared >= 2 || (shared == 1 && first_words[0].ends_with(','))) {
        return lines;
    }
    let mut out = Vec::with_capacity(lines.len());
    out.push(lines[0].clone());
    for line in lines.into_iter().skip(1) {
        let rest = line
            .narration
            .split(' ')
            .skip(shared)
            .collect::<Vec<_>>()
            .join(" ");
        let trimmed = rest.trim_start_matches([',', ';', ':', ' ']);
        let mut chars = trimmed.chars();
        let capitalized = match chars.next() {
            Some(c) => format!("{}{}", c.to_uppercase(), chars.as_str()),
            None => line.narration.clone(),
        };
        out.push(VideoScriptLine {
            narration: capitalized,
            caption: line.caption,
        });
    }
    out
}

/// Pairs generated script lines with workspace media paths and alternating gentle motions.
pub fn script_lines_to_scenes(
    lines: &[VideoScriptLine],
    media_paths: &[String],
) -> Vec<VideoScene> {
    const MOTIONS: [CameraMotion; 3] = [
        CameraMotion::SlowZoom,
        CameraMotion::PanLeft,
        CameraMotion::PanRight,
    ];
    lines
        .iter()
        .zip(media_paths.iter())
        .enumerate()
        .map(|(idx, (line, path))| VideoScene {
            image_path: path.clone(),
            narration: line.narration.clone(),
            overlay: line.caption.clone(),
            motion: MOTIONS[idx % MOTIONS.len()],
        })
        .collect()
}

pub fn script_reply_tokens(scene_count: usize) -> usize {
    64 + 48 * scene_count.max(1)
}

pub fn is_output_path(path: &str) -> bool {
    let trimmed = path.trim();
    if trimmed.len() <= 4
        || trimmed.len() > 160
        || !trimmed.to_ascii_lowercase().ends_with(".mp4")
        || trimmed.starts_with('/')
        || trimmed.contains('\\')
    {
        return false;
    }
    !trimmed
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == "..")
}

pub fn is_video_clip_path(path: &str) -> bool {
    let lower = path.trim().to_ascii_lowercase();
    [".mp4", ".mov", ".webm", ".mkv"]
        .iter()
        .any(|ext| lower.ends_with(ext))
}

pub fn is_media_path(path: &str) -> bool {
    let trimmed = path.trim();
    if trimmed.is_empty()
        || trimmed.len() > 200
        || trimmed.starts_with('/')
        || trimmed.contains('\\')
    {
        return false;
    }
    if trimmed
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    [
        ".png", ".jpg", ".jpeg", ".heic", ".webp", ".mp4", ".mov", ".webm", ".mkv",
    ]
    .iter()
    .any(|ext| lower.ends_with(ext))
}

pub fn parse_voice(raw: &str) -> Result<String> {
    let text = raw.trim();
    if text.is_empty() {
        return Ok(DEFAULT_VOICE_ID.to_string());
    }
    let lower = text.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "system" | "avspeech" | "siri" | "elevenlabs" | "eleven" | "cloud"
    ) {
        bail!("Cloud and system voices are disabled; use a local neural TTS runner.");
    }
    if matches!(
        lower.as_str(),
        "neural" | "kokoro" | "on-device" | "local" | "audiocpp" | "voxcpm2" | "qwen3-tts"
    ) {
        return Ok(lower);
    }
    let id = if let Some((head, tail)) = text.split_once(':') {
        let head_l = head.to_ascii_lowercase();
        let tail_t = tail.trim();
        if !matches!(head_l.as_str(), "neural" | "kokoro" | "audiocpp" | "piper")
            || tail_t.is_empty()
            || tail_t.len() > 64
        {
            bail!("Invalid local voice id `{text}`.");
        }
        tail_t
    } else {
        text
    };
    if !(2..=32).contains(&id.len())
        || !id.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        bail!("Invalid voice id `{text}`.");
    }
    Ok(id.to_string())
}

pub fn parse_scenes(raw: &str) -> Result<Vec<VideoScene>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_SCENES_CHARS {
        bail!("scenes must list 1 to {MAX_SCENES} scenes with image: and say: lines (or a JSON array).");
    }
    let scenes = if trimmed.starts_with('[') {
        parse_json_scenes(trimmed)?
    } else {
        parse_block_scenes(trimmed)?
    };
    if scenes.is_empty() || scenes.len() > MAX_SCENES {
        bail!("Use 1 to {MAX_SCENES} scenes.");
    }
    Ok(scenes)
}

fn parse_block_scenes(raw: &str) -> Result<Vec<VideoScene>> {
    let normalized = raw.replace("\r\n", "\n");
    let chunks: Vec<&str> = normalized
        .split("\n\n")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if chunks.is_empty() {
        bail!("Storyboard must include at least one scene.");
    }
    let mut scenes = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        let mut image: Option<String> = None;
        let mut say: Option<String> = None;
        let mut overlay: Option<String> = None;
        let mut motion = CameraMotion::None;
        for line in chunk.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((raw_key, raw_val)) = line.split_once(':') else {
                bail!("Unrecognized storyboard line `{line}`. Expected `image:`, `say:`, `overlay:`, or `motion:`.");
            };
            let key = raw_key.trim().to_ascii_lowercase();
            let val = raw_val.trim();
            match key.as_str() {
                "image" | "photo" | "picture" | "clip" | "media" => {
                    if val.is_empty() {
                        bail!("Each scene needs an image: or clip: path.");
                    }
                    image = Some(val.to_string());
                }
                "say" | "narration" | "line" | "text" => {
                    if val.is_empty() {
                        bail!("Each say: line must be 1 to {MAX_NARRATION_CHARS} characters.");
                    }
                    say = Some(val.to_string());
                }
                "overlay" | "caption" | "title" => {
                    if !val.is_empty() {
                        overlay = Some(val.to_string());
                    }
                }
                "motion" | "camera" | "kenburns" => {
                    motion = CameraMotion::parse(val)?;
                }
                _ => {
                    bail!("Unknown storyboard field `{key}:`.");
                }
            }
        }
        scenes.push(make_scene(image, say, overlay, motion)?);
    }
    Ok(scenes)
}

fn parse_json_scenes(raw: &str) -> Result<Vec<VideoScene>> {
    let items: Vec<serde_json::Value> =
        serde_json::from_str(raw).context("Invalid JSON scenes array")?;
    if items.is_empty() {
        bail!("JSON scenes array must not be empty.");
    }
    let mut scenes = Vec::with_capacity(items.len());
    for item in items {
        let obj = item
            .as_object()
            .context("Each scene in JSON array must be an object")?;
        let pick = |keys: &[&str]| -> Option<String> {
            for k in keys {
                if let Some(v) = obj.get(*k) {
                    if let Some(s) = v.as_str() {
                        let t = s.trim();
                        if !t.is_empty() {
                            return Some(t.to_string());
                        }
                    }
                }
            }
            None
        };
        let image = pick(&["image", "image_path", "photo", "picture", "clip", "media"]);
        let say = pick(&["say", "narration", "text", "line"]);
        let overlay = pick(&["overlay", "caption", "title"]);
        let motion = match pick(&["motion", "camera"]) {
            Some(m) => CameraMotion::parse(&m)?,
            None => CameraMotion::None,
        };
        scenes.push(make_scene(image, say, overlay, motion)?);
    }
    Ok(scenes)
}

fn make_scene(
    image: Option<String>,
    say: Option<String>,
    overlay: Option<String>,
    motion: CameraMotion,
) -> Result<VideoScene> {
    let image_path = image
        .as_deref()
        .map(str::trim)
        .filter(|s| is_media_path(s))
        .context("Each image: path must be a relative workspace photo (.png/.jpg/.webp) or clip (.mp4/.mov/.webm).")?
        .to_string();
    let narration = say
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let narration_len = narration.chars().count();
    if narration_len == 0 || narration_len > MAX_NARRATION_CHARS {
        bail!("Each say: line must be 1 to {MAX_NARRATION_CHARS} characters.");
    }
    let cleaned_overlay = match overlay {
        Some(o) => {
            let trimmed = o.trim();
            if trimmed.is_empty() {
                None
            } else if trimmed.chars().count() > MAX_OVERLAY_CHARS {
                bail!("Each overlay: line must be at most {MAX_OVERLAY_CHARS} characters.");
            } else {
                Some(trimmed.to_string())
            }
        }
        None => None,
    };
    Ok(VideoScene {
        image_path,
        narration,
        overlay: cleaned_overlay,
        motion,
    })
}

pub fn serialize_storyboard(args: &VideoArguments) -> String {
    let mut lines = Vec::new();
    if let Some(title) = &args.title {
        lines.push(format!("title: {title}"));
    }
    lines.push(format!("voice: {}", args.voice));
    lines.push(format!("aspect_ratio: {}", args.aspect_ratio.as_str()));
    lines.push(String::new());
    for (idx, scene) in args.scenes.iter().enumerate() {
        if idx > 0 {
            lines.push(String::new());
        }
        lines.push(format!("image: {}", scene.image_path));
        lines.push(format!("say: {}", scene.narration));
        if let Some(overlay) = &scene.overlay {
            lines.push(format!("overlay: {overlay}"));
        }
        if scene.motion != CameraMotion::None {
            lines.push(format!("motion: {}", scene.motion.as_str()));
        }
    }
    lines.join("\n")
}

/// Parses a full storyboard document (with optional `title:`, `voice:`, `aspect_ratio:` headers).
pub fn parse_storyboard(
    text: &str,
    path: &str,
    override_voice: Option<&str>,
    override_aspect: Option<&str>,
    override_title: Option<&str>,
) -> Result<VideoArguments> {
    let normalized = text.replace("\r\n", "\n");
    let mut header_title: Option<String> = None;
    let mut header_voice: Option<String> = None;
    let mut header_aspect: Option<String> = None;
    let mut body = Vec::new();
    let mut in_header = true;
    for line in normalized.lines() {
        let trimmed = line.trim();
        if in_header && trimmed.is_empty() {
            continue;
        }
        if in_header {
            if let Some((k, v)) = trimmed.split_once(':') {
                let key = k.trim().to_ascii_lowercase();
                let val = v.trim();
                if !val.is_empty() {
                    match key.as_str() {
                        "title" => {
                            header_title = Some(val.to_string());
                            continue;
                        }
                        "voice" => {
                            header_voice = Some(val.to_string());
                            continue;
                        }
                        "aspect_ratio" => {
                            header_aspect = Some(val.to_string());
                            continue;
                        }
                        _ => {}
                    }
                }
            }
        }
        in_header = false;
        body.push(line);
    }

    let output = path.trim();
    if !is_output_path(output) {
        bail!("path must be a relative .mp4 file name inside the workspace, such as videos/short.mp4.");
    }
    let scenes = parse_scenes(&body.join("\n"))?;
    let aspect = match override_aspect
        .filter(|s| !s.trim().is_empty())
        .or(header_aspect.as_deref())
    {
        Some(raw) => AspectRatio::parse(raw)?,
        None => AspectRatio::Vertical,
    };
    let voice = parse_voice(
        override_voice
            .filter(|s| !s.trim().is_empty())
            .or(header_voice.as_deref())
            .unwrap_or(DEFAULT_VOICE_ID),
    )?;
    let title = override_title
        .filter(|s| !s.trim().is_empty())
        .or(header_title.as_deref())
        .map(str::trim)
        .map(String::from);
    if let Some(ref t) = title {
        if t.chars().count() > MAX_TITLE_CHARS {
            bail!("title must be at most {MAX_TITLE_CHARS} characters.");
        }
    }
    let args = VideoArguments {
        path: output.to_string(),
        scenes,
        voice,
        aspect_ratio: aspect,
        title,
    };
    if args.estimated_seconds() > MAX_ESTIMATED_SECONDS {
        bail!(
            "This storyboard would run longer than {} seconds. Use fewer or shorter scenes.",
            MAX_ESTIMATED_SECONDS as u32
        );
    }
    Ok(args)
}

/// Maps storyboard `image:` paths to real files on disk.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ShortVideoMediaIndex {
    entries: HashMap<String, PathBuf>,
}

impl ShortVideoMediaIndex {
    pub fn register(&mut self, storyboard_path: &str, file_path: PathBuf) {
        self.entries
            .insert(storyboard_path.trim().to_ascii_lowercase(), file_path);
    }

    pub fn resolve(&self, path: &str) -> Result<&Path> {
        if !is_media_path(path) {
            bail!("Invalid scene media path `{path}`.");
        }
        self.entries
            .get(&path.trim().to_ascii_lowercase())
            .map(PathBuf::as_path)
            .with_context(|| format!("Storyboard media `{path}` was not found in the workspace."))
    }

    pub fn missing(&self, scenes: &[VideoScene]) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for scene in scenes {
            let key = scene.image_path.trim().to_ascii_lowercase();
            if !self.entries.contains_key(&key) && seen.insert(key) {
                out.push(scene.image_path.clone());
            }
        }
        out
    }

    /// Generates deduplicated safe storyboard filenames (`IMG_1.HEIC`, `Photo.jpg`, `Photo.jpg` ->
    /// `img_1.heic`, `photo.jpg`, `photo-2.jpg`).
    pub fn photo_names(filenames: &[&str], taken: &HashSet<String>) -> Vec<String> {
        let mut used: HashSet<String> = taken.iter().map(|s| s.to_ascii_lowercase()).collect();
        filenames
            .iter()
            .enumerate()
            .map(|(idx, raw)| {
                let base = Self::storyboard_path_from_filename(raw)
                    .unwrap_or_else(|| format!("photo-{}.jpg", idx + 1));
                let mut candidate = base.clone();
                let mut counter = 2usize;
                while used.contains(&candidate.to_ascii_lowercase()) {
                    if let Some(dot) = base.rfind('.') {
                        candidate = format!("{}-{}{}", &base[..dot], counter, &base[dot..]);
                    } else {
                        candidate = format!("{base}-{counter}");
                    }
                    counter += 1;
                }
                used.insert(candidate.to_ascii_lowercase());
                candidate
            })
            .collect()
    }

    pub fn storyboard_path_from_filename(filename: &str) -> Option<String> {
        let last = filename
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(filename)
            .trim();
        if last.is_empty() {
            return None;
        }
        let (stem, ext) = match last.rfind('.') {
            Some(dot) if dot > 0 => (&last[..dot], last[dot + 1..].to_ascii_lowercase()),
            _ => (last, "jpg".to_string()),
        };
        if !matches!(
            ext.as_str(),
            "png" | "jpg" | "jpeg" | "heic" | "webp" | "mp4" | "mov" | "webm" | "mkv"
        ) {
            return None;
        }
        let mut cleaned: String = stem
            .to_ascii_lowercase()
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        cleaned = cleaned.trim_matches(['-', '.', '_']).to_string();
        if cleaned.is_empty() {
            cleaned = "photo".to_string();
        }
        cleaned.truncate(48);
        let norm_ext = if ext == "jpeg" { "jpg" } else { ext.as_str() };
        let path = format!("{cleaned}.{norm_ext}");
        is_media_path(&path).then_some(path)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoRequest {
    pub path: String,
    pub scenes: String,
    #[serde(default)]
    pub aspect_ratio: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub voice: Option<String>,
    /// When true, use the configured local neural TTS runner (e.g. `audio.cpp` or `Piper`).
    /// If no TTS runner is registered, falls back cleanly to timed silent audio when
    /// `allow_silent_fallback` is true.
    #[serde(default = "default_true")]
    pub use_tts: bool,
    #[serde(default = "default_true")]
    pub allow_silent_fallback: bool,
    #[serde(default)]
    pub music_path: Option<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoJobStatus {
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoJob {
    pub id: String,
    pub status: VideoJobStatus,
    pub request: VideoRequest,
    pub workspace_path: String,
    pub artifact_path: Option<String>,
    pub manifest_path: Option<String>,
    pub storyboard_path: Option<String>,
    pub plan: Option<RenderPlan>,
    pub used_tts: bool,
    pub error: Option<String>,
    pub created_at: u64,
}

struct VideoState {
    jobs: HashMap<String, VideoJob>,
    tasks: HashMap<String, AbortHandle>,
}

#[derive(Clone)]
pub struct VideoService {
    directory: PathBuf,
    data_dir: PathBuf,
    state: Arc<Mutex<VideoState>>,
}

impl VideoService {
    pub fn new(data_dir: &Path) -> Result<Self> {
        let directory = data_dir.join("video-jobs");
        std::fs::create_dir_all(&directory)?;
        let mut jobs = HashMap::new();
        for entry in std::fs::read_dir(&directory)?.flatten() {
            if entry.path().extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(raw) = std::fs::read_to_string(entry.path()) else {
                continue;
            };
            let Ok(mut job) = serde_json::from_str::<VideoJob>(&raw) else {
                continue;
            };
            if job.status == VideoJobStatus::Running {
                job.status = VideoJobStatus::Failed;
                job.error = Some(
                    "Keel closed before this video render finished. You can try again.".into(),
                );
                save_job(&directory, &job)?;
            }
            jobs.insert(job.id.clone(), job);
        }
        Ok(Self {
            directory,
            data_dir: data_dir.to_path_buf(),
            state: Arc::new(Mutex::new(VideoState {
                jobs,
                tasks: HashMap::new(),
            })),
        })
    }

    pub async fn list(&self) -> Vec<VideoJob> {
        let mut jobs: Vec<_> = self.state.lock().await.jobs.values().cloned().collect();
        jobs.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
        jobs
    }

    pub async fn start<F>(
        &self,
        workspace: &Path,
        request: VideoRequest,
        on_change: F,
    ) -> Result<VideoJob>
    where
        F: Fn(VideoJob) + Send + 'static,
    {
        let root = crate::tools::validate_workspace_root(workspace)?;
        let parsed = parse_storyboard(
            &request.scenes,
            &request.path,
            request.voice.as_deref(),
            request.aspect_ratio.as_deref(),
            request.title.as_deref(),
        )?;
        // Validate every referenced media file exists inside the workspace up front.
        for scene in &parsed.scenes {
            let resolved = crate::tools::confine_path(&root, &scene.image_path)?;
            if !resolved.is_file() {
                bail!(
                    "Scene media `{}` was not found in the selected workspace folder.",
                    scene.image_path
                );
            }
        }
        if let Some(ref music) = request.music_path {
            if !music.trim().is_empty() {
                let resolved = crate::tools::confine_path(&root, music)?;
                if !resolved.is_file() {
                    bail!("Background music `{music}` was not found in the selected workspace folder.");
                }
            }
        }

        let initial_plan = RenderPlan::from_args(&parsed);
        let job = VideoJob {
            id: uuid::Uuid::new_v4().to_string(),
            status: VideoJobStatus::Running,
            request: request.clone(),
            workspace_path: root.to_string_lossy().into_owned(),
            artifact_path: None,
            manifest_path: None,
            storyboard_path: None,
            plan: Some(initial_plan),
            used_tts: false,
            error: None,
            created_at: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        };
        let mut state = self.state.lock().await;
        if !state.tasks.is_empty() {
            bail!("A video render is already running. Wait for it or cancel it first.");
        }
        save_job(&self.directory, &job)?;
        state.jobs.insert(job.id.clone(), job.clone());

        let service = self.clone();
        let data_dir = self.data_dir.clone();
        let mut running = job.clone();
        let handle = tokio::spawn(async move {
            let outcome = render_short_video(&root, &data_dir, &running.request).await;
            let mut state = service.state.lock().await;
            if state.jobs.get(&running.id).map(|j| &j.status) != Some(&VideoJobStatus::Running) {
                return;
            }
            match outcome {
                Ok(rendered) => {
                    running.artifact_path = Some(rendered.artifact_path);
                    running.manifest_path = Some(rendered.manifest_path);
                    running.storyboard_path = Some(rendered.storyboard_path);
                    running.plan = Some(rendered.plan);
                    running.used_tts = rendered.used_tts;
                    running.status = VideoJobStatus::Succeeded;
                }
                Err(error) => {
                    running.status = VideoJobStatus::Failed;
                    running.error = Some(error.to_string());
                }
            }
            if let Err(error) = save_job(&service.directory, &running) {
                running.error = Some(format!("Could not save video job history: {error}"));
            }
            state.tasks.remove(&running.id);
            state.jobs.insert(running.id.clone(), running.clone());
            on_change(running);
        });
        state.tasks.insert(job.id.clone(), handle.abort_handle());
        Ok(job)
    }

    pub async fn cancel(&self, id: &str) -> Result<VideoJob> {
        let mut state = self.state.lock().await;
        let mut job = state.jobs.get(id).context("Unknown video job")?.clone();
        if job.status == VideoJobStatus::Running {
            if let Some(handle) = state.tasks.remove(id) {
                handle.abort();
            }
            job.status = VideoJobStatus::Cancelled;
            job.error = Some("Video render cancelled.".into());
            save_job(&self.directory, &job)?;
            state.jobs.insert(id.to_string(), job.clone());
        }
        Ok(job)
    }
}

fn save_job(directory: &Path, job: &VideoJob) -> Result<()> {
    let path = directory.join(format!("{}.json", job.id));
    crate::projects::write_json(&path, job)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderedVideo {
    pub artifact_path: String,
    pub manifest_path: String,
    pub storyboard_path: String,
    pub plan: RenderPlan,
    pub used_tts: bool,
}

pub fn detect_ffmpeg_binary() -> Option<String> {
    let bin_name = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(bin_name);
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().into_owned());
            }
        }
    }
    None
}

/// Reads WAV duration in seconds from standard RIFF/WAVE headers without needing ffprobe.
pub fn wav_duration_seconds(path: &Path) -> Result<f64> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("Failed to read WAV file {}", path.display()))?;
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        bail!("Not a valid WAV file: {}", path.display());
    }
    let mut offset = 12usize;
    let mut byte_rate: Option<u32> = None;
    let mut data_size: Option<u32> = None;
    while offset + 8 <= bytes.len() {
        let chunk_id = &bytes[offset..offset + 4];
        let chunk_len = u32::from_le_bytes([
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]) as usize;
        let payload_start = offset + 8;
        if chunk_id == b"fmt " && payload_start + 12 <= bytes.len() {
            byte_rate = Some(u32::from_le_bytes([
                bytes[payload_start + 8],
                bytes[payload_start + 9],
                bytes[payload_start + 10],
                bytes[payload_start + 11],
            ]));
        } else if chunk_id == b"data" {
            data_size = Some(chunk_len as u32);
            break;
        }
        offset = payload_start.saturating_add(chunk_len + (chunk_len % 2));
    }
    let br = byte_rate
        .filter(|&b| b > 0)
        .context("WAV missing fmt byte rate")?;
    let ds = data_size.context("WAV missing data chunk")?;
    Ok(ds as f64 / br as f64)
}

/// Escapes text safely for FFmpeg `drawtext=text='...'`.
pub fn escape_drawtext(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push('\u{2019}'),
            ':' => out.push_str("\\:"),
            '%' => out.push_str("\\%"),
            '\n' | '\r' => out.push(' '),
            other => out.push(other),
        }
    }
    out
}

fn find_system_font() -> Option<PathBuf> {
    let candidates = [
        "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
        "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
        "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
        "/System/Library/Fonts/Helvetica.ttc",
        "C:\\Windows\\Fonts\\arialbd.ttf",
    ];
    candidates.iter().map(PathBuf::from).find(|p| p.is_file())
}

/// Builds the FFmpeg `-vf` filter chain for one scene (cover crop + Ken Burns motion + safe-zone overlay).
pub fn ffmpeg_scene_filter(
    clip: &RenderClip,
    width: u32,
    height: u32,
    is_video_clip: bool,
    font_path: Option<&Path>,
) -> String {
    let frames = RenderPlan::frame_count(clip.duration);
    let base = if is_video_clip {
        format!(
            "scale={width}:{height}:force_original_aspect_ratio=increase,crop={width}:{height},fps={FRAMES_PER_SECOND},setsar=1"
        )
    } else {
        // Supersample to 2x before zoompan to keep slow pans/zooms silky smooth.
        let sw = width * 2;
        let sh = height * 2;
        let last_f = frames.saturating_sub(1).max(1);
        let zoompan = match clip.motion {
            CameraMotion::None => format!(
                "zoompan=z=1.0:x='iw/2-(iw/zoom/2)':y='ih/2-(ih/zoom/2)':d={frames}:s={width}x{height}:fps={FRAMES_PER_SECOND}"
            ),
            CameraMotion::SlowZoom => format!(
                "zoompan=z='1.0+0.12*(on/{last_f})':x='iw/2-(iw/zoom/2)':y='ih/2-(ih/zoom/2)':d={frames}:s={width}x{height}:fps={FRAMES_PER_SECOND}"
            ),
            CameraMotion::PanLeft => format!(
                "zoompan=z=1.10:x='(iw-iw/zoom)*(1-on/{last_f})':y='(ih-ih/zoom)/2':d={frames}:s={width}x{height}:fps={FRAMES_PER_SECOND}"
            ),
            CameraMotion::PanRight => format!(
                "zoompan=z=1.10:x='(iw-iw/zoom)*(on/{last_f})':y='(ih-ih/zoom)/2':d={frames}:s={width}x{height}:fps={FRAMES_PER_SECOND}"
            ),
        };
        format!(
            "scale={sw}:{sh}:force_original_aspect_ratio=increase,crop={sw}:{sh},{zoompan},setsar=1"
        )
    };

    let mut chain = base;
    if let Some(overlay) = clip
        .overlay
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let escaped = escape_drawtext(overlay);
        let fontsize = (width / 22).clamp(32, 64);
        let font_clause = font_path
            .map(|p| format!("fontfile='{}':", escape_drawtext(&p.to_string_lossy())))
            .unwrap_or_default();
        // TikTok/Reels/Shorts safe-zone caption pill burned LAST (after motion).
        chain.push_str(&format!(
            ",drawtext={font_clause}text='{escaped}':fontsize={fontsize}:fontcolor=white:x=(w-text_w)/2:y=h*0.72:box=1:boxcolor=black@0.62:boxborderw=18"
        ));
    }
    chain.push_str(",format=yuv420p");
    chain
}

struct TempRenderDir(PathBuf);
impl Drop for TempRenderDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Renders a storyboard into a workspace `.mp4`, sidecar `.json` manifest, and `.storyboard.md`.
pub async fn render_short_video(
    workspace: &Path,
    data_dir: &Path,
    request: &VideoRequest,
) -> Result<RenderedVideo> {
    let root = crate::tools::validate_workspace_root(workspace)?;
    let ffmpeg = detect_ffmpeg_binary().context(
        "FFmpeg was not found on PATH. Install `ffmpeg` to render short videos locally.",
    )?;
    let args = parse_storyboard(
        &request.scenes,
        &request.path,
        request.voice.as_deref(),
        request.aspect_ratio.as_deref(),
        request.title.as_deref(),
    )?;

    let out_mp4 = crate::tools::confine_path(&root, &args.path)?;
    if let Some(parent) = out_mp4.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let tmp_root = data_dir
        .join("video-tmp")
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&tmp_root)?;
    let _guard = TempRenderDir(tmp_root.clone());

    // Step 1: Optional local neural narration via configured TTS runner (`audio.cpp` / Piper).
    let mut scene_wavs: Vec<Option<PathBuf>> = Vec::with_capacity(args.scenes.len());
    let mut narration_secs: Vec<Option<f64>> = Vec::with_capacity(args.scenes.len());
    let mut used_tts = false;

    if request.use_tts {
        for (idx, scene) in args.scenes.iter().enumerate() {
            let wav_path = tmp_root.join(format!("tts_{idx:02}.wav"));
            match crate::audio::run_tts(data_dir, &scene.narration, &wav_path) {
                Ok(_) => {
                    let secs = wav_duration_seconds(&wav_path).ok();
                    used_tts = true;
                    scene_wavs.push(Some(wav_path));
                    narration_secs.push(secs);
                }
                Err(err) => {
                    if !request.allow_silent_fallback {
                        return Err(err).context(
                            "Local TTS failed and silent fallback is disabled. Register an audio.cpp or Piper runner in Settings.",
                        );
                    }
                    scene_wavs.clear();
                    narration_secs.clear();
                    used_tts = false;
                    break;
                }
            }
        }
    }

    let base_plan = RenderPlan::from_args(&args);
    let plan = if used_tts {
        base_plan.retimed(&narration_secs)
    } else {
        base_plan
    };

    // Step 2: Render each scene clip with Ken Burns motion + overlay + matching audio track.
    let font_path = find_system_font();
    let mut clip_files = Vec::with_capacity(plan.clips.len());
    for (idx, clip) in plan.clips.iter().enumerate() {
        let media_path = crate::tools::confine_path(&root, &clip.image_path)?;
        if !media_path.is_file() {
            bail!("Missing scene media: {}", clip.image_path);
        }
        let is_video = is_video_clip_path(&clip.image_path);
        let vf = ffmpeg_scene_filter(
            clip,
            plan.width,
            plan.height,
            is_video,
            font_path.as_deref(),
        );
        let clip_out = tmp_root.join(format!("scene_{idx:02}.mp4"));

        let mut cmd = tokio::process::Command::new(&ffmpeg);
        cmd.arg("-y")
            .arg("-hide_banner")
            .arg("-loglevel")
            .arg("error");
        if is_video {
            cmd.arg("-stream_loop").arg("-1").arg("-i").arg(&media_path);
        } else {
            cmd.arg("-loop").arg("1").arg("-i").arg(&media_path);
        }

        let maybe_wav = scene_wavs.get(idx).and_then(|w| w.as_ref());
        if let Some(wav) = maybe_wav {
            cmd.arg("-i").arg(wav);
        } else {
            cmd.arg("-f")
                .arg("lavfi")
                .arg("-i")
                .arg("anullsrc=r=48000:cl=stereo");
        }

        let dur_str = format!("{:.3}", clip.duration);
        cmd.arg("-t")
            .arg(&dur_str)
            .arg("-vf")
            .arg(&vf)
            .arg("-c:v")
            .arg("libx264")
            .arg("-preset")
            .arg("fast")
            .arg("-crf")
            .arg("20")
            .arg("-pix_fmt")
            .arg("yuv420p")
            .arg("-colorspace")
            .arg("bt709")
            .arg("-color_primaries")
            .arg("bt709")
            .arg("-color_trc")
            .arg("bt709")
            .arg("-color_range")
            .arg("tv");

        if maybe_wav.is_some() {
            let fade_out = (clip.duration - 0.03).max(0.0);
            let af = format!(
                "aresample=48000,aformat=channel_layouts=stereo,afade=t=in:st=0:d=0.03,afade=t=out:st={fade_out:.3}:d=0.03,apad=whole_dur={dur_str}"
            );
            cmd.arg("-af").arg(af);
        }

        cmd.arg("-c:a")
            .arg("aac")
            .arg("-b:a")
            .arg("192k")
            .arg("-ar")
            .arg("48000")
            .arg("-ac")
            .arg("2")
            .arg("-shortest")
            .arg("-map")
            .arg("0:v:0")
            .arg("-map")
            .arg("1:a:0")
            .arg(&clip_out);

        let output = cmd
            .output()
            .await
            .context("Failed to launch ffmpeg for scene render")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("ffmpeg failed on scene {}: {}", idx + 1, stderr.trim());
        }
        clip_files.push(clip_out);
    }

    // Step 3: Concatenate scene clips and finalize (`+faststart`, optional loudnorm & music bed).
    let concat_list = tmp_root.join("concat.txt");
    let list_body = clip_files
        .iter()
        .map(|p| {
            let escaped = p.to_string_lossy().replace('\'', "'\\''");
            format!("file '{escaped}'")
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&concat_list, list_body)?;

    let mut final_cmd = tokio::process::Command::new(&ffmpeg);
    final_cmd
        .arg("-y")
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-f")
        .arg("concat")
        .arg("-safe")
        .arg("0")
        .arg("-i")
        .arg(&concat_list);

    let music_file = match request.music_path.as_deref().map(str::trim) {
        Some(rel) if !rel.is_empty() => Some(crate::tools::confine_path(&root, rel)?),
        _ => None,
    };

    if let Some(music) = music_file {
        final_cmd
            .arg("-stream_loop")
            .arg("-1")
            .arg("-i")
            .arg(music)
            .arg("-filter_complex")
            .arg("[0:a]loudnorm=I=-14:TP=-1.5:LRA=11[vo];[1:a]volume=0.12,aresample=48000[bg];[vo][bg]amix=inputs=2:duration=first:dropout_transition=2[aout]")
            .arg("-map")
            .arg("0:v:0")
            .arg("-map")
            .arg("[aout]")
            .arg("-c:v")
            .arg("copy")
            .arg("-c:a")
            .arg("aac")
            .arg("-b:a")
            .arg("192k")
            .arg("-movflags")
            .arg("+faststart")
            .arg(&out_mp4);
    } else if used_tts {
        final_cmd
            .arg("-c:v")
            .arg("copy")
            .arg("-af")
            .arg("loudnorm=I=-14:TP=-1.5:LRA=11")
            .arg("-c:a")
            .arg("aac")
            .arg("-b:a")
            .arg("192k")
            .arg("-movflags")
            .arg("+faststart")
            .arg(&out_mp4);
    } else {
        final_cmd
            .arg("-c")
            .arg("copy")
            .arg("-movflags")
            .arg("+faststart")
            .arg(&out_mp4);
    }

    let final_out = final_cmd
        .output()
        .await
        .context("Failed to launch ffmpeg for final mux")?;
    if !final_out.status.success() {
        let stderr = String::from_utf8_lossy(&final_out.stderr);
        bail!("ffmpeg final mux failed: {}", stderr.trim());
    }

    // Step 4: Write sidecar manifest (.json) and editable storyboard (.storyboard.md).
    let stem = args
        .path
        .strip_suffix(".mp4")
        .or_else(|| args.path.strip_suffix(".MP4"))
        .unwrap_or(&args.path);
    let manifest_rel = format!("{stem}.json");
    let storyboard_rel = format!("{stem}.storyboard.md");
    let manifest_path = crate::tools::confine_path(&root, &manifest_rel)?;
    let storyboard_path = crate::tools::confine_path(&root, &storyboard_rel)?;

    let manifest = ShortVideoManifest::from_plan(&plan);
    crate::projects::write_json(&manifest_path, &manifest)?;
    std::fs::write(&storyboard_path, serialize_storyboard(&args))?;

    Ok(RenderedVideo {
        artifact_path: args.path,
        manifest_path: manifest_rel,
        storyboard_path: storyboard_rel,
        plan,
        used_tts,
    })
}

/// Reads a rendered workspace `.mp4` as a base64 data URL (bounded to 50 MB) for safe UI preview.
pub fn read_workspace_video(root: &Path, relative: &str) -> Result<String> {
    let path = crate::tools::confine_path(root, relative)?;
    let mut file = std::fs::File::open(&path)?;
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take((MAX_VIDEO_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_VIDEO_BYTES {
        bail!("Video preview supports MP4 files up to 50 MB.");
    }
    if bytes.len() < 12 || &bytes[4..8] != b"ftyp" {
        bail!("Preview requires a valid MP4 video file.");
    }
    Ok(format!("data:video/mp4;base64,{}", STANDARD.encode(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_block_and_json_storyboards_and_roundtrips() {
        let block = "title: Morning Harbor\nvoice: voxcpm2\naspect_ratio: 9:16\n\nimage: assets/harbor.png\nsay: Dawn breaks over the quiet marina.\noverlay: DAWN OVER HARBOR\nmotion: slow_zoom\n\nimage: assets/boat.jpg\nsay: Local boats head out before sunrise.\nmotion: pan_left";
        let args = parse_storyboard(block, "videos/harbor.mp4", None, None, None).unwrap();
        assert_eq!(args.title.as_deref(), Some("Morning Harbor"));
        assert_eq!(args.voice, "voxcpm2");
        assert_eq!(args.aspect_ratio, AspectRatio::Vertical);
        assert_eq!(args.scenes.len(), 2);
        assert_eq!(args.scenes[0].motion, CameraMotion::SlowZoom);
        assert_eq!(args.scenes[0].overlay.as_deref(), Some("DAWN OVER HARBOR"));
        assert_eq!(args.scenes[1].motion, CameraMotion::PanLeft);

        let serialized = serialize_storyboard(&args);
        let reparsed =
            parse_storyboard(&serialized, "videos/harbor.mp4", None, None, None).unwrap();
        assert_eq!(args, reparsed);

        let json_scenes = r#"[{"image":"a.png","say":"First line","overlay":"HOOK","motion":"pan_right"},{"clip":"b.mp4","narration":"Second line"}]"#;
        let parsed_json = parse_scenes(json_scenes).unwrap();
        assert_eq!(parsed_json.len(), 2);
        assert_eq!(parsed_json[0].motion, CameraMotion::PanRight);
        assert!(parsed_json[1].is_video_clip());
    }

    #[test]
    fn rejects_cloud_voices_path_escapes_and_oversized_storyboards() {
        assert!(parse_voice("elevenlabs").is_err());
        assert!(parse_voice("system").is_err());
        assert!(!is_output_path("../escape.mp4"));
        assert!(!is_output_path("/tmp/abs.mp4"));
        assert!(!is_output_path("clip.mov"));
        assert!(is_output_path("videos/launch.mp4"));
        assert!(!is_media_path("../secret.png"));
        assert!(is_media_path("assets/frame.png"));
        assert!(is_media_path("clips/demo.mp4"));
    }

    #[test]
    fn timeline_helpers_retiming_crop_rect_and_waveform_match_mobile() {
        let args = parse_storyboard(
            "image: a.png\nsay: One short line.\n\nimage: b.png\nsay: Another line here.\n\nimage: c.png\nsay: Final line.",
            "out.mp4",
            None,
            Some("9:16"),
            None,
        )
        .unwrap();
        let plan = RenderPlan::from_args(&args);
        let retimed = plan.retimed(&[Some(2.0), Some(3.0), None]);
        assert!((retimed.clips[0].duration - 2.35).abs() < 1e-6);
        assert!((retimed.clips[1].start - 2.35).abs() < 1e-6);
        assert!((retimed.clips[1].duration - 3.35).abs() < 1e-6);
        assert_eq!(retimed.clip_index_at(0.0), 0);
        assert_eq!(retimed.clip_index_at(2.5), 1);
        assert_eq!(retimed.clip_index_at(99.0), 2);
        assert!((retimed.previous_start(2.6, 1.0) - 0.0).abs() < 1e-6);
        assert!((retimed.previous_start(4.0, 1.0) - 2.35).abs() < 1e-6);
        assert!(retimed.next_start(99.0).is_none());
        let frac_sum: f64 = retimed.clip_fractions().into_iter().sum();
        assert!((frac_sum - 1.0).abs() < 1e-6);

        let rect_start = CameraMotion::SlowZoom.crop_rect(1920.0, 1080.0, 1080.0, 1920.0, 0.0);
        let rect_end = CameraMotion::SlowZoom.crop_rect(1920.0, 1080.0, 1080.0, 1920.0, 1.0);
        assert!(rect_end.width < rect_start.width);
        assert!(rect_end.x > rect_start.x);

        let bars = waveform_bars(&[0.0, 0.25, -0.5, 1.0], 2);
        assert_eq!(bars.len(), 2);
        assert!((bars[0] - 0.25).abs() < 1e-6);
        assert!((bars[1] - 1.0).abs() < 1e-6);

        let manifest = ShortVideoManifest::from_plan(&retimed);
        assert_eq!(manifest.to_plan(), retimed);
        assert!(manifest.storyboard_block().contains("image: a.png"));
    }

    #[test]
    fn media_index_deduplicates_photo_names() {
        let taken = HashSet::from(["hero.png".to_string()]);
        let names = ShortVideoMediaIndex::photo_names(
            &["IMG_1.HEIC", "Photo.jpg", "Photo.jpg", "Hero.png"],
            &taken,
        );
        assert_eq!(
            names,
            vec!["img_1.heic", "photo.jpg", "photo-2.jpg", "hero-2.png"]
        );
    }

    #[tokio::test]
    async fn renders_real_mp4_and_sidecars_when_ffmpeg_is_installed() {
        let Some(ffmpeg) = detect_ffmpeg_binary() else {
            return;
        };
        let root = std::env::temp_dir().join(format!("keel_video_render_{}", uuid::Uuid::new_v4()));
        let data = std::env::temp_dir().join(format!("keel_video_data_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("assets")).unwrap();
        std::fs::create_dir_all(&data).unwrap();

        // Create two tiny solid test PNGs using ffmpeg itself so there are zero external fixtures.
        for (name, color) in [("scene1.png", "0x1d2433"), ("scene2.png", "0x2b3a4a")] {
            let status = std::process::Command::new(&ffmpeg)
                .args([
                    "-y",
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    &format!("color=c={color}:s=320x480:d=1"),
                    "-frames:v",
                    "1",
                    root.join("assets").join(name).to_str().unwrap(),
                ])
                .status()
                .unwrap();
            assert!(status.success());
        }

        let req = VideoRequest {
            path: "videos/test-short.mp4".into(),
            scenes: "image: assets/scene1.png\nsay: Private AI on your own computer.\noverlay: LOCAL FIRST\nmotion: slow_zoom\n\nimage: assets/scene2.png\nsay: Fast, clean, and offline.\nmotion: pan_right".into(),
            aspect_ratio: Some("9:16".into()),
            title: Some("Keel Test".into()),
            voice: None,
            use_tts: false,
            allow_silent_fallback: true,
            music_path: None,
        };

        let rendered = render_short_video(&root, &data, &req).await.unwrap();
        assert_eq!(rendered.artifact_path, "videos/test-short.mp4");
        assert!(root.join(&rendered.artifact_path).is_file());
        assert!(root.join(&rendered.manifest_path).is_file());
        assert!(root.join(&rendered.storyboard_path).is_file());
        let data_url = read_workspace_video(&root, &rendered.artifact_path).unwrap();
        assert!(data_url.starts_with("data:video/mp4;base64,"));

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn video_script_prompt_carries_subject_tone_and_change() {
        let previous = vec![
            VideoScriptLine {
                narration: "First light hit the harbor.".into(),
                caption: Some("DAWN".into()),
            },
            VideoScriptLine {
                narration: "Coffee warmed our hands.".into(),
                caption: None,
            },
        ];
        let (system, user) = build_video_script_prompt(
            "funny trip with our dog Ruben",
            &["assets/a.jpg".into(), "assets/b.jpg".into()],
            &previous,
            Some("Make it punchier"),
        )
        .unwrap();
        assert!(system.contains("subject and its tone"));
        assert!(system.contains("exactly 2 lines, numbered 1 to 2"));
        assert!(user.contains("The video is about: funny trip with our dog Ruben"));
        assert!(user.contains("1. First light hit the harbor. | DAWN"));
        assert!(user.contains("Change: Make it punchier"));
    }

    #[test]
    fn video_script_parser_strips_think_labels_and_shared_openers() {
        let reply = "\
<think>1. ignore me</think>
1. Test: We built a local AI app for everyone. | LOCAL FIRST
2. Note: We built a fast video maker too. | FAST VIDEOS
3. We built it to stay on your own computer. | PRIVATE
";
        let lines = parse_video_script_reply(reply, 3).unwrap();
        assert_eq!(lines[0].narration, "We built a local AI app for everyone.");
        assert_eq!(lines[0].caption.as_deref(), Some("LOCAL FIRST"));
        assert_eq!(lines[1].narration, "A fast video maker too.");
        assert_eq!(lines[2].narration, "It to stay on your own computer.");

        let scenes =
            script_lines_to_scenes(&lines, &["a.jpg".into(), "b.jpg".into(), "c.jpg".into()]);
        assert_eq!(scenes.len(), 3);
        assert_eq!(scenes[0].motion, CameraMotion::SlowZoom);
        assert_eq!(scenes[1].motion, CameraMotion::PanLeft);
        assert_eq!(scenes[2].motion, CameraMotion::PanRight);
    }
}

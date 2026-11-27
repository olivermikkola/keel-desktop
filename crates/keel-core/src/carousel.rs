//! Photo Slides ("Carousels") studio: turns workspace photos and a plain-English
//! description into a self-contained, swipeable offline HTML slide deck (cover,
//! one slide per photo, and a closing slide) with smart phone-frame layout for
//! screenshots and a subject-first AI copywriter prompt + parser.

use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub const MAX_PHOTOS: usize = 8;
pub const MAX_HEADLINE_WORDS: usize = 8;
pub const MAX_BODY_WORDS: usize = 24;
pub const MAX_DESCRIPTION_CHARS: usize = 600;
pub const MAX_PHOTO_BYTES: usize = 10 * 1024 * 1024;
pub const DEFAULT_ACCENT: &str = "#2bd4a3";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlideKind {
    Cover,
    Photo,
    Closing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SlideStyle {
    #[default]
    Photo,
    Screenshot,
}

impl SlideStyle {
    /// Screenshots sit inside a phone frame so text is never cropped off the sides.
    pub fn from_hint(path_or_note: &str) -> Self {
        let lower = path_or_note.to_ascii_lowercase();
        if lower.contains("screenshot")
            || lower.contains("screen shot")
            || lower.contains("screen_shot")
        {
            Self::Screenshot
        } else {
            Self::Photo
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CarouselSlide {
    pub kind: SlideKind,
    pub headline: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub photo_path: Option<String>,
    #[serde(default)]
    pub style: SlideStyle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CarouselBuildRequest {
    /// Relative `.html` output path inside the workspace, e.g. `photo-slides/trip.html`.
    pub path: String,
    pub description: String,
    pub slides: Vec<CarouselSlide>,
    #[serde(default)]
    pub accent: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CarouselDeckSummary {
    pub id: String,
    pub title: String,
    pub description: String,
    pub html_path: String,
    pub manifest_path: String,
    pub workspace_path: String,
    pub slide_count: usize,
    pub slides: Vec<CarouselSlide>,
    pub created_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CarouselTextSlide {
    pub kind: SlideKind,
    pub headline: String,
    pub body: String,
}

pub fn line_count(photo_count: usize) -> usize {
    photo_count + 2
}

pub fn reply_tokens(photo_count: usize) -> usize {
    96 + 56 * line_count(photo_count)
}

/// Builds a subject-first prompt asking the local model to write a cover slide,
/// one slide per photo in order, and a closing slide. Supports iterative edits
/// via `previous` and `change`.
pub fn build_carousel_prompt(
    description: &str,
    photo_paths: &[String],
    previous: &[CarouselTextSlide],
    change: Option<&str>,
) -> Result<(String, String)> {
    let photo_count = photo_paths.len();
    if photo_count == 0 {
        bail!("Pick at least one photo first.");
    }
    if photo_count > MAX_PHOTOS {
        bail!("Pick up to {MAX_PHOTOS} photos.");
    }
    let about = description.split_whitespace().collect::<Vec<_>>().join(" ");
    if about.is_empty() || about.chars().count() > MAX_DESCRIPTION_CHARS {
        bail!("Describe what these slides are about in 1 to {MAX_DESCRIPTION_CHARS} characters.");
    }
    let total = line_count(photo_count);
    let plural = if photo_count == 1 { "" } else { "s" };
    let system = format!(
        "You write a swipeable set of photo slides (a cover slide, one slide per photo for {photo_count} photo{plural} in order, and a closing slide).\n\
The user's sentence says what the slides are about: that is the subject and tone. Every slide is about that subject. If the sentence names someone (a person, a pet, a place), say the name.\n\
Match the tone the user asks for: funny if they say funny, warm if warm, a pitch only if they are selling something.\n\
Write exactly {total} lines, numbered 1 to {total}. Line 1 is the cover, lines 2 to {} are the photos in order, line {total} closes.\n\
Each line is a short headline of at most {MAX_HEADLINE_WORDS} words, then |, then one sentence of at most 20 words.\n\
The cover makes people swipe. Each photo slide says one clear thing. The closing slide gives a reason to care or one simple next step.\n\
Start every headline differently. Use plain, everyday words; no hashtags, no emoji, no buzzwords like \"unlock\", \"revolutionize\", \"seamless\".\n\
Write nothing else. Each line looks like this:\n\
1. <headline> | <one sentence>",
        photo_count + 1
    );

    let mut user = format!("These photo slides are about: {about}\n\nPhotos in order:\n");
    for (idx, path) in photo_paths.iter().enumerate() {
        let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
        user.push_str(&format!("Photo {}: {name}\n", idx + 1));
    }
    if !previous.is_empty() {
        user.push_str("\nThe slides so far:\n");
        for (idx, slide) in previous.iter().enumerate() {
            if slide.body.is_empty() {
                user.push_str(&format!("{}. {}\n", idx + 1, slide.headline));
            } else {
                user.push_str(&format!(
                    "{}. {} | {}\n",
                    idx + 1,
                    slide.headline,
                    slide.body
                ));
            }
        }
    }
    if let Some(ch) = change {
        let cleaned = ch.split_whitespace().collect::<Vec<_>>().join(" ");
        if !cleaned.is_empty() {
            let capped: String = cleaned.chars().take(MAX_DESCRIPTION_CHARS).collect();
            user.push_str(&format!(
                "\nChange: {capped}\nWrite all {total} lines again with this change."
            ));
        }
    }
    Ok((system, user.trim_end().to_string()))
}

fn clean_piece(raw: &str) -> String {
    raw.trim()
        .trim_matches(['"', '“', '”', '*', '_', ' '])
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Parses numbered `1. <headline> | <sentence>` lines into cover, photo, and closing slides.
pub fn parse_carousel_reply(raw: &str, photo_count: usize) -> Result<Vec<CarouselTextSlide>> {
    if photo_count == 0 || photo_count > MAX_PHOTOS {
        bail!("Use 1 to {MAX_PHOTOS} photos.");
    }
    let body = raw
        .rsplit_once("</think>")
        .map(|(_, after)| after)
        .unwrap_or(raw);
    let total = line_count(photo_count);
    let mut found: HashMap<usize, (String, String)> = HashMap::new();
    for line in body.lines() {
        let trimmed = line.trim().trim_start_matches(['*', '-', '#', '>', ' ']);
        let digit_len = trimmed.bytes().take_while(|b| b.is_ascii_digit()).count();
        if digit_len == 0 {
            continue;
        }
        let Ok(number) = trimmed[..digit_len].parse::<usize>() else {
            continue;
        };
        if !(1..=total).contains(&number) || found.contains_key(&number) {
            continue;
        }
        let rest = &trimmed[digit_len..];
        let Some(mark) = rest.chars().next() else {
            continue;
        };
        if !matches!(mark, '.' | ')' | ':') {
            continue;
        }
        let after = rest[mark.len_utf8()..].trim();
        let (raw_head, raw_body) = match after.split_once('|') {
            Some((h, b)) => (h, b),
            None => (after, ""),
        };
        let head = clean_piece(raw_head);
        if head.is_empty() {
            continue;
        }
        let headline = head
            .split(' ')
            .take(MAX_HEADLINE_WORDS)
            .collect::<Vec<_>>()
            .join(" ");
        let body_text = clean_piece(raw_body)
            .split(' ')
            .take(MAX_BODY_WORDS)
            .collect::<Vec<_>>()
            .join(" ");
        found.insert(number, (headline, body_text));
    }
    if found.len() != total {
        bail!("The AI did not write every slide. Try again, or describe it more simply.");
    }
    let mut out = Vec::with_capacity(total);
    for num in 1..=total {
        let (headline, body) = found.remove(&num).unwrap();
        let kind = if num == 1 {
            SlideKind::Cover
        } else if num == total {
            SlideKind::Closing
        } else {
            SlideKind::Photo
        };
        out.push(CarouselTextSlide {
            kind,
            headline,
            body,
        });
    }
    Ok(out)
}

/// Escapes text so it is safe inside HTML elements and quoted attributes.
pub fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

pub fn is_valid_output_path(path: &str) -> bool {
    let trimmed = path.trim();
    if trimmed.len() <= 5
        || trimmed.len() > 160
        || !trimmed.to_ascii_lowercase().ends_with(".html")
        || trimmed.starts_with('/')
        || trimmed.contains('\\')
    {
        return false;
    }
    !trimmed
        .split('/')
        .any(|p| p.is_empty() || p == "." || p == "..")
}

fn sanitize_accent(raw: Option<&str>) -> &str {
    match raw.map(str::trim) {
        Some(hex)
            if hex.len() == 7
                && hex.starts_with('#')
                && hex[1..].bytes().all(|b| b.is_ascii_hexdigit()) =>
        {
            hex
        }
        _ => DEFAULT_ACCENT,
    }
}

fn image_data_url(workspace_root: &Path, rel_path: &str) -> Result<String> {
    let full = crate::tools::confine_path(workspace_root, rel_path)?;
    let bytes = std::fs::read(&full)
        .with_context(|| format!("Could not read photo `{rel_path}` in this folder."))?;
    if bytes.is_empty() || bytes.len() > MAX_PHOTO_BYTES {
        bail!("Photo `{rel_path}` must be between 1 byte and 10 MB.");
    }
    let ext = full
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("png")
        .to_ascii_lowercase();
    let mime = match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => "image/png",
    };
    Ok(format!("data:{mime};base64,{}", STANDARD.encode(bytes)))
}

/// Renders a complete, self-contained, zero-network HTML Photo Slides deck
/// with swipe/arrow controls, screenshot phone framing, and print-to-PDF layout.
pub fn render_deck_html(
    title: &str,
    slides: &[(CarouselSlide, Option<String>)],
    accent: Option<&str>,
) -> String {
    let accent = sanitize_accent(accent);
    let safe_title = escape_html(title);
    let count = slides.len();
    let cards: String = slides
        .iter()
        .enumerate()
        .map(|(idx, (slide, data_url))| {
            let headline = escape_html(&slide.headline);
            let body = escape_html(&slide.body);
            let swipe = if slide.kind == SlideKind::Cover {
                "<div class=\"swipe\">Swipe →</div>"
            } else {
                ""
            };
            let body_html = if body.is_empty() {
                String::new()
            } else {
                format!("<p>{body}</p>")
            };
            let active = if idx == 0 { " active" } else { "" };
            if slide.style == SlideStyle::Screenshot {
                if let Some(photo) = data_url {
                    let safe_photo = escape_html(photo);
                    return format!(
                        "<section class=\"slide screenshot{active}\" data-index=\"{idx}\">\
<div class=\"device\" style=\"background-image:url('{safe_photo}')\"></div>\
<div class=\"count\">{}/{count}</div>\
<div class=\"text center\"><div class=\"kicker\"></div><h1>{headline}</h1>{body_html}{swipe}</div>\
</section>",
                        idx + 1
                    );
                }
            }
            let bg = match data_url {
                Some(photo) => format!("background-image:url('{}')", escape_html(photo)),
                None => "background:linear-gradient(160deg,#0f2a24,#071311)".to_string(),
            };
            format!(
                "<section class=\"slide photo{active}\" data-index=\"{idx}\">\
<div class=\"photo-bg\" style=\"{bg}\"></div><div class=\"fade\"></div>\
<div class=\"count\">{}/{count}</div>\
<div class=\"text\"><div class=\"kicker\"></div><h1>{headline}</h1>{body_html}{swipe}</div>\
</section>",
                idx + 1
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; img-src data:\">\n\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n\
<title>{safe_title}</title>\n<style>\n\
*{{box-sizing:border-box}}html,body{{margin:0;height:100%;background:#091110;color:#fff;font-family:-apple-system,BlinkMacSystemFont,Segoe UI,Helvetica,sans-serif}}\n\
.stage{{min-height:100%;display:flex;flex-direction:column;align-items:center;justify-content:center;padding:18px;gap:14px}}\n\
.deck{{position:relative;width:min(420px,92vw);aspect-ratio:4/5;border-radius:20px;overflow:hidden;box-shadow:0 18px 48px rgba(0,0,0,.55);background:#071311}}\n\
.slide{{position:absolute;inset:0;display:none;overflow:hidden}}\n\
.slide.active{{display:block}}\n\
.photo-bg{{position:absolute;inset:0;background-size:cover;background-position:center;background-repeat:no-repeat}}\n\
.fade{{position:absolute;inset:0;background:linear-gradient(180deg,rgba(0,0,0,0) 32%,rgba(0,0,0,.84) 80%)}}\n\
.slide.screenshot{{background:radial-gradient(circle at 50% 28%,#163a32,#071311 72%)}}\n\
.device{{position:absolute;left:20%;right:20%;top:7%;height:54%;border-radius:20px;border:5px solid #1c1f20;box-shadow:0 14px 36px rgba(0,0,0,.6);background-size:cover;background-position:top center;background-repeat:no-repeat}}\n\
.count{{position:absolute;top:16px;right:16px;font-size:13px;font-weight:700;background:rgba(0,0,0,.42);padding:5px 11px;border-radius:999px}}\n\
.text{{position:absolute;left:7%;right:7%;bottom:8%}}\n\
.text.center{{text-align:center}}\n\
.kicker{{display:inline-block;width:44px;height:6px;background:{accent};border-radius:4px;margin-bottom:12px}}\n\
h1{{margin:0;font-size:clamp(24px,6vw,38px);font-weight:900;line-height:1.05;letter-spacing:-.02em}}\n\
.slide.screenshot h1{{font-size:clamp(20px,4.8vw,30px)}}\n\
p{{margin:10px 0 0;color:rgba(255,255,255,.92);font-size:clamp(14px,3.2vw,18px);line-height:1.35;font-weight:500}}\n\
.swipe{{margin-top:12px;color:{accent};font-size:15px;font-weight:800}}\n\
.nav{{display:flex;gap:10px;align-items:center}}\n\
.nav button{{background:#172523;color:#fff;border:1px solid rgba(255,255,255,.16);border-radius:999px;padding:8px 16px;font:inherit;font-size:13px;font-weight:600;cursor:pointer}}\n\
.nav button:disabled{{opacity:.4;cursor:not-allowed}}\n\
@media print{{body{{background:#fff}}.stage{{display:block;padding:0}}.nav{{display:none}}.deck{{width:100%;aspect-ratio:auto;box-shadow:none;overflow:visible;border-radius:0}}.slide{{position:relative;display:block!important;width:100%;height:100vh;page-break-after:always}}}}\n\
</style>\n</head>\n<body>\n\
<div class=\"stage\">\n<div class=\"deck\" id=\"deck\">{cards}</div>\n\
<div class=\"nav\"><button id=\"prev\">← Previous</button><span id=\"pos\">1 / {count}</span><button id=\"next\">Next →</button><button id=\"print\">Print / PDF</button></div>\n\
</div>\n<script>\n\
const slides=[...document.querySelectorAll('.slide')];let i=0;\n\
function show(n){{i=Math.max(0,Math.min(slides.length-1,n));slides.forEach((s,idx)=>s.classList.toggle('active',idx===i));document.getElementById('pos').textContent=(i+1)+' / '+slides.length;document.getElementById('prev').disabled=i===0;document.getElementById('next').disabled=i===slides.length-1;}}\n\
document.getElementById('prev').onclick=()=>show(i-1);document.getElementById('next').onclick=()=>show(i+1);document.getElementById('print').onclick=()=>window.print();\n\
window.addEventListener('keydown',e=>{{if(e.key==='ArrowLeft')show(i-1);if(e.key==='ArrowRight')show(i+1);}});\n\
show(0);\n\
</script>\n</body>\n</html>\n"
    )
}

/// Builds and saves a Photo Slides HTML deck and its JSON manifest in the workspace.
pub fn build_carousel_deck(
    workspace_root: &Path,
    req: &CarouselBuildRequest,
) -> Result<CarouselDeckSummary> {
    let html_rel = req.path.trim();
    if !is_valid_output_path(html_rel) {
        bail!("Save path must be a relative .html file inside your folder, such as photo-slides/trip.html.");
    }
    if req.slides.is_empty() || req.slides.len() > MAX_PHOTOS + 2 {
        bail!("Photo Slides need between 1 and {} slides.", MAX_PHOTOS + 2);
    }

    let mut rendered_slides = Vec::with_capacity(req.slides.len());
    for slide in &req.slides {
        if slide.headline.trim().is_empty() {
            bail!("Every slide needs a headline.");
        }
        let data_url = match slide.photo_path.as_deref().map(str::trim) {
            Some(p) if !p.is_empty() => Some(image_data_url(workspace_root, p)?),
            _ => None,
        };
        rendered_slides.push((slide.clone(), data_url));
    }

    let title = req
        .slides
        .first()
        .map(|s| s.headline.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Photo Slides".to_string());
    let html = render_deck_html(&title, &rendered_slides, req.accent.as_deref());

    let html_full = crate::tools::confine_path(workspace_root, html_rel)?;
    if let Some(parent) = html_full.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&html_full, html)?;

    let manifest_rel = format!("{}.json", html_rel.trim_end_matches(".html"));
    let manifest_full = crate::tools::confine_path(workspace_root, &manifest_rel)?;
    let created_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let summary = CarouselDeckSummary {
        id: uuid::Uuid::new_v4().to_string(),
        title,
        description: req.description.trim().to_string(),
        html_path: html_rel.to_string(),
        manifest_path: manifest_rel,
        workspace_path: workspace_root.to_string_lossy().into_owned(),
        slide_count: req.slides.len(),
        slides: req.slides.clone(),
        created_at,
    };
    std::fs::write(&manifest_full, serde_json::to_vec_pretty(&summary)?)?;
    Ok(summary)
}

/// Lists saved Photo Slides manifests inside `<workspace>/photo-slides/`.
pub fn list_workspace_carousels(workspace_root: &Path) -> Vec<CarouselDeckSummary> {
    let dir = workspace_root.join("photo-slides");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut decks = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Ok(raw) = std::fs::read_to_string(&path) {
            if let Ok(deck) = serde_json::from_str::<CarouselDeckSummary>(&raw) {
                if crate::tools::confine_path(workspace_root, &deck.html_path)
                    .is_ok_and(|p| p.is_file())
                {
                    decks.push(deck);
                }
            }
        }
    }
    decks.sort_by_key(|d| std::cmp::Reverse(d.created_at));
    decks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_carries_subject_tone_and_change_request() {
        let prev = vec![
            CarouselTextSlide {
                kind: SlideKind::Cover,
                headline: "Meet Ruben".into(),
                body: "Our clumsy dog.".into(),
            },
            CarouselTextSlide {
                kind: SlideKind::Photo,
                headline: "Sofa king".into(),
                body: "He claimed the whole couch.".into(),
            },
            CarouselTextSlide {
                kind: SlideKind::Closing,
                headline: "Good boy".into(),
                body: "Give him a treat.".into(),
            },
        ];
        let (sys, usr) = build_carousel_prompt(
            "funny slides about my dog Ruben",
            &["photos/ruben.jpg".into()],
            &prev,
            Some("Make the cover punchier"),
        )
        .unwrap();
        assert!(sys.contains("Write exactly 3 lines, numbered 1 to 3"));
        assert!(sys.contains("subject and tone"));
        assert!(usr.contains("The slides so far:\n1. Meet Ruben | Our clumsy dog."));
        assert!(usr.contains("Change: Make the cover punchier"));
    }

    #[test]
    fn parses_slides_strips_think_and_caps_words() {
        let raw = "\
<think>1. ignore | nope</think>
1. **Your phone can do this right now today** | Runs privately without any cloud account.
2. Pick a photo | Turn everyday shots into clean slides.
3. Try it out | Save as PDF or share anywhere.
";
        let slides = parse_carousel_reply(raw, 1).unwrap();
        assert_eq!(slides.len(), 3);
        assert_eq!(slides[0].kind, SlideKind::Cover);
        assert_eq!(slides[1].kind, SlideKind::Photo);
        assert_eq!(slides[2].kind, SlideKind::Closing);
        assert_eq!(slides[0].headline.split(' ').count(), MAX_HEADLINE_WORDS);
    }

    #[test]
    fn renders_escaped_html_with_phone_frame_for_screenshots() {
        assert_eq!(
            SlideStyle::from_hint("assets/Screenshot_1.png"),
            SlideStyle::Screenshot
        );
        assert_eq!(SlideStyle::from_hint("assets/beach.jpg"), SlideStyle::Photo);
        let slides = vec![
            (
                CarouselSlide {
                    kind: SlideKind::Cover,
                    headline: "<script>alert(1)</script> & \"you\"".into(),
                    body: "It's private".into(),
                    photo_path: None,
                    style: SlideStyle::Photo,
                },
                None,
            ),
            (
                CarouselSlide {
                    kind: SlideKind::Photo,
                    headline: "Clean screen".into(),
                    body: "Nothing cropped".into(),
                    photo_path: Some("assets/screenshot.png".into()),
                    style: SlideStyle::Screenshot,
                },
                Some("data:image/png;base64,AAAA".into()),
            ),
        ];
        let html = render_deck_html("Demo", &slides, None);
        assert!(!html.contains("<script>alert(1)</script>"));
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(html.contains("It&#39;s private"));
        assert!(html.contains("Swipe →"));
        assert!(html.contains("<div class=\"device\""));
        assert!(html.contains("Content-Security-Policy"));
    }

    #[test]
    fn builds_and_lists_deck_in_temp_workspace() {
        let root = std::env::temp_dir().join(format!("keel-carousel-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("assets")).unwrap();
        // Minimal 1x1 transparent PNG bytes.
        let png = STANDARD
            .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+j5uoAAAAASUVORK5CYII=")
            .unwrap();
        std::fs::write(root.join("assets/shot.png"), png).unwrap();

        let req = CarouselBuildRequest {
            path: "photo-slides/demo.html".into(),
            description: "Weekend demo".into(),
            slides: vec![
                CarouselSlide {
                    kind: SlideKind::Cover,
                    headline: "Weekend trip".into(),
                    body: "Three quick highlights.".into(),
                    photo_path: Some("assets/shot.png".into()),
                    style: SlideStyle::Photo,
                },
                CarouselSlide {
                    kind: SlideKind::Closing,
                    headline: "See you next time".into(),
                    body: "Saved offline.".into(),
                    photo_path: None,
                    style: SlideStyle::Photo,
                },
            ],
            accent: None,
        };
        let built = build_carousel_deck(&root, &req).unwrap();
        assert_eq!(built.slide_count, 2);
        assert!(root.join("photo-slides/demo.html").is_file());
        assert!(root.join("photo-slides/demo.json").is_file());

        let listed = list_workspace_carousels(&root);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].html_path, "photo-slides/demo.html");

        // Reject traversal output path.
        let bad = CarouselBuildRequest {
            path: "../escape.html".into(),
            ..req
        };
        assert!(build_carousel_deck(&root, &bad).is_err());

        let _ = std::fs::remove_dir_all(&root);
    }
}

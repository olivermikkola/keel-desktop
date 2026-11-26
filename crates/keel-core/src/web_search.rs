//! Explicit web search for Chat and Work. Results leave the machine over the network;
//! the agent loop requires user approval before calling this tool.
use anyhow::{bail, Context, Result};
use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;

const MAX_QUERY_CHARS: usize = 300;
const MAX_RESULT_CHARS: usize = 6_000;
const MAX_ORGANIC: usize = 5;
const MAX_PAGE_BYTES: usize = 512 * 1024;
const USER_AGENT: &str = concat!(
    "KeelDesktop/",
    env!("CARGO_PKG_VERSION"),
    " (local assistant; +https://github.com/olivermikkola/keel-desktop)"
);

fn search_http_client() -> Result<&'static reqwest::Client> {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    if let Some(c) = CLIENT.get() {
        return Ok(c);
    }
    let built = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::limited(5))
        .user_agent(USER_AGENT)
        .build()?;
    Ok(CLIENT.get_or_init(|| built))
}

fn read_url_http_client() -> Result<&'static reqwest::Client> {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    if let Some(c) = CLIENT.get() {
        return Ok(c);
    }
    let built = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(USER_AGENT)
        .build()?;
    Ok(CLIENT.get_or_init(|| built))
}

fn is_private_or_local_host(host: &str) -> bool {
    let h = host
        .trim()
        .trim_matches(|c| c == '[' || c == ']')
        .to_ascii_lowercase();
    if h.is_empty()
        || h == "localhost"
        || h == "0.0.0.0"
        || h == "::1"
        || h == "::"
        || h.ends_with(".local")
        || h.ends_with(".internal")
        || h.ends_with(".localhost")
    {
        return true;
    }
    if let Ok(ip) = h.parse::<std::net::IpAddr>() {
        return match ip {
            std::net::IpAddr::V4(v4) => {
                let o = v4.octets();
                v4.is_loopback()
                    || v4.is_private()
                    || v4.is_link_local()
                    || v4.is_unspecified()
                    || v4.is_broadcast()
                    || v4.is_multicast()
                    || (o[0] == 100 && (64..=127).contains(&o[1]))
            }
            std::net::IpAddr::V6(v6) => {
                let s = v6.segments();
                v6.is_loopback()
                    || v6.is_unspecified()
                    || v6.is_multicast()
                    || (s[0] & 0xfe00) == 0xfc00
                    || (s[0] & 0xffc0) == 0xfe80
            }
        };
    }
    false
}

/// Validates that `raw` is a public `http`/`https` URL without credentials or local/private hosts.
pub fn validate_public_url(raw: &str) -> Result<reqwest::Url> {
    let trimmed = raw.trim();
    let parsed = reqwest::Url::parse(trimmed).context("Enter a valid http:// or https:// URL")?;
    if !matches!(parsed.scheme(), "http" | "https") {
        bail!("Only http:// and https:// URLs can be read");
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        bail!("URLs with embedded credentials are not allowed");
    }
    let host = parsed.host_str().unwrap_or("");
    if is_private_or_local_host(host) {
        bail!("Private or local network URLs cannot be read by read_url");
    }
    Ok(parsed)
}

/// Converts HTML into bounded readable plain text (stripping scripts, styles, and tags).
pub fn html_to_bounded_text(html: &str, max_chars: usize) -> String {
    let no_scripts =
        Regex::new(r"(?is)<(script|style|noscript)[^>]*>.*?</(script|style|noscript)>")
            .unwrap()
            .replace_all(html, " ");
    let no_tags = Regex::new(r"(?s)<[^>]+>")
        .unwrap()
        .replace_all(&no_scripts, " ");
    let decoded = decode_basic_entities(&no_tags);
    let collapsed = Regex::new(r"[ \t\r\n]+")
        .unwrap()
        .replace_all(&decoded, " ");
    let clean = collapsed.trim();
    if clean.chars().count() <= max_chars {
        clean.to_string()
    } else {
        let mut out: String = clean.chars().take(max_chars).collect();
        out.push_str("\n\n[Page content truncated.]");
        out
    }
}

/// Fetches a public web page (no redirects, 512 KB cap) and returns bounded readable text.
pub async fn read_url(url: &str) -> Result<String> {
    read_url_with_client(read_url_http_client()?, url).await
}

pub async fn read_url_with_client(client: &reqwest::Client, url: &str) -> Result<String> {
    let parsed = validate_public_url(url)?;
    let mut resp = client
        .get(parsed.clone())
        .send()
        .await
        .context("Could not reach that web page")?;
    if resp.status().is_redirection() {
        bail!("That URL redirected to another address; ask to read the final public URL directly.");
    }
    if !resp.status().is_success() {
        bail!("Web page returned HTTP {}", resp.status());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .context("Failed while reading web page")?
    {
        let remaining = MAX_PAGE_BYTES.saturating_sub(bytes.len());
        if remaining == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
        if bytes.len() >= MAX_PAGE_BYTES {
            break;
        }
    }
    let raw_text = String::from_utf8_lossy(&bytes);
    let body = html_to_bounded_text(&raw_text, MAX_RESULT_CHARS);
    if body.is_empty() {
        bail!("That page contained no readable text.");
    }
    Ok(format!("Content from {}:\n\n{}", parsed, body))
}

/// Search the public web for `query` and return a bounded text summary for the model.
pub async fn web_search(query: &str) -> Result<String> {
    web_search_with_client(search_http_client()?, query).await
}

pub async fn web_search_with_client(client: &reqwest::Client, query: &str) -> Result<String> {
    let query = query.trim();
    if query.is_empty() {
        bail!("Search query must not be empty");
    }
    if query.chars().count() > MAX_QUERY_CHARS {
        bail!("Search query is too long (max {MAX_QUERY_CHARS} characters)");
    }

    let mut sections = Vec::new();
    match fetch_instant_answer(client, query).await {
        Ok(text) if !text.trim().is_empty() => sections.push(text),
        Ok(_) => {}
        Err(err) => sections.push(format!("Instant answer unavailable: {err}")),
    }
    match fetch_organic_results(client, query).await {
        Ok(text) if !text.trim().is_empty() => sections.push(text),
        Ok(_) => {}
        Err(err) => {
            if sections.is_empty() {
                return Err(err);
            }
            sections.push(format!("Organic results unavailable: {err}"));
        }
    }

    if sections.is_empty() {
        bail!("No search results found for that query.");
    }

    let mut out = format!(
        "Web search results for {query:?}:\n\n{}",
        sections.join("\n\n")
    );
    if out.chars().count() > MAX_RESULT_CHARS {
        out = out.chars().take(MAX_RESULT_CHARS).collect::<String>();
        out.push_str("\n\n[Search results truncated.]");
    }
    Ok(out)
}

async fn fetch_instant_answer(client: &reqwest::Client, query: &str) -> Result<String> {
    let resp = client
        .get("https://api.duckduckgo.com/")
        .query(&[
            ("q", query),
            ("format", "json"),
            ("no_html", "1"),
            ("skip_disambig", "1"),
        ])
        .send()
        .await
        .context("Could not reach the search service")?;
    if !resp.status().is_success() {
        bail!("Search service returned HTTP {}", resp.status());
    }
    let value: Value = resp
        .json()
        .await
        .context("Search service returned invalid JSON")?;
    Ok(format_instant_answer(&value))
}

/// Formats a DuckDuckGo Instant Answer JSON payload into readable tool output.
pub fn format_instant_answer(value: &Value) -> String {
    let mut lines = Vec::new();
    let heading = value
        .get("Heading")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let abstract_text = value
        .get("AbstractText")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let abstract_url = value
        .get("AbstractURL")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let answer = value
        .get("Answer")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let definition = value
        .get("Definition")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();

    if !heading.is_empty() && !abstract_text.is_empty() {
        lines.push(format!("Summary ({heading}): {abstract_text}"));
        if !abstract_url.is_empty() {
            lines.push(format!("Source: {abstract_url}"));
        }
    } else if !abstract_text.is_empty() {
        lines.push(format!("Summary: {abstract_text}"));
        if !abstract_url.is_empty() {
            lines.push(format!("Source: {abstract_url}"));
        }
    }
    if !answer.is_empty() {
        lines.push(format!("Direct answer: {answer}"));
    }
    if !definition.is_empty() {
        lines.push(format!("Definition: {definition}"));
    }

    if let Some(topics) = value.get("RelatedTopics").and_then(|v| v.as_array()) {
        let mut related = Vec::new();
        collect_related_topics(topics, &mut related, 5);
        if !related.is_empty() {
            lines.push("Related:".to_string());
            lines.extend(related);
        }
    }

    lines.join("\n")
}

fn collect_related_topics(topics: &[Value], out: &mut Vec<String>, limit: usize) {
    for topic in topics {
        if out.len() >= limit {
            return;
        }
        if let Some(nested) = topic.get("Topics").and_then(|v| v.as_array()) {
            collect_related_topics(nested, out, limit);
            continue;
        }
        let text = topic
            .get("Text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let url = topic
            .get("FirstURL")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if text.is_empty() {
            continue;
        }
        if url.is_empty() {
            out.push(format!("- {text}"));
        } else {
            out.push(format!("- {text} ({url})"));
        }
    }
}

async fn fetch_organic_results(client: &reqwest::Client, query: &str) -> Result<String> {
    let resp = client
        .get("https://html.duckduckgo.com/html/")
        .query(&[("q", query)])
        .send()
        .await
        .context("Could not reach organic search results")?;
    if !resp.status().is_success() {
        bail!("Organic search returned HTTP {}", resp.status());
    }
    let html = resp
        .text()
        .await
        .context("Could not read organic search HTML")?;
    Ok(format_organic_html(&html))
}

/// Extracts a few organic result titles/URLs/snippets from DuckDuckGo HTML.
pub fn format_organic_html(html: &str) -> String {
    let link_re =
        Regex::new(r#"(?s)class="result__a"[^>]*href="([^"]+)"[^>]*>(.*?)</a>"#).expect("link re");
    let snippet_re =
        Regex::new(r#"(?s)class="result__snippet"[^>]*>(.*?)</(?:a|td|div)>"#).expect("snippet re");
    let tag_re = Regex::new(r"<[^>]+>").expect("tag strip regex");
    let links: Vec<_> = link_re.captures_iter(html).take(MAX_ORGANIC).collect();
    let snippets: Vec<_> = snippet_re.captures_iter(html).take(MAX_ORGANIC).collect();
    let mut lines = Vec::new();
    for (i, caps) in links.iter().enumerate() {
        let href = caps.get(1).map(|m| m.as_str()).unwrap_or("");
        let title = caps.get(2).map(|m| m.as_str()).unwrap_or("");
        let snippet = snippets
            .get(i)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str())
            .unwrap_or("");
        let title = decode_basic_entities(tag_re.replace_all(title, "").trim());
        let snippet = decode_basic_entities(tag_re.replace_all(snippet, "").trim());
        let url = unwrap_duckduckgo_redirect(href);
        if title.is_empty() && url.is_empty() {
            continue;
        }
        let mut block = format!("{}. {title}", lines.len() + 1);
        if !url.is_empty() {
            block.push_str(&format!("\n   {url}"));
        }
        if !snippet.is_empty() {
            block.push_str(&format!("\n   {snippet}"));
        }
        lines.push(block);
    }
    if lines.is_empty() {
        String::new()
    } else {
        format!("Top results:\n{}", lines.join("\n"))
    }
}

fn unwrap_duckduckgo_redirect(href: &str) -> String {
    let href = html_unescape_attr(href);
    if let Some(idx) = href.find("uddg=") {
        let encoded = &href[idx + 5..];
        let end = encoded.find('&').unwrap_or(encoded.len());
        return urlencoding_decode(&encoded[..end]);
    }
    if href.starts_with("http://") || href.starts_with("https://") {
        href
    } else {
        String::new()
    }
}

fn html_unescape_attr(value: &str) -> String {
    decode_basic_entities(value)
}

fn decode_basic_entities(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

fn urlencoding_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let h = from_hex(bytes[i + 1]);
                let l = from_hex(bytes[i + 2]);
                if let (Some(h), Some(l)) = (h, l) {
                    out.push((h << 4) | l);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn from_hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn formats_instant_answer_summary_and_related() {
        let value = json!({
            "Heading": "Helsinki",
            "AbstractText": "Capital of Finland.",
            "AbstractURL": "https://en.wikipedia.org/wiki/Helsinki",
            "Answer": "",
            "Definition": "",
            "RelatedTopics": [
                {"Text": "Espoo is nearby", "FirstURL": "https://example.com/espoo"},
                {"Topics": [{"Text": "Vantaa", "FirstURL": "https://example.com/vantaa"}]}
            ]
        });
        let text = format_instant_answer(&value);
        assert!(text.contains("Capital of Finland"));
        assert!(text.contains("https://en.wikipedia.org/wiki/Helsinki"));
        assert!(text.contains("Espoo is nearby"));
        assert!(text.contains("Vantaa"));
    }

    #[test]
    fn formats_organic_html_results() {
        let html = r#"
            <div class="result">
              <a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fone">Example One</a>
              <a class="result__snippet">First snippet about one.</a>
            </div>
            <div class="result">
              <a class="result__a" href="https://example.com/two">Example Two</a>
              <a class="result__snippet">Second snippet.</a>
            </div>
        "#;
        let text = format_organic_html(html);
        assert!(text.contains("Example One"));
        assert!(text.contains("https://example.com/one"));
        assert!(text.contains("First snippet about one."));
        assert!(text.contains("Example Two"));
    }

    #[test]
    fn rejects_empty_query() {
        let err = futures_executor_block_on(web_search("   ")).unwrap_err();
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn read_url_blocks_private_hosts_credentials_and_cleans_html() {
        assert!(validate_public_url("https://example.com/docs").is_ok());
        for bad in [
            "file:///etc/passwd",
            "http://127.0.0.1:8080/health",
            "http://localhost/admin",
            "http://10.0.0.1/internal",
            "http://172.16.0.5/secret",
            "http://192.168.1.1/router",
            "http://169.254.169.254/latest/meta-data",
            "http://100.64.0.1/ts",
            "http://[::1]/meta",
            "http://user:pass@example.com/",
            "http://mybox.local/status",
        ] {
            assert!(validate_public_url(bad).is_err(), "expected reject: {bad}");
        }
        let clean = html_to_bounded_text(
            "<html><head><style>body{color:red}</style><script>alert(1)</script></head><body><h1>Hello &amp; Welcome</h1><p>Local AI</p></body></html>",
            200,
        );
        assert!(clean.contains("Hello & Welcome"));
        assert!(clean.contains("Local AI"));
        assert!(!clean.contains("alert(1)"));
        assert!(!clean.contains("color:red"));
    }

    #[tokio::test]
    #[ignore = "hits the live DuckDuckGo network; run with --ignored when online"]
    async fn live_web_search_returns_bounded_text() {
        let out = web_search("Helsinki")
            .await
            .expect("live search should succeed");
        assert!(out.contains("Helsinki") || out.contains("helsinki"));
        assert!(out.chars().count() <= MAX_RESULT_CHARS + 40);
        assert!(!out.trim().is_empty());
    }

    fn futures_executor_block_on<F: std::future::Future>(fut: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(fut)
    }
}

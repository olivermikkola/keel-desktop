//! Explicit web search for Chat and Work. Results leave the machine over the network;
//! the agent loop requires user approval before calling this tool.
use anyhow::{bail, Context, Result};
use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;

const MAX_QUERY_CHARS: usize = 300;
const MAX_RESULT_CHARS: usize = 6_000;
const MAX_ORGANIC: usize = 5;

fn search_http_client() -> Result<&'static reqwest::Client> {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    if let Some(c) = CLIENT.get() {
        return Ok(c);
    }
    let built = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::limited(5))
        .user_agent(
            "KeelDesktop/0.5 (local assistant; +https://github.com/olivermikkola/keel-desktop)",
        )
        .build()?;
    Ok(CLIENT.get_or_init(|| built))
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

    fn futures_executor_block_on<F: std::future::Future>(fut: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(fut)
    }
}

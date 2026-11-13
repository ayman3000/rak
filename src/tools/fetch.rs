//! fetch_url: a public web page as readable text. Ported from SwiftAgentKit's
//! `FetchURLTool`, `HTMLReader` and `SSRFGuard`.
//!
//! The shell could run curl, but a typical page is ~100 KB of HTML carrying a
//! few KB of text, and all of it would land in the model's context. This keeps
//! headings, lists and links, drops scripts, styles and chrome, and caps what
//! it returns. It only reads, so it needs no approval.
//!
//! "Fetch this address" is exactly what a hostile page would plant, and this
//! machine and its network are where the interesting things listen. So only
//! public http(s) addresses are read: every address a host name resolves to
//! must be public, checked in the resolver the connection itself uses — for
//! the first address and for every redirect — so a DNS answer cannot change
//! between the check and the connection (SAK checks before connecting only).

use super::{int_arg, str_arg, Tool, ToolResult};
use serde_json::{json, Value};
use std::io::Read;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::time::Duration;

/// Bytes read off the wire before giving up: a cap on the download, so a huge
/// page fails fast instead of filling memory.
const MAX_DOWNLOAD: u64 = 8 * 1024 * 1024;
const DEFAULT_MAX_CHARS: usize = 40_000;
/// Marks the resolver's refusal inside ureq's error text.
const BLOCKED: &str = "rak-blocked:";

pub struct FetchUrl {
    http: ureq::Agent,
}

impl Default for FetchUrl {
    fn default() -> Self {
        Self::new()
    }
}

impl FetchUrl {
    pub fn new() -> Self {
        let http = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(25))
            .redirects(5)
            .resolver(|netloc: &str| public_addresses(netloc, &|n| n.to_socket_addrs().map(Iterator::collect)))
            // Some sites serve a stub or a block page to an unknown client.
            .user_agent(
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15",
            )
            .build();
        FetchUrl { http }
    }
}

/// Pure — unit-tested (with a stand-in lookup). The addresses for `netloc`
/// ("host:port") when every one is public; otherwise an error naming why.
pub fn public_addresses(netloc: &str, lookup: &dyn Fn(&str) -> std::io::Result<Vec<SocketAddr>>) -> std::io::Result<Vec<SocketAddr>> {
    let refuse = |why: String| Err(std::io::Error::other(format!("{BLOCKED}{why}")));
    let host = netloc.rsplit_once(':').map(|(h, _)| h).unwrap_or(netloc).trim_matches(['[', ']']).to_ascii_lowercase();
    if let Some(why) = local_name(&host) {
        return refuse(why);
    }
    let addresses = lookup(netloc)?;
    if addresses.is_empty() {
        return refuse(format!("{host} did not resolve"));
    }
    for address in &addresses {
        if let Some(why) = private_ip(address.ip()) {
            return refuse(format!("{host} resolves to {why}"));
        }
    }
    Ok(addresses)
}

/// Names that mean this machine or the local network, before any lookup.
fn local_name(host: &str) -> Option<String> {
    let local = host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".internal")
        || host == "metadata.google.internal";
    local.then(|| format!("{host} is a name on this machine or a private network"))
}

/// Pure — unit-tested. Why an address is not public, or None when it is.
pub fn private_ip(ip: IpAddr) -> Option<String> {
    let v4 = |ip: std::net::Ipv4Addr| {
        let [a, b, ..] = ip.octets();
        let blocked = matches!(a, 0 | 10 | 127)
            || (a == 100 && (64..=127).contains(&b)) // carrier-grade NAT, some metadata services
            || (a == 169 && b == 254) // link-local: cloud metadata at 169.254.169.254
            || (a == 172 && (16..=31).contains(&b))
            || (a == 192 && b == 168)
            || a >= 224; // multicast and reserved
        blocked.then(|| format!("{ip}, a private or reserved address"))
    };
    match ip {
        IpAddr::V4(ip) => v4(ip),
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return v4(mapped);
            }
            let s = ip.segments();
            let blocked = ip.is_loopback()
                || ip.is_unspecified()
                || (s[0] & 0xfe00) == 0xfc00 // unique local
                || (s[0] & 0xffc0) == 0xfe80 // link-local
                || (s[0] & 0xff00) == 0xff00; // multicast
            blocked.then(|| format!("{ip}, a private or reserved address"))
        }
    }
}

/// Pure — unit-tested. The scheme and host of an address the tool may try, or
/// why not. The resolver checks the addresses; this gives clearer reasons.
pub fn check_url(raw: &str) -> Result<(), String> {
    let Some((scheme, rest)) = raw.split_once("://") else {
        return Err(format!("Not a valid URL: {raw} (it needs http:// or https://)."));
    };
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err(format!("fetch_url only reads http(s) pages; {scheme}:// is refused. Read local files with the file tools."));
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit_once('@').map(|(_, h)| h).unwrap_or(authority);
    let host = if host_port.starts_with('[') {
        host_port.split(']').next().unwrap_or("").trim_start_matches('[')
    } else {
        host_port.split(':').next().unwrap_or("")
    }
    .to_ascii_lowercase();
    if host.is_empty() {
        return Err(format!("That URL has no host: {raw}"));
    }
    let local = local_name(&host).or_else(|| host.parse::<IpAddr>().ok().and_then(private_ip));
    match local {
        Some(why) => Err(format!("{why}: fetch_url reads public pages only.")),
        None => Ok(()),
    }
}

impl Tool for FetchUrl {
    fn name(&self) -> &str {
        "fetch_url"
    }
    fn description(&self) -> &str {
        "Fetch a public http(s) page and return its readable text as Markdown (headings, links and lists kept; \
         scripts, styles and navigation chrome dropped). Use `format: \"html\"` only when you need the raw markup. \
         Output is capped — narrow with `max_chars` rather than fetching the same page twice. Pages built entirely \
         by JavaScript return almost nothing here; that is the page, not a failure. Addresses on this machine or a \
         private network are refused."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "Absolute http(s) URL." },
                "format": { "type": "string", "description": "markdown (default), \"text\", or \"html\" for the raw markup." },
                "max_chars": { "type": "integer", "description": "Cap on the characters returned (default 40000)." }
            },
            "required": ["url"]
        })
    }
    // Only reads: no approval, like read_file and web_search.

    fn run(&self, args: &Value) -> ToolResult {
        let Some(url) = str_arg(args, "url").map(str::trim).filter(|u| !u.is_empty()) else {
            return ToolResult::err("fetch_url requires a `url`.");
        };
        if let Err(why) = check_url(url) {
            return ToolResult::err(why);
        }
        let format = str_arg(args, "format").unwrap_or("markdown").to_ascii_lowercase();
        let max_chars = int_arg(args, "max_chars").map(|n| n.max(500) as usize).unwrap_or(DEFAULT_MAX_CHARS);
        let response = match self.http.get(url).set("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8").call() {
            Ok(r) => r,
            Err(ureq::Error::Status(code, _)) => {
                let busy = matches!(code, 429 | 502 | 503 | 504);
                let more = if busy { " The server is busy; try again in a moment." } else { "" };
                return ToolResult::err(format!("{url} returned HTTP {code}.{more}"));
            }
            Err(e) => {
                let text = e.to_string();
                if let Some(at) = text.find(BLOCKED) {
                    let why = text[at + BLOCKED.len()..].lines().next().unwrap_or("").to_string();
                    return ToolResult::err(format!("Refused: {why}. fetch_url reads public pages only (redirects included)."));
                }
                return ToolResult::err(format!("Could not reach {url}: {text}"));
            }
        };
        let final_url = response.get_url().to_string();
        let content_type = response.content_type().to_ascii_lowercase();
        let mut bytes = Vec::new();
        if response.into_reader().take(MAX_DOWNLOAD + 1).read_to_end(&mut bytes).is_err() {
            return ToolResult::err(format!("{final_url}: the page stopped arriving part way."));
        }
        if bytes.len() as u64 > MAX_DOWNLOAD {
            return ToolResult::err(format!("{final_url} is over {} MB: too large to read.", MAX_DOWNLOAD / 1_048_576));
        }
        let Some(body) = decode(&bytes, &content_type) else {
            return ToolResult::err(format!("{final_url} is not text ({content_type})."));
        };
        let html_like = content_type.contains("html") || content_type.is_empty() || body.trim_start().starts_with('<');
        let rendered = match format.as_str() {
            "html" => body.clone(),
            "text" if html_like => to_text(&body, false),
            _ if html_like => to_text(&body, true),
            _ => body.clone(),
        };
        let (content, truncated) = clip(&rendered, max_chars);
        let mut head = format!("URL: {final_url}");
        if let Some(title) = html_like.then(|| title(&body)).flatten().filter(|t| !t.is_empty()) {
            head.push_str(&format!("\nTitle: {title}"));
        }
        if final_url != url {
            head.push_str(&format!("\n(redirected from {url})"));
        }
        if truncated {
            head.push_str(&format!("\n(truncated at {max_chars} characters — raise max_chars or fetch a narrower page)"));
        }
        if format != "html" && html_like && content.trim().chars().count() < 200 {
            head.push_str("\n(almost no text: this page is likely built by JavaScript, which this tool does not run)");
        }
        ToolResult::ok(format!("{head}\n\n{content}"))
    }
}

/// The body as text: UTF-8, or Latin-1 when the page says so or UTF-8 fails;
/// None for what is plainly not text.
fn decode(bytes: &[u8], content_type: &str) -> Option<String> {
    let textual = content_type.is_empty()
        || content_type.starts_with("text/")
        || ["xml", "json", "javascript", "html"].iter().any(|t| content_type.contains(t));
    if !textual || bytes.contains(&0) {
        return None;
    }
    match String::from_utf8(bytes.to_vec()) {
        Ok(text) => Some(text),
        Err(_) => Some(bytes.iter().map(|&b| b as char).collect()),
    }
}

fn clip(text: &str, max_chars: usize) -> (String, bool) {
    match text.char_indices().nth(max_chars) {
        Some((at, _)) => (text[..at].to_string(), true),
        None => (text.to_string(), false),
    }
}

/// Pure — unit-tested. The page's `<title>`.
pub fn title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let open = lower.find("<title")?;
    let start = open + lower[open..].find('>')? + 1;
    let end = start + lower[start..].find("</title")?;
    Some(squeeze(&decode_entities(&strip_tags(&html[start..end]))).replace('\n', " "))
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for c in s.chars() {
        match c {
            '<' => inside = true,
            '>' if inside => {
                inside = false;
                out.push(' ');
            }
            _ if !inside => out.push(c),
            _ => {}
        }
    }
    out
}

/// Elements whose content is machinery or chrome, not text.
const SKIPPED: &[&str] = &["script", "style", "noscript", "svg", "template", "iframe", "canvas", "head"];
/// Elements whose end is a paragraph break.
const BLOCKS: &[&str] = &[
    "p",
    "div",
    "section",
    "article",
    "header",
    "footer",
    "tr",
    "ul",
    "ol",
    "dl",
    "blockquote",
    "pre",
    "table",
    "figure",
    "main",
    "aside",
    "nav",
];

/// Pure — unit-tested. Visible text; with `markdown`, also the structure that
/// carries meaning: headings, list items, links, image alt text.
pub fn to_text(html: &str, markdown: bool) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::new();
    // The open link: where its text starts in `out`, and its address.
    let mut link: Option<(usize, String)> = None;
    let mut i = 0;
    while i < html.len() {
        if lower[i..].starts_with("<!--") {
            i = lower[i..].find("-->").map(|e| i + e + 3).unwrap_or(html.len());
            out.push(' ');
            continue;
        }
        if !html[i..].starts_with('<') {
            let end = html[i..].find('<').map(|e| i + e).unwrap_or(html.len());
            out.push_str(&decode_entities(&html[i..end]));
            i = end;
            continue;
        }
        let Some(close) = html[i..].find('>').map(|e| i + e) else {
            out.push_str(&decode_entities(&html[i..]));
            break;
        };
        let inner = &html[i + 1..close];
        let closing = inner.starts_with('/');
        let name: String =
            inner.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
        if name.is_empty() && !inner.starts_with('!') {
            out.push('<'); // a bare "<", as in "a < b"
            i += 1;
            continue;
        }
        i = close + 1;
        let name = name.as_str();
        if !closing && SKIPPED.contains(&name) && !inner.trim_end().ends_with('/') {
            let end_tag = format!("</{name}");
            i = lower[i..].find(&end_tag).and_then(|e| lower[i + e..].find('>').map(|g| i + e + g + 1)).unwrap_or(html.len());
            out.push(' ');
            continue;
        }
        let heading =
            name.len() == 2 && name.starts_with('h') && name.as_bytes()[1].is_ascii_digit() && (b'1'..=b'6').contains(&name.as_bytes()[1]);
        match (closing, name) {
            (_, "br") => out.push('\n'),
            (true, n) if BLOCKS.contains(&n) => out.push_str("\n\n"),
            (false, "li") => out.push_str(if markdown { "\n- " } else { "\n" }),
            (false, "td" | "th") => out.push_str(if markdown { " | " } else { " " }),
            (false, _) if heading => {
                out.push_str("\n\n");
                if markdown {
                    out.push_str(&"#".repeat((name.as_bytes()[1] - b'0') as usize));
                    out.push(' ');
                }
            }
            (true, _) if heading => out.push_str("\n\n"),
            (false, "a") if markdown => match attribute(inner, "href") {
                Some(href) => link = Some((out.len(), href)),
                None => out.push(' '),
            },
            (true, "a") if markdown => match link.take() {
                Some((start, href)) => {
                    let text = squeeze(&out[start..]).replace('\n', " ");
                    out.truncate(start);
                    if !text.is_empty() {
                        out.push_str(&format!("[{text}]({href})"));
                    }
                }
                None => out.push(' '),
            },
            (false, "img") if markdown => {
                if let Some(alt) = attribute(inner, "alt").filter(|a| !a.trim().is_empty()) {
                    out.push_str(&format!("![{alt}]"));
                }
            }
            _ => out.push(' '),
        }
    }
    let text = squeeze(&out);
    if !markdown {
        return text;
    }
    // Decorative headings survive as a bare "###": they carry nothing.
    let kept: Vec<&str> = text.split('\n').filter(|l| !(l.starts_with('#') && l.trim_start_matches('#').trim().is_empty())).collect();
    squeeze(&kept.join("\n"))
}

/// The value of attribute `name` in a tag's inside, quoted or not.
fn attribute(inner: &str, name: &str) -> Option<String> {
    let lower = inner.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find(name).map(|a| from + a) {
        from = at + name.len();
        let before_ok = at == 0 || lower.as_bytes()[at - 1].is_ascii_whitespace();
        let rest = inner[from..].trim_start();
        if !before_ok || !rest.starts_with('=') {
            continue;
        }
        let value = rest[1..].trim_start();
        let found = match value.chars().next() {
            Some(q @ ('"' | '\'')) => value[1..].split(q).next().unwrap_or(""),
            _ => value.split(|c: char| c.is_whitespace() || c == '>').next().unwrap_or(""),
        };
        return Some(decode_entities(found));
    }
    None
}

/// Runs of spaces collapsed, lines trimmed, never two blank lines in a row.
fn squeeze(s: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for line in s.split('\n') {
        let line = line.split([' ', '\t', '\r', '\u{a0}']).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ");
        if line.is_empty() && out.last().is_none_or(|l| l.is_empty()) {
            continue;
        }
        out.push(line);
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    out.join("\n")
}

/// Pure — unit-tested. HTML entities: the common named ones and numeric.
pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    const NAMED: &[(&str, &str)] = &[
        ("nbsp", " "),
        ("amp", "&"),
        ("lt", "<"),
        ("gt", ">"),
        ("quot", "\""),
        ("apos", "'"),
        ("mdash", "—"),
        ("ndash", "–"),
        ("hellip", "…"),
        ("rsquo", "’"),
        ("lsquo", "‘"),
        ("ldquo", "“"),
        ("rdquo", "”"),
        ("copy", "©"),
    ];
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let entity = rest[1..].find(';').filter(|&e| e > 0 && e <= 10).map(|e| &rest[1..=e]);
        let decoded = entity.and_then(|e| {
            if let Some(num) = e.strip_prefix('#') {
                let code = match num.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => num.parse().ok(),
                };
                code.and_then(char::from_u32).map(String::from)
            } else {
                NAMED.iter().find(|(n, _)| *n == e).map(|(_, v)| v.to_string())
            }
        });
        match (entity, decoded) {
            (Some(e), Some(text)) => {
                out.push_str(&text);
                rest = &rest[e.len() + 2..];
            }
            _ => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_and_reserved_addresses_are_refused() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "172.20.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(private_ip(ip.parse().unwrap()).is_some(), "{ip}");
        }
        for ip in ["93.184.216.34", "1.1.1.1", "172.32.0.1", "2606:4700::1111"] {
            assert_eq!(private_ip(ip.parse().unwrap()), None, "{ip}");
        }
    }

    #[test]
    fn a_name_is_refused_when_any_of_its_addresses_is_private() {
        let fake = |answer: &'static [&'static str]| {
            move |_: &str| -> std::io::Result<Vec<SocketAddr>> {
                Ok(answer.iter().map(|a| SocketAddr::new(a.parse().unwrap(), 80)).collect())
            }
        };
        assert!(public_addresses("example.com:443", &fake(&["93.184.216.34"])).is_ok());
        let mixed = public_addresses("evil.example:80", &fake(&["93.184.216.34", "127.0.0.1"])).unwrap_err().to_string();
        assert!(mixed.contains(BLOCKED) && mixed.contains("127.0.0.1"), "{mixed}");
        assert!(public_addresses("printer.local:80", &fake(&["93.184.216.34"])).is_err(), "local names before any lookup");
        assert!(public_addresses("[::1]:80", &fake(&["::1"])).is_err());
        assert!(public_addresses("nowhere.example:80", &fake(&[])).is_err());
    }

    #[test]
    fn only_public_web_addresses_are_tried() {
        assert!(check_url("https://example.com/a?b").is_ok());
        assert!(check_url("file:///etc/passwd").unwrap_err().contains("file://"));
        assert!(check_url("http://localhost:8080/").unwrap_err().contains("public pages only"));
        assert!(check_url("http://user@127.0.0.1/").is_err());
        assert!(check_url("http://[::1]:80/").is_err());
        assert!(check_url("example.com").unwrap_err().contains("http://"));
        assert!(check_url("https:///path").unwrap_err().contains("no host"));
    }

    #[test]
    fn a_page_becomes_markdown_with_its_structure() {
        let html = "<html><head><title>Doc &amp; more</title><style>p{}</style></head><body>\
            <nav><a href=\"/\"><svg>icon</svg></a></nav><h1>Intro</h1><p>Hello&nbsp;<b>world</b> &#8212; see \
            <a href='https://x.dev/a'>the docs</a>.</p><script>alert(1)</script><!-- note --><ul><li>one</li><li>two</li></ul>\
            <img src=x alt=\"A chart\"><h3> </h3><table><tr><td>a</td><td>b</td></tr></table><p>1 < 2</p></body></html>";
        assert_eq!(title(html).as_deref(), Some("Doc & more"));
        let md = to_text(html, true);
        assert_eq!(
            md, "# Intro\n\nHello world — see [the docs](https://x.dev/a).\n\n- one\n- two\n\n![A chart]\n\n| a | b\n\n1 < 2",
            "{md}"
        );
        let text = to_text(html, false);
        assert!(text.contains("Hello world — see the docs .") && !text.contains('#') && !text.contains("alert"), "{text}");
    }

    #[test]
    fn entities_decode_and_broken_ones_stay() {
        assert_eq!(decode_entities("a &lt;b&gt; &#65;&#x42; &unknown; & done"), "a <b> AB &unknown; & done");
    }

    #[test]
    fn the_tool_refuses_before_any_request() {
        let tool = FetchUrl::new();
        let r = tool.run(&json!({ "url": "http://127.0.0.1:9/" }));
        assert!(!r.ok && r.text.contains("public pages only"), "{}", r.text);
        assert!(!tool.run(&json!({})).ok);
    }

    /// Real network: `cargo test -- --ignored live_fetch --nocapture`.
    #[test]
    #[ignore]
    fn live_fetch() {
        let tool = FetchUrl::new();
        let page = tool.run(&json!({ "url": "https://example.com/" }));
        println!("{}\n---", page.text);
        assert!(page.ok && page.text.contains("Title: Example Domain"), "{}", page.text);
        // A public name that resolves to this machine: refused by the
        // resolver the connection uses (the same check every redirect gets).
        let local = tool.run(&json!({ "url": "http://localtest.me:9/" }));
        println!("{}", local.text);
        assert!(!local.ok && local.text.contains("Refused") && local.text.contains("127.0.0.1"), "{}", local.text);
    }
}

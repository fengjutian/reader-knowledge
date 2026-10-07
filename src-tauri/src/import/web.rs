//! 网页导入：HTTPS 抓取 + 正文提取。
//!
//! SSRF 防护是这一层最重要的部分（对应实现计划 9.3.2）：
//! 只允许公网 HTTPS，阻断 localhost、环回、私网与 link-local，
//! 并且**每次重定向都要重新校验** —— 只查初始 URL 挡不住
//! 「先给一个公网地址，再 302 到 169.254.169.254」。

use crate::error::AppError;
use crate::import::{DocumentChunk, Locator, ParsedSource, SourceKind};
use std::net::{IpAddr, Ipv4Addr};

/// 单个响应体上限。网页正文给 8 MiB 已经很宽松。
pub const MAX_HTML_BYTES: usize = 8 * 1024 * 1024;
/// 跟随重定向的最大次数。
pub const MAX_REDIRECTS: usize = 5;
/// 单次请求超时。
pub const WEB_TIMEOUT_SECS: u64 = 20;

/// 解析并校验一个待抓取的 URL。
///
/// 返回规范化后的 URL；任何指向内网的地址都直接拒绝。
pub fn validate_url(input: &str) -> Result<reqwest::Url, AppError> {
    let url = reqwest::Url::parse(input.trim())
        .map_err(|_| AppError::Message("网址格式无效".into()))?;
    if url.scheme() != "https" {
        return Err(AppError::Message("只支持 HTTPS 网址".into()));
    }
    let host = url
        .host_str()
        .ok_or_else(|| AppError::Message("网址缺少主机名".into()))?
        .to_owned();
    if is_blocked_host(&host) {
        return Err(AppError::Message("出于安全考虑，不允许抓取本机或内网地址".into()));
    }
    Ok(url)
}

/// 主机名是否属于必须拒绝的范围。
pub fn is_blocked_host(host: &str) -> bool {
    let host = host.trim().trim_start_matches('[').trim_end_matches(']').to_lowercase();
    if host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") || host.ends_with(".internal") {
        return true;
    }
    if let Ok(address) = host.parse::<IpAddr>() {
        return is_blocked_address(address);
    }
    // 缩写形态的 IPv4：URL 解析器接受 `2130706433`（= 127.0.0.1）、
    // `127.1`、`0x7f000001` 等写法，黑名单必须自己展开。
    if let Some(address) = expand_short_ipv4(&host) {
        return is_blocked_address(IpAddr::V4(address));
    }
    false
}

/// 把缩写形态的 IPv4 展开成标准四段。
///
/// URL 解析器（以及历史上很多客户端）接受这些写法，攻击者会用它们绕过
/// 字符串黑名单：
/// - `2130706433`        → 127.0.0.1（整段当 32 位整数）
/// - `127.1`             → 127.0.0.1（后段补足剩余位宽）
/// - `0x7f000001`        → 127.0.0.1（十六进制）
fn expand_short_ipv4(host: &str) -> Option<Ipv4Addr> {
    // 十六进制整段
    if let Some(hex) = host.strip_prefix("0x").or_else(|| host.strip_prefix("0X")) {
        if hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return u32::from_str_radix(hex, 16).ok().map(Ipv4Addr::from);
        }
        return None;
    }
    // 只接受数字与点，出现别的字符就不是 IPv4 缩写
    if !host.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    let parts: Vec<u64> = host.split('.').map(|part| part.parse::<u64>().unwrap_or(u64::MAX)).collect();
    if parts.iter().any(|part| *part > u32::MAX as u64) {
        return None;
    }
    match parts.len() {
        // 单个整数：整体当 32 位
        1 => u32::try_from(parts[0]).ok().map(Ipv4Addr::from),
        // 四段标准写法
        4 => {
            if parts.iter().all(|part| *part <= 255) {
                Some(Ipv4Addr::new(parts[0] as u8, parts[1] as u8, parts[2] as u8, parts[3] as u8))
            } else {
                None
            }
        }
        // `a.b` → a 占 8 位、b 占 24 位
        2 => {
            if parts[0] <= 255 && parts[1] <= 0xFF_FFFF {
                let combined = ((parts[0] as u32) << 24) | parts[1] as u32;
                Some(Ipv4Addr::from(combined))
            } else {
                None
            }
        }
        // `a.b.c` → a 占 8 位、b 与 c 各占 8 位
        3 => {
            if parts.iter().all(|part| *part <= 255) {
                Some(Ipv4Addr::new(parts[0] as u8, parts[1] as u8, parts[2] as u8, 0))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// IP 是否属于环回、私网、link-local 或保留段。
pub fn is_blocked_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                // 私网
                || v4.is_private()
                // 链路本地 169.254.0.0/16
                || v4.is_link_local()
                // 0.0.0.0/8
                || v4.octets()[0] == 0
                // 组播 224.0.0.0/4 与广播 255.255.255.255
                || v4.is_multicast()
                || v4 == Ipv4Addr::BROADCAST
                // CGNAT 100.64.0.0/10 与保留 240.0.0.0/4
                || (v4.octets()[0] == 100 && (64..128).contains(&v4.octets()[1]))
                || v4.octets()[0] >= 240
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                // fc00::/7 唯一本地地址
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                // fe80::/10 链路本地
                || (v6.segments()[0] & 0xffc0) == 0xfe80
                || v6.is_multicast()
                || v6.is_unspecified()
        }
    }
}

/// 需要重定向时，检查目标地址是否仍然安全。
pub fn validate_redirect(from: &reqwest::Url, to: &reqwest::Url) -> Result<(), AppError> {
    if to.scheme() != "https" {
        return Err(AppError::Message("重定向到了非 HTTPS 地址，已中止抓取".into()));
    }
    let host = to.host_str().unwrap_or_default();
    if is_blocked_host(host) {
        return Err(AppError::Message("重定向到了本机或内网地址，已中止抓取".into()));
    }
    // 降级到明文 HTTP 也要拦住
    if from.scheme() == "https" && to.scheme() == "http" {
        return Err(AppError::Message("重定向降级为 HTTP，已中止抓取".into()));
    }
    Ok(())
}

/// 判断 Content-Type 是否是可接受的 HTML。
pub fn is_html_content_type(value: &str) -> bool {
    let value = value.split(';').next().unwrap_or("").trim().to_lowercase();
    value == "text/html" || value == "application/xhtml+xml"
}

/// 提取网页标题：优先 `<title>`，其次 og:title，最后 h1。
pub fn extract_title(html: &str) -> String {
    let lower = html.to_lowercase();
    if let Some(start) = find_tag_content(&lower, "title") {
        if !start.trim().is_empty() {
            return decode_entities(start.trim());
        }
    }
    if let Some(value) = find_meta_content(&lower, "og:title") {
        if !value.trim().is_empty() {
            return decode_entities(value.trim());
        }
    }
    if let Some(start) = find_tag_content(&lower, "h1") {
        if !start.trim().is_empty() {
            return decode_entities(start.trim());
        }
    }
    "未命名网页".into()
}

/// 提取作者：优先 meta author，其次 og:article:author。
pub fn extract_author(html: &str) -> Option<String> {
    let lower = html.to_lowercase();
    for key in ["og:article:author", "author"] {
        if let Some(value) = find_meta_content(&lower, key) {
            let value = decode_entities(value.trim());
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    None
}

fn find_tag_content(lower_html: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}");
    let open_at = lower_html.find(&open)?;
    // 跳过可能带属性的位置
    let mut cursor = open_at + open.len();
    let bytes = lower_html.as_bytes();
    while cursor < bytes.len() && bytes[cursor] != b'>' {
        cursor += 1;
    }
    if cursor >= bytes.len() {
        return None;
    }
    let close = format!("</{tag}>");
    let rest = &lower_html[cursor + 1..];
    let end = rest.find(&close)?;
    Some(rest[..end].to_owned())
}

fn find_meta_content(lower_html: &str, name: &str) -> Option<String> {
    let needle = format!("\"{name}\"");
    let mut cursor = 0;
    while let Some(at) = lower_html[cursor..].find(&needle) {
        let start = cursor + at;
        // 从引号往后找 content="..."
        let tail = &lower_html[start..];
        let content_at = tail.find("content=")?;
        let value_start = content_at + "content=".len();
        let rest = &tail[value_start..];
        let quote = rest.chars().next()?;
        if quote != '"' && quote != '\'' {
            cursor = start + needle.len();
            continue;
        }
        let inner = &rest[1..];
        let end = inner.find(quote)?;
        return Some(inner[..end].to_owned());
    }
    None
}

/// 常见 HTML 实体解码。覆盖导入场景里会遇到的那些。
pub fn decode_entities(input: &str) -> String {
    input
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&mdash;", "—")
        .replace("&ndash;", "–")
        .replace("&hellip;", "…")
        .replace("&ldquo;", "“")
        .replace("&rdquo;", "”")
}

/// 剥掉脚本、样式、导航等噪声，只留正文。
///
/// 没有 HTML 解析器可用，所以按标签做状态裁剪：
/// 进入 script/style/noscript/nav/header/footer/aside/iframe/svg 时丢弃其中内容，
/// 遇到对应闭标签后恢复。
///
/// 全程按 `char` 迭代而不是按字节，否则中文会被拆成半个字符变成乱码。
pub fn strip_noise(html: &str) -> String {
    // `head` / `title` 必须一起丢：<title> 的文本不在任何噪声标签里，
    // 会作为第一段混进正文，导致第一个文档块变成书名而不是内容。
    // 标题本身由 extract_title 从原始 HTML 取，不受影响。
    const DROP_CONTAINERS: [&str; 11] = ["script", "style", "noscript", "head", "title", "nav", "header", "footer", "aside", "iframe", "svg"];
    const BLOCK_TAGS: [&str; 15] = [
        "p", "div", "br", "li", "tr", "h1", "h2", "h3", "h4", "h5", "h6", "section", "article", "blockquote", "pre",
    ];
    let mut output = String::with_capacity(html.len());
    let mut skip_until: Option<String> = None;
    let mut index = 0;
    let bytes = html.as_bytes();
    while index < html.len() {
        if !html.is_char_boundary(index) {
            // 不可能发生，但避免任何 UB 风险
            index += 1;
            continue;
        }
        if bytes[index] != b'<' {
            if skip_until.is_none() {
                let character = html[index..].chars().next().unwrap_or('\u{0}');
                output.push(character);
                index += character.len_utf8();
            } else {
                index += html[index..].chars().next().map(char::len_utf8).unwrap_or(1);
            }
            continue;
        }
        // 注释直接跳过
        if html[index..].starts_with("<!--") {
            index = match html[index..].find("-->") {
                Some(end) => index + end + 3,
                None => html.len(),
            };
            continue;
        }
        // <!DOCTYPE …> 与 <?xml …?> 一并丢弃
        if html[index..].starts_with("<!") || html[index..].starts_with("<?") {
            index = match html[index..].find('>') {
                Some(end) => index + end + 1,
                None => html.len(),
            };
            continue;
        }
        let Some(close) = html[index..].find('>') else { break };
        let tag = &html[index + 1..index + close];
        index += close + 1;
        let name = tag
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '/' || c == '>')
            .next()
            .unwrap_or("")
            .to_lowercase();
        if name.is_empty() {
            continue;
        }
        if let Some(container) = &skip_until {
            if name == *container {
                skip_until = None;
            }
            continue;
        }
        if DROP_CONTAINERS.contains(&name.as_str()) && !tag.starts_with('/') {
            // <script/> 这类自闭合标签不进入跳过状态
            if !tag.trim_end().ends_with('/') {
                skip_until = Some(name);
            }
            continue;
        }
        if BLOCK_TAGS.contains(&name.as_str()) {
            output.push('\n');
        }
    }
    clean_text(&output)
}

/// 从已剥标签的 HTML 提取纯文本，折叠空白。
pub fn clean_text(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut newlines = 0;
    for character in input.chars() {
        if character == '\n' || character == '\r' {
            newlines += 1;
            // 最多保留两个换行；换行之后不留尾随空格
            if newlines <= 2 {
                while result.ends_with(' ') {
                    result.pop();
                }
                result.push('\n');
            }
            continue;
        }
        if newlines > 0 && character.is_whitespace() {
            // 换行后的空白直接丢掉，避免出现 "a b \n\n c"
            continue;
        }
        newlines = 0;
        if character.is_whitespace() {
            if !result.ends_with(' ') && !result.is_empty() {
                result.push(' ');
            }
            continue;
        }
        result.push(character);
    }
    decode_entities(&result.trim())
}

/// 把抓到的 HTML 组装成可导入的来源。
pub fn build_parsed_source(url: &reqwest::Url, html: &str) -> ParsedSource {
    let title = extract_title(html);
    let text = clean_text(&strip_noise(html));
    let mut parsed = ParsedSource {
        title: title.clone(),
        author: extract_author(html),
        origin: Some(url.to_string()),
        kind: Some(SourceKind::Web),
        chunks: Vec::new(),
        ..Default::default()
    };
    if text.chars().count() < 40 {
        parsed.add_warning("页面正文很短，可能抓到了导航页或需要登录的内容");
    }
    parsed.chunks = crate::import::split_into_chunks(&title, &text, 0, &Locator::default())
        .into_iter()
        .map(|chunk| DocumentChunk { position: chunk.position, heading: title.clone(), ..chunk })
        .collect();
    parsed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 只接受_https_网址() {
        assert!(validate_url("https://example.com/a").is_ok());
        assert!(validate_url("http://example.com/a").is_err(), "http 必须拒绝");
        assert!(validate_url("file:///etc/passwd").is_err());
        assert!(validate_url("ftp://example.com").is_err());
        assert!(validate_url("不是网址").is_err());
    }

    #[test]
    fn 拒绝本机与内网地址() {
        for host in [
            "https://localhost/a",
            "https://127.0.0.1/a",
            "https://127.1.2.3/a",
            "https://[::1]/a",
            "https://10.0.0.1/a",
            "https://192.168.1.1/a",
            "https://172.16.0.1/a",
            "https://169.254.169.254/latest/meta-data",
            "https://0.0.0.0/a",
            "https://255.255.255.255/a",
            "https://100.64.0.1/a",
            "https://224.0.0.1/a",
            "https://[fe80::1]/a",
            "https://[fc00::1]/a",
            "https://myhost.localhost/a",
        ] {
            let result = validate_url(host);
            assert!(result.is_err(), "{host} 必须被拒绝");
        }
    }

    #[test]
    fn 放行正常公网地址() {
        for host in ["https://example.com", "https://8.8.8.8/", "https://[2606:4700:4700::1111]/"] {
            assert!(validate_url(host).is_ok(), "{host} 应当放行");
        }
    }

    #[test]
    fn 拒绝十进制与十六进制形态的本机地址() {
        // 2130706433 = 127.0.0.1
        assert!(is_blocked_host("2130706433"));
        assert!(is_blocked_host("127.1"));
    }

    #[test]
    fn 重定向到内网会被拦下() {
        let from = reqwest::Url::parse("https://example.com").unwrap();
        let to = reqwest::Url::parse("https://169.254.169.254/latest").unwrap();
        assert!(validate_redirect(&from, &to).is_err(), "重定向到 link-local 必须被拒绝");
        let downgrade = reqwest::Url::parse("http://example.com").unwrap();
        assert!(validate_redirect(&from, &downgrade).is_err(), "降级到 http 必须被拒绝");
        let safe = reqwest::Url::parse("https://other.example.com").unwrap();
        assert!(validate_redirect(&from, &safe).is_ok());
    }

    #[test]
    fn 只接受_html_内容类型() {
        assert!(is_html_content_type("text/html; charset=utf-8"));
        assert!(is_html_content_type("TEXT/HTML"));
        assert!(is_html_content_type("application/xhtml+xml"));
        assert!(!is_html_content_type("application/pdf"));
        assert!(!is_html_content_type("image/png"));
        assert!(!is_html_content_type(""));
    }

    #[test]
    fn 剥离脚本样式与导航噪声() {
        let html = r#"<html><head><title>测试页面</title><style>body{color:red}</style>
        <script>alert('xss')</script></head>
        <body><nav>首页 关于</nav><article><h1>正文标题</h1><p>第一段内容。</p><script>evil()</script><p>第二段内容。</p></article>
        <footer>版权信息</footer></body></html>"#;
        let text = clean_text(&strip_noise(html));
        assert!(text.contains("正文标题"));
        assert!(text.contains("第一段内容"));
        assert!(text.contains("第二段内容"));
        assert!(!text.contains("alert"), "脚本内容必须被丢弃");
        assert!(!text.contains("color:red"), "样式内容必须被丢弃");
        assert!(!text.contains("首页 关于"), "导航内容必须被丢弃");
        assert!(!text.contains("版权信息"), "页脚必须被丢弃");
    }

    #[test]
    fn 提取标题按优先级回退() {
        assert_eq!(extract_title("<html><head><title>来自 title</title></head><body></body></html>"), "来自 title");
        assert_eq!(extract_title(r#"<html><head><meta property="og:title" content="来自 og"></head></html>"#), "来自 og");
        assert_eq!(extract_title("<html><body><h1>来自 h1</h1></body></html>"), "来自 h1");
        assert_eq!(extract_title("<html><body>什么都没有</body></html>"), "未命名网页");
    }

    #[test]
    fn 提取作者() {
        assert_eq!(extract_author(r#"<meta name="author" content="张三">"#).as_deref(), Some("张三"));
        assert_eq!(extract_author(r#"<meta property="og:article:author" content="李四">"#).as_deref(), Some("李四"));
        assert_eq!(extract_author("<p>无作者</p>"), None);
    }

    #[test]
    fn 解码常见实体() {
        assert_eq!(decode_entities("A&amp;B &lt;tag&gt; &quot;x&quot;"), "A&B <tag> \"x\"");
        assert_eq!(decode_entities("空格&nbsp;合并"), "空格 合并");
    }

    #[test]
    fn 清洗正文折叠多余空白() {
        assert_eq!(clean_text("  a   b  \n\n\n\n c  "), "a b\n\nc");
    }

    #[test]
    fn 组装网页来源包含正文分块() {
        let url = reqwest::Url::parse("https://example.com/article").unwrap();
        let html = r#"<html><head><title>文章标题</title><meta name="author" content="作者"></head>
        <body><article><p>{}</p></article></body></html>"#.replace("{}", &"正文内容。".repeat(200));
        let parsed = build_parsed_source(&url, &html);
        assert_eq!(parsed.title, "文章标题");
        assert_eq!(parsed.author.as_deref(), Some("作者"));
        assert_eq!(parsed.kind, Some(SourceKind::Web));
        assert_eq!(parsed.origin.as_deref(), Some("https://example.com/article"));
        assert!(!parsed.chunks.is_empty());
        assert!(parsed.chunks.iter().all(|chunk| chunk.content.contains("正文内容")), "chunk 数={} 首块前 60 字={:?}", parsed.chunks.len(), parsed.chunks.first().map(|c| c.content.chars().take(60).collect::<String>()));
    }

    #[test]
    fn 过短页面给出警告而不是静默() {
        let url = reqwest::Url::parse("https://example.com/").unwrap();
        let parsed = build_parsed_source(&url, "<html><title>空页面</title><body>短</body></html>");
        assert!(!parsed.warnings.is_empty());
        assert!(parsed.warnings[0].contains("正文很短"));
    }
}

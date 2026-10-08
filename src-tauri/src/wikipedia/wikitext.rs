//! Wikitext -> 纯文本摘要。
//!
//! 需求 7.4 明确不允许用一个脆弱的正则解析全部嵌套模板，所以这里是一个
//! 逐字符状态机：`{{ }}`、`[[ ]]`、`<tag>` 都按嵌套深度配对，只对少量
//! 明确安全的模板做展开，其余模板整段丢弃。
//!
//! 输出永远是纯文本：不含 HTML 标签、不含模板语法，前端按文本渲染是安全的。

/// 摘要长度策略。min_chars 是希望达到的下限（不够就多吞几段），
/// max_chars 是硬上限，截断优先落在自然句读边界上。
#[derive(Debug, Clone, Copy)]
pub struct SummaryOptions {
    pub min_chars: usize,
    pub max_chars: usize,
}

impl Default for SummaryOptions {
    fn default() -> Self {
        Self { min_chars: 300, max_chars: 600 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummaryQuality {
    /// 首段完整，未截断。
    Complete,
    /// 超长，在句读边界截断。
    Truncated,
    /// 整页正文都不到 min_chars，属于短条目。
    Short,
    /// 模板/表格占比过高，摘要可信度低，建议人工抽查。
    Degraded,
}

impl SummaryQuality {
    pub fn as_str(&self) -> &'static str {
        match self {
            SummaryQuality::Complete => "complete",
            SummaryQuality::Truncated => "truncated",
            SummaryQuality::Short => "short",
            SummaryQuality::Degraded => "degraded",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CleanedSummary {
    pub summary: String,
    pub quality: SummaryQuality,
    /// 原始文本与清洗后文本的长度比，值越大说明被剥掉的脚手架越多。
    pub discarded_ratio: f64,
}

/// 内容为空的标签：整段丢掉，含标签本身。
const DROP_WITH_CONTENT: &[&str] = &[
    "ref",
    "references",
    "reflist",
    "gallery",
    "imagemap",
    "imagemapframe",
    "math",
    "syntaxhighlight",
    "source",
    "pre",
    "nowiki",
    "timeline",
    "score",
    "templatestyles",
    "poem",
    "graph",
    "mapframe",
    "maplink",
    "section",
    "includeonly",
    "noinclude",
    "onlyinclude",
    "sup",
    "sub",
    "references-list",
    "imagemap-inline",
];

/// 只丢掉标签本身、保留内容的标签。
const INLINE_TAGS: &[&str] = &[
    "b", "i", "u", "s", "span", "center", "small", "big", "bdo", "abbr", "cite", "q", "font",
    "div", "p", "li", "ul", "ol", "dd", "dt", "dl", "tr", "td", "th", "table", "caption",
    "blockquote", "h1", "h2", "h3", "h4", "h5", "h6", "section", "div col", "nowiki",
];

/// 会产生换行的标签。
const BLOCK_TAGS: &[&str] = &[
    "br", "p", "div", "li", "ul", "ol", "tr", "dd", "dt", "dl", "h1", "h2", "h3", "h4", "h5",
    "h6", "caption", "blockquote", "pre", "table",
];

/// 名字命中前缀就整段丢弃的模板（信息框、导航框、维护模板、歧义提示等）。
const DROPPED_TEMPLATE_PREFIXES: &[&str] = &[
    "infobox",
    "导航",
    "navbox",
    "sidebar",
    "sistersubbox",
    "subject bar",
    "commons category",
    "maintenance",
    "cleanup",
    "unreferenced",
    "orphan",
    "stub",
    "featured article",
    "good article",
    "featured list",
    "short description",
    "about",
    "for",
    "other uses",
    "hatnote",
    "coord",
    "position",
    "clear",
    "columns-list",
    "div col",
    "portal",
    "wikidata",
    "authority control",
    "taxonbar",
    "automatic navigation",
    "use dmy dates",
    "uselang",
    "维基百科",
    "重定向",
    "redirect",
    "参见",
    "参考文献",
    "reference",
    "外部链接",
    "external links",
    "延伸阅读",
    "further reading",
    "注释",
    "notes",
    "来源",
    "bibliography",
    "书目",
    "扩展阅读",
    "相关",
    "另见",
    "注釋",
    "參考文獻",
];

/// 名字命中前缀就整段丢弃的文件/分类/图片类链接。
const DROPPED_LINK_PREFIXES: &[&str] = &[
    "file:",
    "image:",
    "media:",
    "video:",
    "文件:",
    "檔案:",
    "图像:",
    "圖像:",
    "媒体:",
    "category:",
    "分类:",
    "分類:",
    "categorytree:",
    "主题分类:",
    "mediawiki:",
    "template:",
    "模板:",
    "module:",
    "模块:",
    "help:",
    "帮助:",
    "wikipedia:",
];

/// 首字母类：zhwiki 常见写法。
const SOFT_REDIRECT_MARKERS: &[&str] = &["#redirect", "#重定向", "#轉向"];

/// 主入口：把一段 wikitext 变成可展示的纯文本摘要。
pub fn extract_summary(wikitext: &str, options: SummaryOptions) -> CleanedSummary {
    let chars: Vec<char> = wikitext.chars().collect();
    let raw_len = chars.len().max(1);
    let plain = to_plain_text(&chars);
    let kept_len = plain.chars().count();
    let discarded_ratio = 1.0 - (kept_len as f64 / raw_len as f64);
    let (summary, truncated) = select_summary(&plain, options);
    let length = summary.chars().count();
    // 质量判定看“被剥掉的比例”，而不是整页长度占比：章节标题之后的正文
    // 本来就不进摘要，拿整页算会把正常的长条目全判成 degraded。
    let quality = if raw_len > 400 && discarded_ratio > 0.6 && length * 100 / raw_len < 40 {
        // lead 区里超过六成是模板/表格/引用，摘要很可能是脚手架而不是正文。
        SummaryQuality::Degraded
    } else if truncated {
        SummaryQuality::Truncated
    } else if length < options.min_chars {
        SummaryQuality::Short
    } else {
        SummaryQuality::Complete
    };
    CleanedSummary { summary, quality, discarded_ratio }
}

/// 词条里出现软重定向时返回目标标题（需求 7.7 要求过滤软重定向，这里提供识别能力）。
pub fn soft_redirect_target(wikitext: &str) -> Option<String> {
    let first_line = wikitext.lines().next().unwrap_or_default().trim();
    let lowered = first_line.to_lowercase();
    let marker = SOFT_REDIRECT_MARKERS
        .iter()
        .find(|marker| lowered.starts_with(**marker))?;
    let rest = first_line[marker.len()..].trim();
    let target = rest
        .trim_start_matches("[[")
        .trim_end_matches("]]")
        .split(['|', '\n', '#'])
        .next()
        .unwrap_or_default()
        .trim();
    (!target.is_empty()).then(|| target.to_string())
}

struct Scanner<'a> {
    input: &'a [char],
    index: usize,
    out: String,
    /// 正在被整段丢弃的 HTML 标签名，同时记录嵌套深度。
    skipping: Option<(String, usize)>,
    /// 表格深度，>0 时整段丢弃。
    table_depth: usize,
}

/// 状态机主循环。
fn to_plain_text(input: &[char]) -> String {
    let mut scanner = Scanner { input, index: 0, out: String::new(), skipping: None, table_depth: 0 };
    while scanner.index < scanner.input.len() {
        if let Some((tag, depth)) = scanner.skipping.clone() {
            if scanner.consume_skipping_tag(&tag, depth) {
                continue;
            }
        }
        if scanner.table_depth > 0 {
            scanner.consume_table();
            continue;
        }
        let rest = &scanner.input[scanner.index..];
        if scanner.heading_ahead(rest) {
            // 摘要到第一个章节标题为止，后面的正文不参与。
            break;
        }
        if let Some(consumed) = scanner.consume_comment() {
            scanner.index += consumed;
            continue;
        }
        if let Some(consumed) = scanner.consume_html_tag() {
            scanner.index += consumed;
            continue;
        }
        // 注意：consume_* 自己推进 index，返回值只是这次消耗的字符数。
        if scanner.restarts_with("{{") {
            scanner.consume_template();
            continue;
        }
        if scanner.restarts_with("[[") {
            scanner.consume_internal_link();
            continue;
        }
        if scanner.restarts_with("[") {
            scanner.consume_external_link();
            continue;
        }
        if scanner.restarts_with("'''") {
            scanner.index += 3;
            continue;
        }
        if scanner.restarts_with("''") {
            scanner.index += 2;
            continue;
        }
        if scanner.restarts_with("&") {
            if let Some((text, consumed)) = decode_entity(&scanner.input[scanner.index..]) {
                scanner.out.push_str(&text);
                scanner.index += consumed;
                continue;
            }
        }
        if scanner.restarts_with("{|") || scanner.restarts_with("|-") || scanner.restarts_with("|}") {
            scanner.table_depth = 1;
            scanner.index += 2;
            continue;
        }
        if scanner.restarts_with("----") {
            scanner.push_break();
            scanner.index += 4;
            continue;
        }
        let current = scanner.input[scanner.index];
        if current == '\n' {
            scanner.push_break();
            scanner.index += 1;
            continue;
        }
        // 行首的列表/缩进标记不属于正文，丢掉标记保留文字。
        if matches!(current, '*' | '#' | ':' | ';' | '!') && scanner.at_line_start() {
            scanner.index += 1;
            continue;
        }
        if current == '|' || current == '}' || current == '{' || current == ']' {
            // 表格残片或不配对的定界符，直接丢弃。
            scanner.index += 1;
            continue;
        }
        scanner.out.push(current);
        scanner.index += 1;
    }
    normalize_output(&scanner.out)
}

impl Scanner<'_> {
    fn restarts_with(&self, prefix: &str) -> bool {
        let prefix: Vec<char> = prefix.chars().collect();
        self.input.len() >= self.index + prefix.len()
            && self.input[self.index..self.index + prefix.len()] == prefix[..]
    }

    fn peek(&self, offset: usize) -> Option<char> {
        self.input.get(self.index + offset).copied()
    }

    fn push_break(&mut self) {
        while self.out.ends_with(' ') || self.out.ends_with('\n') {
            self.out.pop();
        }
        if !self.out.is_empty() {
            self.out.push('\n');
        }
    }

    /// 行首判定：目前输出为空或刚换行。
    fn at_line_start(&self) -> bool {
        self.out.is_empty() || self.out.ends_with('\n')
    }

    /// 行首的 `= 标题 =` 判定。命中就说明首段结束。
    fn heading_ahead(&self, rest: &[char]) -> bool {
        let mut offset = 0;
        while matches!(rest.get(offset), Some(' ') | Some('\t') | Some('\n') | Some('\r')) {
            offset += 1;
        }
        let mut level = 0;
        while rest.get(offset) == Some(&'=') {
            level += 1;
            offset += 1;
        }
        if !(2..=6).contains(&level) {
            return false;
        }
        // 找与开头等长的收尾 `=`，且该行 `=` 之后只能是空白。
        // 注意不能要求整段剩余都空白：MediaWiki 的标题行本身就是
        // `== 标题 ==`，后面紧跟下一节内容。
        let mut scan = offset;
        while scan < rest.len() {
            if rest[scan] == '\n' {
                break;
            }
            if rest[scan] == '=' {
                let mut closing = 0;
                while rest.get(scan + closing) == Some(&'=') {
                    closing += 1;
                }
                if closing == level {
                    let line_tail = &rest[scan + closing..];
                    let tail_until_newline = line_tail
                        .iter()
                        .take_while(|value| **value != '\n')
                        .all(|value| value.is_whitespace());
                    if tail_until_newline {
                        return true;
                    }
                }
                scan += closing;
                continue;
            }
            scan += 1;
        }
        false
    }

    /// `<!-- ... -->`，返回消耗的字符数。
    fn consume_comment(&self) -> Option<usize> {
        if !self.restarts_with("<!--") {
            return None;
        }
        let mut cursor = self.index + 4;
        while cursor + 2 < self.input.len() {
            if self.input[cursor] == '-' && self.input[cursor + 1] == '-' && self.input[cursor + 2] == '>' {
                return Some(cursor + 3 - self.index);
            }
            cursor += 1;
        }
        Some(self.input.len() - self.index)
    }

    /// 解析一个 `<tag ...>` 或 `</tag>`；根据标签类型决定丢内容还是留内容。
    /// 返回消耗的字符数。
    fn consume_html_tag(&mut self) -> Option<usize> {
        let value = *self.input.get(self.index)?;
        if value != '<' {
            return None;
        }
        let closing = self.peek(1) == Some('/');
        let name_start = if closing { 2 } else { 1 };
        let mut cursor = self.index + name_start;
        let mut name = String::new();
        while let Some(value) = self.input.get(cursor).copied() {
            if value.is_ascii_alphanumeric() || value == '-' {
                name.push(value.to_ascii_lowercase());
                cursor += 1;
            } else {
                break;
            }
        }
        if name.is_empty() {
            return None;
        }
        // 扫到 '>' 为止，属性里的 '>' 很少见，遇到引号就跳到引号后面。
        let mut in_quote: Option<char> = None;
        let mut self_closing = false;
        while cursor < self.input.len() {
            let value = self.input[cursor];
            match in_quote {
                Some(quote) if value == quote => in_quote = None,
                Some(_) => {}
                None if value == '"' || value == '\'' => in_quote = Some(value),
                None if value == '>' => {
                    // `<ref ... />` 没有闭合标签，不能进入丢弃模式，
                    // 否则后面整篇正文都会被吃掉。
                    self_closing = self.input.get(cursor.wrapping_sub(1)) == Some(&'/');
                    cursor += 1;
                    break;
                }
                None => {}
            }
            cursor += 1;
        }
        let consumed = cursor - self.index;
        if !closing && !self_closing && DROP_WITH_CONTENT.contains(&name.as_str()) {
            self.skipping = Some((name, 1));
        } else if BLOCK_TAGS.contains(&name.as_str()) {
            self.push_break();
        } else if INLINE_TAGS.contains(&name.as_str()) {
            // 保留内容，只丢标签本身。
        } else {
            // 未知标签按保留内容处理，避免误删正文。
        }
        Some(consumed)
    }

    /// 当前处于整段丢弃状态时前进，返回是否已经处理完本轮。
    fn consume_skipping_tag(&mut self, tag: &str, depth: usize) -> bool {
        if let Some(consumed) = self.consume_comment() {
            self.index += consumed;
            return true;
        }
        if self.restarts_with("<") {
            if let Some(consumed) = self.consume_html_tag() {
                let value = self.input[self.index];
                let closing = value == '<' && self.peek(1) == Some('/');
                if !closing && self.skipping.as_ref().map(|(name, _)| name == tag).unwrap_or(false) {
                    let current = self.skipping.as_ref().map(|(_, level)| *level).unwrap_or(1);
                    self.skipping = Some((tag.to_string(), current + 1));
                } else if closing && self.skipping.as_ref().map(|(name, _)| name == tag).unwrap_or(false) {
                    let current = self.skipping.as_ref().map(|(_, level)| *level).unwrap_or(1);
                    if current <= 1 {
                        self.skipping = None;
                    } else {
                        self.skipping = Some((tag.to_string(), current - 1));
                    }
                }
                self.index += consumed;
                return true;
            }
        }
        let value = self.input[self.index];
        if value == '\n' {
            self.push_break();
        }
        self.index += 1;
        let _ = depth;
        true
    }

    /// `{|` ... `|}` 之间整段丢弃。
    fn consume_table(&mut self) {
        if self.restarts_with("|}") {
            self.table_depth = 0;
            self.index += 2;
            return;
        }
        // 表格里的模板与链接也要整段丢掉，所以调用前先记下输出长度，
        // 之后把 consume_* 写进 out 的内容回滚。
        if self.restarts_with("{{") {
            let mark = self.out.len();
            self.consume_template();
            self.out.truncate(mark);
            return;
        }
        if self.restarts_with("[[") {
            let mark = self.out.len();
            self.consume_internal_link();
            self.out.truncate(mark);
            return;
        }
        if self.restarts_with("<!--") {
            if let Some(consumed) = self.consume_comment() {
                self.index += consumed;
                return;
            }
        }
        if self.restarts_with("{|") {
            self.table_depth += 1;
            self.index += 2;
            return;
        }
        let value = self.input[self.index];
        if value == '\n' {
            self.push_break();
        }
        self.index += 1;
    }

    /// 消费一个 `{{ ... }}` / `{{{ ... }}}`，返回消耗的字符数，并按需写入输出。
    fn consume_template(&mut self) -> usize {
        let triple = self.restarts_with("{{{");
        let delim = if triple { DELIM_PARAM } else { DELIM_TEMPLATE };
        let body_start = self.index + delim.len;
        let (body_end, block_end) = find_block_end(self.input, body_start, delim)
            .unwrap_or((self.input.len(), self.input.len()));
        let total = block_end - self.index;
        self.index += total;
        let body: String = self.input[body_start..body_end].iter().collect();
        if triple {
            // 参数占位：没有默认值就丢弃。
            let parts = split_top_level(&body, '|');
            if let Some(default) = parts.get(1) {
                let value = default.trim();
                if !value.is_empty() {
                    self.out.push_str(value);
                }
            }
            return total;
        }
        let parts = split_top_level(&body, '|');
        let name = parts
            .first()
            .map(|value| normalize_template_name(value))
            .unwrap_or_default();
        if name.is_empty() || name.starts_with('#') {
            return total;
        }
        if DROPPED_TEMPLATE_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
        {
            return total;
        }
        if let Some(expanded) = expand_template(&name, &parts) {
            self.out.push_str(&expanded);
        }
        total
    }

    /// 消费一个 `[[ ... ]]`，返回消耗的字符数。
    fn consume_internal_link(&mut self) -> usize {
        let body_start = self.index + DELIM_LINK.len;
        let (body_end, block_end) = find_block_end(self.input, body_start, DELIM_LINK)
            .unwrap_or((self.input.len(), self.input.len()));
        let total = block_end - self.index;
        self.index += total;
        let body: String = self.input[body_start..body_end].iter().collect();
        let parts = split_top_level(&body, '|');
        let target = parts.first().cloned().unwrap_or_default();
        let lowered = target.trim().to_lowercase();
        if DROPPED_LINK_PREFIXES
            .iter()
            .any(|prefix| lowered.starts_with(prefix))
        {
            return total;
        }
        let display = if parts.len() > 1 {
            parts[1..].join(" ")
        } else {
            // 没有显示文字时用目标标题，章节锚点不算标题。
            target.split('#').next().unwrap_or_default().to_string()
        };
        // 语言链接前缀（[[:en:Term]]）不算内容。
        let display = display
            .strip_prefix(':')
            .unwrap_or(&display)
            .split(':')
            .next_back()
            .unwrap_or(&display)
            .trim()
            .to_string();
        if !display.is_empty() {
            self.out.push_str(&display);
        }
        total
    }

    /// 消费一个 `[ ... ]` 外部链接，返回消耗的字符数。
    fn consume_external_link(&mut self) -> usize {
        let body_start = self.index + 1;
        let mut cursor = body_start;
        while cursor < self.input.len() {
            let value = self.input[cursor];
            if value == ']' {
                break;
            }
            if value == '\n' && self.input[cursor + 1..].starts_with(&['[', '[']) {
                // 链接里换行后接内链，不是一个外链。
                self.index += 1;
                return 1;
            }
            cursor += 1;
        }
        let body_end = cursor.min(self.input.len());
        let consumed = if cursor < self.input.len() { body_end + 1 - self.index } else { self.input.len() - self.index };
        self.index += consumed;
        let body: String = self.input[body_start..body_end].iter().collect();
        let (url, label) = match body.split_once(' ') {
            Some((url, label)) => (url.trim(), label.trim()),
            None => (body.trim(), ""),
        };
        if !url.contains("://") && !url.starts_with("//") {
            // 形如 [1] 的引用标记，直接丢弃。
            return consumed;
        }
        if label.is_empty() {
            // 裸链接只保留去掉查询串和锚点的地址。
            let clean = url
                .split(['?', '#'])
                .next()
                .unwrap_or(url)
                .trim_start_matches("//")
                .trim_start_matches("www.");
            self.out.push_str(clean);
        } else {
            self.out.push_str(label);
        }
        consumed
    }
}

/// 一对定界符，字符与长度都显式给出。`{{{`/`}}}` 与 `{{`/`}}` 用同一个 close 字符。
#[derive(Clone, Copy)]
struct Delim {
    open: char,
    close: char,
    len: usize,
}

const DELIM_TEMPLATE: Delim = Delim { open: '{', close: '}', len: 2 };
const DELIM_PARAM: Delim = Delim { open: '{', close: '}', len: 3 };
const DELIM_LINK: Delim = Delim { open: '[', close: ']', len: 2 };

impl Delim {
    fn starts_at(&self, input: &[char], index: usize) -> bool {
        input.len() >= index + self.len
            && input[index] == self.open
            && (0..self.len).all(|offset| input[index + offset] == self.open)
    }

    fn ends_at(&self, input: &[char], index: usize) -> bool {
        input.len() >= index + self.len
            && input[index] == self.close
            && (0..self.len).all(|offset| input[index + offset] == self.close)
    }
}

/// 找到与 `start` 处起始定界符配对的结束位置，返回 (body_end, block_end)。
/// block_end 是收尾定界符之后的位置；找不到配对时返回 None，调用方按“到文件末尾”处理。
/// 嵌套时只认同一种定界符：`{{ }}` 里的 `[[ ]]` 不影响模板深度，反之亦然。
fn find_block_end(input: &[char], start: usize, delim: Delim) -> Option<(usize, usize)> {
    let mut depth = 1usize;
    let mut cursor = start;
    while cursor < input.len() {
        if delim.ends_at(input, cursor) {
            depth -= 1;
            if depth == 0 {
                return Some((cursor, cursor + delim.len));
            }
            cursor += delim.len;
            continue;
        }
        if delim.starts_at(input, cursor) {
            depth += 1;
            cursor += delim.len;
            continue;
        }
        cursor += 1;
    }
    None
}

/// 按顶层分隔符切分，忽略嵌套结构内部的同种分隔符。
fn split_top_level(value: &str, separator: char) -> Vec<String> {
    let chars: Vec<char> = value.chars().collect();
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut index = 0;
    while index < chars.len() {
        let point = chars[index];
        // 跳过嵌套的模板/参数/内链，内部的分隔符不参与顶层切分。
        if point == '{' {
            let delim = if chars[index..].starts_with(&['{', '{', '{']) {
                DELIM_PARAM
            } else if chars[index..].starts_with(&['{', '{']) {
                DELIM_TEMPLATE
            } else {
                DELIM_TEMPLATE
            };
            if let Some((body_end, block_end)) = find_block_end(&chars, index + delim.len, delim) {
                let _ = body_end;
                current.extend(&chars[index..block_end]);
                index = block_end;
                continue;
            }
        }
        if point == '[' && chars[index..].starts_with(&['[', '[']) {
            if let Some((_, block_end)) = find_block_end(&chars, index + DELIM_LINK.len, DELIM_LINK) {
                current.extend(&chars[index..block_end]);
                index = block_end;
                continue;
            }
        }
        if point == separator {
            parts.push(std::mem::take(&mut current));
            index += 1;
            continue;
        }
        current.push(point);
        index += 1;
    }
    parts.push(current);
    parts
}

fn normalize_template_name(value: &str) -> String {
    value
        .trim()
        .trim_start_matches(":")
        .trim()
        .to_lowercase()
        .replace(['_', '-'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// 只展开明确安全的模板；其余返回 None 丢弃。
fn expand_template(name: &str, parts: &[String]) -> Option<String> {
    // 位置参数：顶层不含 '=' 的段。
    let positional: Vec<&str> = parts
        .iter()
        .skip(1)
        .filter(|part| !has_named_param(part))
        .map(String::as_str)
        .collect();
    let first = positional.first().map(|value| value.trim()).unwrap_or_default();
    if name.starts_with("lang") || name == "ill" || name.starts_with("lang ") {
        // {{lang|zh|中文}} / {{lang-zh|中文}}
        return positional
            .get(1)
            .map(|value| value.trim().to_string())
            .or_else(|| (!first.is_empty()).then(|| first.to_string()))
            .filter(|value| !value.is_empty());
    }
    if matches!(name, "nowrap" | "nobr" | "small" | "big" | "sic" | "proper" | "nobreak" | "sc" | "center") {
        return Some(first.to_string());
    }
    if name == "convert" || name == "cvt" {
        let value = positional.get(1).map(|value| value.trim()).unwrap_or_default();
        return Some(if value.is_empty() { first.to_string() } else { format!("{first} {value}") });
    }
    if matches!(name, "nts" | "num" | "数字" | "circa" | "c" | "约" | "ca") {
        return Some(match name {
            "circa" | "c" | "ca" | "约" => format!("约{first}"),
            _ => first.to_string(),
        });
    }
    if matches!(
        name,
        "birth date" | "death date" | "birth date and age" | "start date" | "end date" | "日期"
    ) {
        let numbers: Vec<String> = positional
            .iter()
            .filter(|value| !value.is_empty() && value.chars().all(|v| v.is_ascii_digit()))
            .map(|value| value.to_string())
            .collect();
        if numbers.is_empty() {
            return None;
        }
        let mut out = numbers[0].clone();
        if let Some(month) = numbers.get(1) {
            out.push('年');
            out.push_str(month);
            out.push('月');
        }
        if let Some(day) = numbers.get(2) {
            out.push_str(day);
            out.push('日');
        }
        return Some(out);
    }
    if matches!(name, "mdash" | "em dash") {
        return Some("—".to_string());
    }
    if matches!(name, "ndash" | "spaced ndash" | "en dash") {
        return Some("–".to_string());
    }
    if matches!(name, "ellipsis" | "…" | "ldots") {
        return Some("……".to_string());
    }
    if matches!(name, "degree" | "deg") {
        return Some("°".to_string());
    }
    None
}

fn has_named_param(part: &str) -> bool {
    let chars: Vec<char> = part.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        let value = chars[index];
        if value == '{' {
            let delim = if chars[index..].starts_with(&['{', '{', '{']) { DELIM_PARAM } else { DELIM_TEMPLATE };
            if let Some((_, block_end)) = find_block_end(&chars, index + delim.len, delim) {
                index = block_end;
                continue;
            }
        }
        if value == '[' && chars[index..].starts_with(&['[', '[']) {
            if let Some((_, block_end)) = find_block_end(&chars, index + DELIM_LINK.len, DELIM_LINK) {
                index = block_end;
                continue;
            }
        }
        if value == '=' {
            return true;
        }
        index += 1;
    }
    false
}

/// HTML/维基实体。只处理白名单，未知实体原样输出 '&'。
fn decode_entity(input: &[char]) -> Option<(String, usize)> {
    if input.first() != Some(&'&') {
        return None;
    }
    let mut cursor = 1;
    while cursor < input.len() && input[cursor] != ';' && cursor < 12 {
        cursor += 1;
    }
    if cursor >= input.len() || input[cursor] != ';' {
        return None;
    }
    let name: String = input[1..cursor].iter().collect();
    let replacement = if let Some(digits) = name.strip_prefix('#') {
        let code = if let Some(hex) = digits.strip_prefix('x').or_else(|| digits.strip_prefix('X')) {
            u32::from_str_radix(hex, 16).ok()
        } else {
            digits.parse::<u32>().ok()
        };
        code.and_then(char::from_u32).filter(|value| *value != '\0')
    } else {
        match name.to_lowercase().as_str() {
            "nbsp" => Some(' '),
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "mdash" => Some('—'),
            "ndash" => Some('–'),
            "hellip" => Some('…'),
            "middot" => Some('·'),
            "times" => Some('×'),
            "minus" => Some('−'),
            "deg" => Some('°'),
            "plusmn" => Some('±'),
            "frac12" => Some('½'),
            "frac14" => Some('¼'),
            "frac34" => Some('¾'),
            "laquo" => Some('«'),
            "raquo" => Some('»'),
            "ldquo" => Some('\u{201c}'),
            "rdquo" => Some('\u{201d}'),
            "lsquo" => Some('\u{2018}'),
            "rsquo" => Some('\u{2019}'),
            "copy" => Some('©'),
            "reg" => Some('®'),
            "trade" => Some('™'),
            "prime" => Some('\u{2032}'),
            "shy" => Some('\u{00ad}'),
            "ensp" | "emsp" | "thinsp" => Some(' '),
            _ => None,
        }
    }?;
    Some((replacement.to_string(), cursor + 1))
}

/// 归一化空白：行尾空白去掉，连续空行压成一个，三行以上空行压成两行。
/// 归一化：行内多余空白压成一个空格，段间空行折叠成一个换行。
/// 段落用单换行分隔即可，摘要层不需要保留空行。
fn normalize_output(value: &str) -> String {
    value
        .split('\n')
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// 段选择 + 句读边界截断。
fn select_summary(plain: &str, options: SummaryOptions) -> (String, bool) {
    let paragraphs: Vec<&str> = plain
        .split('\n')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect();
    if paragraphs.is_empty() {
        return (String::new(), false);
    }
    // 至少收第一段；之后每加一段先看够不够 min_chars。
    // min_chars=0 表示不设下限，把 lead 区剩下的段落都收完。
    let mut buffer = String::new();
    for paragraph in paragraphs {
        if !buffer.is_empty() {
            if options.min_chars > 0 && buffer.chars().count() >= options.min_chars {
                break;
            }
            buffer.push('\n');
        }
        buffer.push_str(paragraph);
    }
    if buffer.chars().count() <= options.max_chars {
        return (buffer, false);
    }
    // 超长：在最后一个自然句读边界截断，截不到就硬切。
    let head: String = buffer.chars().take(options.max_chars).collect();
    let cut = head
        .rfind(['。', '！', '？', '；', '!', '?', ';', '\n'])
        .map(|index| index + head[index..].chars().next().map(char::len_utf8).unwrap_or(1))
        .filter(|index| *index > 0)
        .unwrap_or(options.max_chars);
    (head.chars().take(cut).collect(), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary_of(wikitext: &str) -> String {
        extract_summary(wikitext, SummaryOptions { min_chars: 0, max_chars: 100_000 }).summary
    }

    #[test]
    fn debug_table() {
        let w = "概述段落。\n{| class=\"wikitable\"\n|-\n! 名称 !! 说明\n|-\n| 甲 || 乙\n|}\n后续段落。";
        let chars: Vec<char> = w.chars().collect();
        let mut s = Scanner { input: &chars, index: 0, out: String::new(), skipping: None, table_depth: 0 };
        let mut log = String::new();
        while s.index < s.input.len() {
            let at = s.index;
            if s.table_depth > 0 { s.consume_table(); log.push_str(&format!("T{at}->{} ", s.index)); continue; }
            if s.restarts_with("{|") || s.restarts_with("|-") || s.restarts_with("|}") { s.table_depth = 1; s.index += 2; log.push_str(&format!("S{at} ")); continue; }
            let c = s.input[s.index];
            if c == '\n' { s.push_break(); s.index += 1; continue; }
            s.out.push(c); s.index += 1;
        }
        println!("LOG={log}");
        println!("OUTDBG={:?}", s.out);
    }
    #[test]
    fn 纯正文原样保留() {
        assert_eq!(summary_of("人工智能是计算机科学的一个分支。"), "人工智能是计算机科学的一个分支。");
    }

    #[test]
    fn 删除_html_注释() {
        assert_eq!(summary_of("前面<!-- 内部说明 -->后面"), "前面后面");
    }

    #[test]
    fn 删除未闭合注释() {
        assert_eq!(summary_of("正文<!-- 一直注释到文件末尾"), "正文");
    }

    #[test]
    fn 删除_ref_标签含内容() {
        assert_eq!(summary_of("机器学习<sup>[1]</sup>是人工智能分支<sup class=\"ref\">脚注</sup>。"), "机器学习是人工智能分支。");
    }

    #[test]
    fn 删除自闭合_ref() {
        assert_eq!(summary_of("量子计算<ref name=\"a\"/>很有前景。"), "量子计算很有前景。");
    }

    #[test]
    fn 嵌套_ref_整体丢弃() {
        assert_eq!(summary_of("正文<ref>外层<ref>内层</ref>尾巴</ref>结束"), "正文结束");
    }

    #[test]
    fn 内链取显示文字() {
        assert_eq!(summary_of("参见[[机器学习|机器学习技术]]的发展。"), "参见机器学习技术的发展。");
    }

    #[test]
    fn 内链无显示文字取标题() {
        assert_eq!(summary_of("参见[[机器学习]]的发展。"), "参见机器学习的发展。");
    }

    #[test]
    fn 内链去掉章节锚点() {
        assert_eq!(summary_of("参见[[机器学习#历史]]。"), "参见机器学习。");
        assert_eq!(summary_of("参见[[机器学习#历史|发展脉络]]。"), "参见发展脉络。");
    }

    #[test]
    fn 文件链接整段删除() {
        // 文件/图片连同说明文字一起丢，不留在摘要里。
        assert_eq!(summary_of("示例图[[File:example.png|thumb|说明文字]]之后"), "示例图之后");
    }

    #[test]
    fn 中文文件链接整段删除() {
        assert_eq!(summary_of("参见[[文件:示例.png|thumb|说明]]。"), "参见。");
    }

    #[test]
    fn 分类链接整段删除() {
        assert_eq!(summary_of("正文[[Category:人工智能]]结尾"), "正文结尾");
    }

    #[test]
    fn 分类链接删除含参数写法() {
        assert_eq!(summary_of("正文[[分类:人工智能|sort]]结尾"), "正文结尾");
    }

    #[test]
    fn 外链保留可见文字() {
        assert_eq!(summary_of("见[http://example.com 官方网站]。"), "见官方网站。");
    }

    #[test]
    fn 裸外链去掉追踪参数() {
        assert_eq!(summary_of("见[https://example.com/a?utm_source=x&id=3]。"), "见https://example.com/a。");
    }

    #[test]
    fn 形如引用编号的单方括号删除() {
        assert_eq!(summary_of("结论[1]成立。"), "结论成立。");
    }

    #[test]
    fn 删除信息框模板() {
        let wikitext = "{{Infobox person\n| name = 张三\n| birth = 1900\n}}\n张三是一名数学家。";
        assert_eq!(summary_of(wikitext), "张三是一名数学家。");
    }

    #[test]
    fn 删除导航框模板() {
        let wikitext = "{{导航|生物学|分类}}\n细胞是生命的基本单位。";
        assert_eq!(summary_of(wikitext), "细胞是生命的基本单位。");
    }

    #[test]
    fn 删除维护模板() {
        let wikitext = "{{cleanup|date=2026}}\n该条目需要补充引用。";
        assert_eq!(summary_of(wikitext), "该条目需要补充引用。");
    }

    #[test]
    fn 删除消歧义模板() {
        let wikitext = "{{消歧义}}\n苹果属多种植物。";
        assert_eq!(summary_of(wikitext), "苹果属多种植物。");
    }

    #[test]
    fn 嵌套模板整体丢弃() {
        let wikitext = "{{导航|组=a|{{嵌套|x}}}}正文开始。";
        assert_eq!(summary_of(wikitext), "正文开始。");
    }

    #[test]
    fn 参数占位符使用默认值() {
        assert_eq!(summary_of("{{{目标|默认名称}}}是术语。"), "默认名称是术语。");
    }

    #[test]
    fn 无默认值的参数占位符丢弃() {
        assert_eq!(summary_of("前{{未知参数}}后"), "前后");
    }

    #[test]
    fn 展开语言模板() {
        assert_eq!(summary_of("{{lang|zh|人工智能}}"), "人工智能");
        assert_eq!(summary_of("{{lang-zh|人工智能}}"), "人工智能");
    }

    #[test]
    fn 展开日期模板() {
        assert_eq!(summary_of("生于{{birth date|1900|1|2}}。"), "生于1900年1月2日。");
    }

    #[test]
    fn 展开换行禁用模板() {
        assert_eq!(summary_of("{{nowrap|长词组}}测试"), "长词组测试");
    }

    #[test]
    fn 模板命名参数不影响位置参数() {
        assert_eq!(summary_of("{{nowrap|文本|abbr=on}}"), "文本");
    }

    #[test]
    fn 删除整个表格() {
        let wikitext = "概述段落。\n{| class=\"wikitable\"\n|-\n! 名称 !! 说明\n|-\n| 甲 || 乙\n|}\n后续段落。";
        let summary = summary_of(wikitext);
        assert!(!summary.contains("甲"));
        assert!(summary.contains("概述段落。"));
        assert!(summary.contains("后续段落。"));
    }

    #[test]
    fn 表格里的模板也不泄漏() {
        let wikitext = "{| class=\"wikitable\"\n|-\n| {{lang|zh|不该出现}}\n|}\n干净正文。";
        let summary = summary_of(wikitext);
        assert!(!summary.contains("不该出现"));
        assert!(summary.contains("干净正文。"));
    }

    #[test]
    fn 章节标题之后的内容不进入摘要() {
        let wikitext = "首段定义。\n\n== 历史 ==\n历史正文。\n\n== 参考 ==\n文献。";
        let summary = summary_of(wikitext);
        assert_eq!(summary, "首段定义。");
    }

    #[test]
    fn 粗体斜体标记删除() {
        assert_eq!(summary_of("这是'''重要'''的'''概念'''。"), "这是重要的概念。");
        assert_eq!(summary_of("这是''强调''的概念。"), "这是强调的概念。");
    }

    #[test]
    fn 实体解码() {
        assert_eq!(summary_of("A&amp;B 与 &lt;tag&gt; 与 &nbsp;空格"), "A&B 与 <tag> 与 空格");
        assert_eq!(summary_of("&#20013;&#25991;"), "中文");
        assert_eq!(summary_of("&#x4E2D;文"), "中文");
    }

    #[test]
    fn 未知实体保留字面量() {
        assert_eq!(summary_of("&unknown; 保持"), "&unknown; 保持");
    }

    #[test]
    fn 列表行标记被丢弃() {
        let wikitext = "* 第一项\n* 第二项";
        let summary = summary_of(wikitext);
        assert!(!summary.contains('*'));
    }

    #[test]
    fn 空模板段落被跳过() {
        let wikitext = "{{Infobox|x}}\n{{导航|y}}\n真正的首段。";
        assert_eq!(summary_of(wikitext), "真正的首段。");
    }

    #[test]
    fn 摘要截断落在句读边界() {
        let long = "句子内容。".repeat(200);
        let result = extract_summary(&long, SummaryOptions { min_chars: 30, max_chars: 60 });
        assert_eq!(result.quality, SummaryQuality::Truncated);
        assert!(result.summary.chars().count() <= 60);
        assert!(result.summary.ends_with('。'), "应在句号处停下：{}", result.summary);
    }

    #[test]
    fn 摘要超长时按上限硬切() {
        let long = "无标点长文本".repeat(200);
        let result = extract_summary(&long, SummaryOptions { min_chars: 10, max_chars: 50 });
        assert_eq!(result.summary.chars().count(), 50);
    }

    #[test]
    fn 短条目标记为_short() {
        let result = extract_summary("很短的解释。", SummaryOptions { min_chars: 300, max_chars: 600 });
        assert_eq!(result.quality, SummaryQuality::Short);
    }

    #[test]
    fn 纯模板页面被标记为_degraded() {
        // 脚手架占绝大多数、真正的正文只有一句，标记为 degraded 供人工抽查。
        let body = "这是一段真实正文。".repeat(8);
        let wikitext = format!("{{Infobox|a}}\n{}\n{}\n{{Navbox|b}}", "{{嵌套|内容}}".repeat(60), body);
        let result = extract_summary(&wikitext, SummaryOptions { min_chars: 0, max_chars: 600 });
        assert_eq!(result.quality, SummaryQuality::Degraded);
    }

    #[test]
    fn 正文占比高时不算_degraded() {
        let body = "这是一段真实正文。".repeat(80);
        let result = extract_summary(&body, SummaryOptions { min_chars: 0, max_chars: 600 });
        assert_ne!(result.quality, SummaryQuality::Degraded);
    }

    #[test]
    fn 输出不含模板与标签残留() {
        let wikitext = "正文{{Infobox|x}}[[File:a.png]]<ref>r</ref>\n== 章节 ==\n后文'''粗'''";
        let summary = summary_of(wikitext);
        for marker in ["{{", "}}", "[[", "]]", "<ref", "==", "'''"] {
            assert!(!summary.contains(marker), "残留 {marker}：{summary}");
        }
        // 章节标题之后的内容整体不进入摘要
        assert!(!summary.contains("后文"), "章节后的内容不该出现：{summary}");
    }

    #[test]
    fn 软重定向目标可识别() {
        assert_eq!(soft_redirect_target("#REDIRECT [[条目A]]").as_deref(), Some("条目A"));
        assert_eq!(soft_redirect_target("#重定向 [[条目B|显示]]").as_deref(), Some("条目B"));
        assert_eq!(soft_redirect_target("普通正文"), None);
    }

    #[test]
    fn 解析不随输入长度线性占用() {
        // 连续解析 200 次，摘要结果必须稳定，说明没有跨页累积状态。
        let first = extract_summary("人工智能是科学。{{Infobox|x}}", SummaryOptions::default()).summary;
        for _ in 0..200 {
            let again = extract_summary("人工智能是科学。{{Infobox|x}}", SummaryOptions::default()).summary;
            assert_eq!(again, first);
        }
    }

    #[test]
    fn 多个段落按顺序拼接() {
        // min_chars=0 表示不设下限，把 lead 区所有段落都收下。
        let wikitext = "第一段。\n\n第二段。\n\n第三段。";
        let result = extract_summary(wikitext, SummaryOptions { min_chars: 0, max_chars: 600 });
        assert_eq!(result.summary, "第一段。\n第二段。\n第三段。");
    }

    #[test]
    fn 摘要达到下限后停止吞段() {
        // min_chars=10：第一段就够长，不该把后面两段也吞进来。
        let wikitext = "第一段内容足够长了。\n\n第二段不应出现。\n\n第三段也不应出现。";
        let result = extract_summary(wikitext, SummaryOptions { min_chars: 10, max_chars: 600 });
        assert_eq!(result.summary, "第一段内容足够长了。");
    }
}

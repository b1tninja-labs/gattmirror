//! The workspace relays attribute bytes. This test fails when Rust source implements OBD, ELM327,
//! or UDS parsing.
//!
//! Comments are not implementations. A comment that states the prohibition is not a hit, and neither
//! is any other comment. Markdown is not Rust source, so a roadmap sentence that states the
//! prohibition is not a hit either. The simulated adapter's canned identity banner and its `NO DATA`
//! refusal are not parsers: they do not name one, and they are not diagnostic responses.

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug)]
struct Hit {
    line: usize,
    detail: String,
}

fn workspace_root() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        let manifest = dir.join("Cargo.toml");
        if let Ok(text) = fs::read_to_string(&manifest) {
            if text.lines().any(|line| line.trim() == "[workspace]") {
                return dir;
            }
        }
        assert!(
            dir.pop(),
            "no [workspace] manifest above {}",
            env!("CARGO_MANIFEST_DIR")
        );
    }
}

fn scan_workspace(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    visit(root, root, &mut found);
    found.sort();
    found
}

fn visit(root: &Path, dir: &Path, found: &mut Vec<String>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()));
        let path = entry.path();
        let name = entry.file_name();
        if name == "target" || name == ".git" {
            continue;
        }
        let kind = entry
            .file_type()
            .unwrap_or_else(|e| panic!("type of {}: {e}", path.display()));
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            visit(root, &path, found);
            continue;
        }
        let rel = rel_path(root, &path);
        if is_rust(&path) {
            let text = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
            for hit in rust_hits(&text) {
                found.push(format!("{rel}:{}: {}", hit.line, hit.detail));
            }
        } else if is_manifest(&path) {
            let text = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
            for hit in toml_hits(&text) {
                found.push(format!("{rel}:{}: {}", hit.line, hit.detail));
            }
        }
    }
}

fn rel_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn is_rust(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("rs"))
}

fn is_manifest(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("Cargo.toml"))
}

fn toml_hits(src: &str) -> Vec<Hit> {
    let mut hits = Vec::new();
    let mut table = false;
    for (idx, line) in src.lines().enumerate() {
        let stripped = strip_toml_comment(line).trim();
        if stripped.starts_with('[') && stripped.ends_with(']') {
            let header = &stripped[1..stripped.len() - 1];
            table = dep_table(header);
            if let Some(name) = dep_header_name(header) {
                if parser_crate(&name) {
                    hits.push(dependency_hit(idx + 1, &name));
                }
            }
            continue;
        }
        if !table {
            continue;
        }
        if let Some((key, package)) = dep_key(stripped) {
            if parser_crate(&key) {
                hits.push(dependency_hit(idx + 1, &key));
            }
            if let Some(package) = package {
                if parser_crate(&package) {
                    hits.push(dependency_hit(idx + 1, &package));
                }
            }
        }
    }
    hits
}

fn dependency_hit(line: usize, name: &str) -> Hit {
    Hit {
        line,
        detail: format!("dependency `{name}` is an OBD, ELM327 or UDS crate"),
    }
}

fn dep_table(header: &str) -> bool {
    header == "dependencies"
        || header == "dev-dependencies"
        || header == "build-dependencies"
        || header.ends_with(".dependencies")
}

fn dep_header_name(header: &str) -> Option<String> {
    let marker = ".dependencies.";
    let idx = header.rfind(marker)?;
    let name = &header[idx + marker.len()..];
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn dep_key(line: &str) -> Option<(String, Option<String>)> {
    if line.is_empty() || line.starts_with('[') {
        return None;
    }
    let (key, value) = line.split_once('=')?;
    let key = key.trim().trim_matches('"');
    if key.is_empty() {
        return None;
    }
    Some((key.to_string(), package_rename(value)))
}

fn package_rename(value: &str) -> Option<String> {
    let mut rest = value;
    while let Some(idx) = rest.find("package") {
        let before = idx == 0 || {
            let b = rest.as_bytes()[idx - 1];
            !b.is_ascii_alphanumeric() && b != b'_'
        };
        let after = rest[idx + "package".len()..].trim_start();
        if before {
            if let Some(name) = after
                .strip_prefix('=')
                .map(str::trim_start)
                .and_then(quoted)
            {
                return Some(name);
            }
        }
        rest = &rest[idx + "package".len()..];
    }
    None
}

fn quoted(value: &str) -> Option<String> {
    let mut chars = value.chars();
    let quote = chars.next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let mut out = String::new();
    for c in chars {
        if c == quote {
            return Some(out);
        }
        out.push(c);
    }
    None
}

fn strip_toml_comment(line: &str) -> &str {
    let mut quote = None;
    for (idx, c) in line.char_indices() {
        if let Some(open) = quote {
            if c == open {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '#' => return &line[..idx],
            _ => {}
        }
    }
    line
}

fn parser_crate(name: &str) -> bool {
    let parts: Vec<String> = name
        .split(['-', '_', '.'])
        .filter(|part| !part.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    if parts.iter().any(|part| protocol_part(part)) {
        return true;
    }
    parts.windows(2).any(|pair| {
        let joined = format!("{}{}", pair[0], pair[1]);
        matches!(
            joined.as_str(),
            "elm327" | "iso14229" | "iso15765" | "j1979" | "saej1979"
        )
    })
}

fn rust_hits(src: &str) -> Vec<Hit> {
    let mut src = Src {
        text: src,
        i: 0,
        line: 1,
    };
    let mut hits = Vec::new();
    while src.peek().is_some() {
        let at = src.i;
        if src.starts("//") {
            src.skip_line_comment();
        } else if src.starts("/*") {
            src.skip_block_comment();
        } else if src.skip_char_literal() || src.take_literal(&mut hits) {
            // A character or string was consumed.
        } else if src.peek().is_some_and(is_ident_start) {
            let line = src.line;
            let ident = src.read_ident();
            if identifier_hit(&ident) {
                hits.push(Hit {
                    line,
                    detail: format!("identifier `{ident}` names an OBD, ELM327 or UDS parser"),
                });
            }
        } else {
            src.eat();
        }
        if src.i == at {
            src.eat();
        }
    }
    hits
}

struct Src<'a> {
    text: &'a str,
    i: usize,
    line: usize,
}

impl<'a> Src<'a> {
    fn rest(&self) -> &'a str {
        &self.text[self.i..]
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn eat(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.i += c.len_utf8();
        if c == '\n' {
            self.line += 1;
        }
        Some(c)
    }

    fn starts(&self, prefix: &str) -> bool {
        self.rest().starts_with(prefix)
    }

    fn eat_str(&mut self, prefix: &str) {
        for _ in 0..prefix.len() {
            self.eat();
        }
    }

    fn skip_line_comment(&mut self) {
        while let Some(c) = self.eat() {
            if c == '\n' {
                break;
            }
        }
    }

    fn skip_block_comment(&mut self) {
        self.eat_str("/*");
        while self.peek().is_some() && !self.starts("*/") {
            self.eat();
        }
        if self.starts("*/") {
            self.eat_str("*/");
        }
    }

    /// Consumes a `'x'` or `'\n'` literal. A lifetime tick is left for the caller, except for the
    /// opening quote, which is not a parser name.
    fn skip_char_literal(&mut self) -> bool {
        if !self.rest().starts_with('\'') {
            return false;
        }
        if !is_char_literal(self.rest()) {
            self.eat();
            return true;
        }
        self.eat();
        if self.starts("\\") {
            self.eat();
            if self.starts("u{") {
                while let Some(c) = self.eat() {
                    if c == '}' {
                        break;
                    }
                }
            } else {
                self.eat();
            }
        } else {
            self.eat();
        }
        if self.starts("'") {
            self.eat();
        }
        true
    }

    fn take_literal(&mut self, hits: &mut Vec<Hit>) -> bool {
        let line = self.line;
        let Some(body) = self.read_literal_body() else {
            return false;
        };
        if let Some(why) = literal_hit(&body) {
            hits.push(Hit {
                line,
                detail: format!("string literal {why}"),
            });
        }
        true
    }

    fn read_literal_body(&mut self) -> Option<Vec<u8>> {
        if self.rest().starts_with("b'") {
            self.eat_str("b'");
            return Some(self.read_byte_char());
        }
        // Longest prefix first, so `br"` is a byte raw string and not an identifier.
        for prefix in ["br", "cr", "b", "c", "r", ""] {
            if let Some(body) = self.read_prefixed(prefix) {
                return Some(body);
            }
        }
        None
    }

    fn read_prefixed(&mut self, prefix: &str) -> Option<Vec<u8>> {
        let after = self.rest().strip_prefix(prefix)?;
        let raw = prefix.contains('r');
        if after.starts_with('"') {
            self.eat_str(prefix);
            self.eat();
            return Some(if raw {
                self.read_raw(0)
            } else {
                self.read_quoted()
            });
        }
        if !raw {
            return None;
        }
        let hashes = raw_hashes(after)?;
        self.eat_str(prefix);
        self.eat_str(&"#".repeat(hashes));
        self.eat();
        Some(self.read_raw(hashes))
    }

    fn read_byte_char(&mut self) -> Vec<u8> {
        if self.starts("\\") {
            self.eat();
            if self.starts("x") {
                self.eat();
                let hex = self.eat_hex(2);
                self.eat_closer();
                return hex.map_or_else(Vec::new, |b| vec![b]);
            }
            let escaped = self.eat();
            self.eat_closer();
            return simple_escape(escaped).map_or_else(Vec::new, |b| vec![b]);
        }
        let c = self.eat();
        self.eat_closer();
        c.map_or_else(Vec::new, |c| {
            let mut buf = [0; 4];
            c.encode_utf8(&mut buf).as_bytes().to_vec()
        })
    }

    fn read_quoted(&mut self) -> Vec<u8> {
        let mut body = Vec::new();
        while let Some(c) = self.peek() {
            if c == '"' {
                self.eat();
                break;
            }
            if c == '\\' {
                self.eat();
                self.push_escape(&mut body);
                continue;
            }
            push_utf8(&mut body, c);
            self.eat();
        }
        body
    }

    fn read_raw(&mut self, hashes: usize) -> Vec<u8> {
        let closer = format!("\"{}", "#".repeat(hashes));
        let mut body = Vec::new();
        while self.peek().is_some() && !self.starts(&closer) {
            let c = self.peek().unwrap_or('\0');
            push_utf8(&mut body, c);
            self.eat();
        }
        if self.starts(&closer) {
            self.eat_str(&closer);
        }
        body
    }

    fn push_escape(&mut self, body: &mut Vec<u8>) {
        if self.starts("x") {
            self.eat();
            if let Some(byte) = self.eat_hex(2) {
                body.push(byte);
            }
            return;
        }
        if self.starts("u{") {
            self.eat_str("u{");
            let mut hex = String::new();
            while let Some(c) = self.peek() {
                if c == '}' {
                    self.eat();
                    break;
                }
                hex.push(c);
                self.eat();
            }
            if let Some(scalar) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                push_utf8(body, scalar);
            }
            return;
        }
        if self.starts("\n") || self.starts("\r\n") {
            if self.starts("\r\n") {
                self.eat_str("\r\n");
            } else {
                self.eat();
            }
            while self.peek().is_some_and(|c| c == ' ' || c == '\t') {
                self.eat();
            }
            return;
        }
        if let Some(byte) = simple_escape(self.eat()) {
            body.push(byte);
        }
    }

    fn eat_hex(&mut self, n: usize) -> Option<u8> {
        let mut hex = String::new();
        for _ in 0..n {
            let c = self.peek()?;
            if !c.is_ascii_hexdigit() {
                return None;
            }
            hex.push(c);
            self.eat();
        }
        u8::from_str_radix(&hex, 16).ok()
    }

    fn eat_closer(&mut self) {
        if self.starts("'") {
            self.eat();
        }
    }

    fn read_ident(&mut self) -> String {
        let mut ident = String::new();
        while self.peek().is_some_and(is_ident_continue) {
            ident.push(self.eat().unwrap_or('\0'));
        }
        ident
    }
}

fn push_utf8(body: &mut Vec<u8>, c: char) {
    let mut buf = [0; 4];
    body.extend(c.encode_utf8(&mut buf).as_bytes());
}

fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_ascii_alphabetic()
}

fn is_ident_continue(c: char) -> bool {
    c == '_' || c.is_ascii_alphanumeric()
}

fn is_char_literal(rest: &str) -> bool {
    let mut chars = rest.chars();
    if chars.next() != Some('\'') {
        return false;
    }
    match chars.next() {
        Some('\\') => match chars.next() {
            Some('u') => {
                if chars.next() != Some('{') {
                    return false;
                }
                for c in chars {
                    if c == '}' {
                        return true;
                    }
                    if c == '\'' || c == '\n' {
                        return false;
                    }
                }
                false
            }
            Some('\'' | '\n') | None => false,
            Some(_) => chars.next() == Some('\''),
        },
        Some('\'' | '\n') | None => false,
        Some(_) => chars.next() == Some('\''),
    }
}

fn raw_hashes(after_prefix: &str) -> Option<usize> {
    let hashes = after_prefix.chars().take_while(|c| *c == '#').count();
    if hashes == 0 {
        return None;
    }
    let tail = &after_prefix[hashes..];
    tail.starts_with('"').then_some(hashes)
}

fn simple_escape(c: Option<char>) -> Option<u8> {
    match c? {
        'n' => Some(b'\n'),
        'r' => Some(b'\r'),
        't' => Some(b'\t'),
        '0' => Some(0),
        '\\' => Some(b'\\'),
        '\'' => Some(b'\''),
        '"' => Some(b'"'),
        other if other.is_ascii() => Some(other as u8),
        _ => None,
    }
}

/// A literal is a parser when its bytes are adapter-status text or a diagnostic response.
///
/// Text is judged as characters, so the canned banner (`ELM327 v1.5`, which starts with `A`) is
/// not a response. Compact responses are only the OBD positive range `0x41..=0x4f` and the UDS
/// negative `0x7f`: a Nordic UART UUID begins with `6e`, and that byte is a UDS positive response
/// only when it is written as spaced hex. A byte string that is not text is a response when it
/// opens with one of those compact heads and is at least three bytes.
fn literal_hit(bytes: &[u8]) -> Option<&'static str> {
    if is_text(bytes) {
        let text = std::str::from_utf8(bytes).unwrap_or("");
        if has_status(text) {
            return Some("is adapter status text");
        }
        if has_spaced_response(text) {
            return Some("is a spaced diagnostic response");
        }
        if has_compact_response(text) {
            return Some("is a compact diagnostic response");
        }
        return None;
    }
    if bytes.len() >= 3 && bytes.first().is_some_and(|b| compact_head(*b)) {
        return Some("is a binary diagnostic response");
    }
    None
}

fn is_text(bytes: &[u8]) -> bool {
    bytes
        .iter()
        .all(|b| matches!(b, b'\t' | b'\n' | b'\r' | 0x20..=0x7e))
}

fn has_status(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    status_phrases().iter().any(|phrase| lower.contains(phrase))
}

fn status_phrases() -> Vec<String> {
    let groups: [&[&str]; 9] = [
        &["search", "ing"],
        &["unable", " to ", "connect"],
        &["bus", " init"],
        &["can", " error"],
        &["buffer", " full"],
        &["act", " alert"],
        &["lv", " reset"],
        &["lp", " alert"],
        &["data", " error"],
    ];
    groups.into_iter().map(<[&str]>::concat).collect()
}

fn has_spaced_response(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if hex_at(bytes, i)
            && token_edge(bytes, i)
            && spaced_head(bytes, i)
            && spaced_tail(bytes, i + 2)
        {
            return true;
        }
        i += 1;
    }
    false
}

fn hex_at(bytes: &[u8], i: usize) -> bool {
    bytes.get(i).is_some_and(u8::is_ascii_hexdigit)
        && bytes.get(i + 1).is_some_and(u8::is_ascii_hexdigit)
}

fn token_edge(bytes: &[u8], i: usize) -> bool {
    i == 0 || !bytes[i - 1].is_ascii_hexdigit()
}

fn spaced_head(bytes: &[u8], i: usize) -> bool {
    let Ok(head) = u8::from_str_radix(&String::from_utf8_lossy(&bytes[i..i + 2]), 16) else {
        return false;
    };
    if i + 2 < bytes.len() && bytes[i + 2].is_ascii_hexdigit() {
        return false;
    }
    spaced_response_head(head)
}

fn spaced_tail(bytes: &[u8], mut i: usize) -> bool {
    if bytes.get(i) != Some(&b' ') {
        return false;
    }
    i += 1;
    hex_at(bytes, i) && bytes.get(i + 2).is_none_or(|b| !b.is_ascii_hexdigit())
}

fn has_compact_response(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_hexdigit() {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_hexdigit() {
            i += 1;
        }
        let run = &text[start..i];
        if run.len() >= 6 && run.len() % 2 == 0 {
            if let Ok(head) = u8::from_str_radix(&run[..2], 16) {
                if compact_head(head) {
                    return true;
                }
            }
        }
    }
    false
}

fn compact_head(b: u8) -> bool {
    (0x41..=0x4f).contains(&b) || b == 0x7f
}

fn spaced_response_head(b: u8) -> bool {
    compact_head(b)
        || matches!(
            b,
            0x50 | 0x51 | 0x54 | 0x59 | 0x62 | 0x67 | 0x6e | 0x71 | 0x74 | 0x76 | 0x77 | 0x7e
        )
}

/// An identifier implements a parser when a snake or camel part is a protocol name, when a parsing
/// verb sits next to a diagnostic noun (`decode_pid`), or when the parts spell a UDS service.
fn identifier_hit(id: &str) -> bool {
    let parts = ident_parts(id);
    if parts.iter().any(|part| protocol_part(part)) {
        return true;
    }
    let verb = parts.iter().any(|part| verb_part(part));
    let noun = parts.iter().any(|part| noun_part(part));
    if verb && noun {
        return true;
    }
    service_parts(&parts)
}

fn ident_parts(id: &str) -> Vec<String> {
    let chars: Vec<char> = id.chars().collect();
    let mut snake = String::new();
    for (i, c) in chars.iter().enumerate() {
        if *c == '_' {
            snake.push('_');
            continue;
        }
        if c.is_uppercase() && i > 0 && {
            let prev = chars[i - 1];
            let next_lower = chars.get(i + 1).is_some_and(|c| c.is_lowercase());
            prev.is_lowercase() || prev == '_' || (prev.is_uppercase() && next_lower)
        } {
            snake.push('_');
        }
        snake.push(c.to_ascii_lowercase());
    }
    snake
        .split('_')
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

fn protocol_part(part: &str) -> bool {
    matches!(
        part,
        "obd" | "obd2" | "obdii" | "elm" | "elm327" | "uds" | "j1979" | "iso14229" | "iso15765"
    )
}

fn verb_part(part: &str) -> bool {
    matches!(
        part,
        "parse"
            | "parsed"
            | "parser"
            | "parsing"
            | "decode"
            | "decoded"
            | "decoder"
            | "decoding"
            | "interpret"
            | "interpreted"
            | "interpreter"
            | "classify"
            | "classified"
            | "classifier"
            | "dissect"
            | "dissected"
            | "implement"
            | "implemented"
    )
}

fn noun_part(part: &str) -> bool {
    matches!(part, "pid" | "dtc" | "vin" | "nrc")
}

fn service_parts(parts: &[String]) -> bool {
    const SERVICES: &[&[&str]] = &[
        &["read", "data", "by", "identifier"],
        &["write", "data", "by", "identifier"],
        &["diagnostic", "session", "control"],
        &["tester", "present"],
        &["read", "dtc", "information"],
        &["clear", "diagnostic", "information"],
    ];
    SERVICES.iter().any(|service| {
        parts.windows(service.len()).any(|window| {
            window
                .iter()
                .zip(service.iter())
                .all(|(got, want)| got.as_str() == *want)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(src: &str) {
        let hits = rust_hits(src);
        assert!(hits.is_empty(), "{hits:?}");
    }

    #[test]
    fn a_comment_that_states_the_prohibition_is_not_a_hit() {
        clean("// It must not parse OBD, ELM327 or UDS.\nfn forward(b: &[u8]) -> &[u8] { b }\n");
        clean("//! The crate parses no application protocol.\nfn forward() {}\n");
        clean("/* no OBD, ELM327 or UDS parsing */\nfn forward() {}\n");
    }

    #[test]
    fn canned_banner_and_refusal_are_not_hits() {
        clean("const IDENT: &str = \"ELM327 v1.5\";\nconst NO: &str = \"NO DATA\";\n");
        clean("const X: &[u8] = b\"ATI\\rELM327 v1.5\\r\\n\\r\\n>\";\n");
        clean("fn parse_address() {}\n");
        clean("const U: &str = \"6E400001-B5A3-F393-E0A9-E50E24DCCA9E\";\n");
    }

    #[test]
    fn a_function_that_names_a_parser_is_a_hit() {
        let name = ["parse", "obd", "mode"].join("_");
        let src = format!("fn {name}() {{}}\n");
        assert!(!rust_hits(&src).is_empty(), "{src}");
        let pid = ["decode", "pid"].join("_");
        let src = format!("fn {pid}() {{}}\n");
        assert!(!rust_hits(&src).is_empty(), "{src}");
    }

    #[test]
    fn a_diagnostic_response_literal_is_a_hit() {
        let spaced = ["41", "0C", "1A", "F8"].join(" ");
        let src = format!("const R: &str = \"{spaced}\";\n");
        assert!(!rust_hits(&src).is_empty(), "{src}");
        let compact = ["41", "0C", "1A", "F8"].concat();
        let src = format!("const R: &str = \"{compact}\";\n");
        assert!(!rust_hits(&src).is_empty(), "{src}");
        let bytes = ["41", "0C", "1A", "F8"].join("\\x");
        let src = format!("const R: &[u8] = b\"\\x{bytes}\";\n");
        assert!(!rust_hits(&src).is_empty(), "{src}");
    }

    #[test]
    fn a_protocol_dependency_is_a_hit_and_a_comment_is_not() {
        let name = ["obd", "bridge"].join("-");
        let src = format!("[dependencies]\n{name} = {{ path = \"../x\" }}\n");
        assert!(!toml_hits(&src).is_empty(), "{src}");
        assert!(
            toml_hits("# as in obd-bridge\n[dependencies]\nserial-stream = \"0.1\"\n").is_empty()
        );
        assert!(
            toml_hits("[target.'cfg(windows)'.dependencies.windows]\nversion = \"0.58\"\n")
                .is_empty()
        );
        assert!(toml_hits("[package]\nname = \"gattmirror\"\n").is_empty());
    }
}

#[test]
fn workspace_rust_relays_attribute_bytes_only() {
    let found = scan_workspace(&workspace_root());
    assert!(
        found.is_empty(),
        "Rust in this workspace relays attribute bytes and does not implement OBD, ELM327 or UDS parsing:\n{}",
        found.join("\n")
    );
}

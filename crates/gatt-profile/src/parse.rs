use std::fmt;

use crate::model::{Characteristic, Descriptor, Profile, Properties, Service, ServiceKind};
use crate::uuid::{ParseUuidError, Uuid};

/// What was wrong with a line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseErrorKind {
    /// The first word is not a directive.
    UnknownDirective(String),
    /// A required argument is missing.
    MissingArgument(&'static str),
    /// Words left over after the arguments.
    TrailingText(String),
    /// A UUID did not parse.
    BadUuid(ParseUuidError),
    /// A property name or number is not recognised.
    BadProperty(String),
    /// A `value=` is not an even number of hex digits.
    BadHex(String),
    /// An option other than `value=`, or `value=` given twice.
    BadOption(String),
    /// `primary` or `secondary` expected.
    BadServiceKind(String),
    /// A `characteristic` with no service before it.
    NoService,
    /// A `descriptor` with no characteristic before it.
    NoCharacteristic,
    /// A single-use directive given twice.
    Duplicate(&'static str),
    /// A directive that takes text was given none.
    EmptyText(&'static str),
}

/// A parse failure at a 1-based line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    /// 1-based line number.
    pub line: usize,
    /// The reason.
    pub kind: ParseErrorKind,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: ", self.line)?;
        match &self.kind {
            ParseErrorKind::UnknownDirective(w) => write!(f, "unknown directive `{w}`"),
            ParseErrorKind::MissingArgument(a) => write!(f, "missing {a}"),
            ParseErrorKind::TrailingText(t) => write!(f, "unexpected `{t}`"),
            ParseErrorKind::BadUuid(e) => write!(f, "bad UUID: {e}"),
            ParseErrorKind::BadProperty(p) => write!(f, "unknown property `{p}`"),
            ParseErrorKind::BadHex(h) => write!(f, "`{h}` is not an even run of hex digits"),
            ParseErrorKind::BadOption(o) => write!(f, "bad option `{o}`"),
            ParseErrorKind::BadServiceKind(k) => {
                write!(f, "expected `primary` or `secondary`, found `{k}`")
            }
            ParseErrorKind::NoService => f.write_str("characteristic before any service"),
            ParseErrorKind::NoCharacteristic => f.write_str("descriptor before any characteristic"),
            ParseErrorKind::Duplicate(d) => write!(f, "`{d}` given twice"),
            ParseErrorKind::EmptyText(d) => write!(f, "`{d}` needs a name"),
        }
    }
}

impl std::error::Error for ParseError {}

fn parse_hex(s: &str) -> Result<Vec<u8>, ParseErrorKind> {
    let bad = || ParseErrorKind::BadHex(s.to_owned());
    if s.len() % 2 != 0 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(bad());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| bad()))
        .collect()
}

fn parse_props(s: &str) -> Result<Properties, ParseErrorKind> {
    let bad = || ParseErrorKind::BadProperty(s.to_owned());
    if s == "none" {
        return Ok(Properties(0));
    }
    if let Some(h) = s.strip_prefix("0x") {
        return u8::from_str_radix(h, 16).map(Properties).map_err(|_| bad());
    }
    let mut bits = 0;
    for name in s.split(',') {
        let (_, bit) = Properties::NAMES
            .iter()
            .find(|(n, _)| *n == name)
            .ok_or_else(|| ParseErrorKind::BadProperty(name.to_owned()))?;
        bits |= bit;
    }
    Ok(Properties(bits))
}

fn parse_uuid(word: Option<&str>) -> Result<Uuid, ParseErrorKind> {
    word.ok_or(ParseErrorKind::MissingArgument("UUID"))?
        .parse()
        .map_err(ParseErrorKind::BadUuid)
}

/// Zero or one `value=hex` option, then nothing else.
fn parse_value<'a>(
    words: impl Iterator<Item = &'a str>,
) -> Result<Option<Vec<u8>>, ParseErrorKind> {
    let mut value = None;
    for w in words {
        match w.strip_prefix("value=") {
            Some(h) if value.is_none() => value = Some(parse_hex(h)?),
            _ => return Err(ParseErrorKind::BadOption(w.to_owned())),
        }
    }
    Ok(value)
}

fn no_more<'a>(mut words: impl Iterator<Item = &'a str>) -> Result<(), ParseErrorKind> {
    words
        .next()
        .map_or(Ok(()), |w| Err(ParseErrorKind::TrailingText(w.to_owned())))
}

fn text(rest: &str, name: &'static str) -> Result<String, ParseErrorKind> {
    if rest.is_empty() {
        Err(ParseErrorKind::EmptyText(name))
    } else {
        Ok(rest.to_owned())
    }
}

fn line(profile: &mut Profile, raw: &str) -> Result<(), ParseErrorKind> {
    let trimmed = raw.trim();
    let (word, rest) = trimmed
        .split_once(char::is_whitespace)
        .map_or((trimmed, ""), |(w, r)| (w, r.trim()));
    let mut words = rest.split_whitespace();
    match word {
        "device-name" | "adv-name" => {
            let (slot, name) = if word == "device-name" {
                (&mut profile.device_name, "device-name")
            } else {
                (&mut profile.advertised_name, "adv-name")
            };
            if slot.is_some() {
                return Err(ParseErrorKind::Duplicate(name));
            }
            *slot = Some(text(rest, name)?);
        }
        "adv-service" => {
            let u = parse_uuid(words.next())?;
            no_more(words)?;
            profile.advertised_services.push(u);
        }
        "service" => {
            let uuid = parse_uuid(words.next())?;
            let kind = match words.next() {
                None | Some("primary") => ServiceKind::Primary,
                Some("secondary") => ServiceKind::Secondary,
                Some(k) => return Err(ParseErrorKind::BadServiceKind(k.to_owned())),
            };
            no_more(words)?;
            profile.services.push(Service {
                uuid,
                kind,
                characteristics: Vec::new(),
            });
        }
        "characteristic" => {
            let uuid = parse_uuid(words.next())?;
            let properties = parse_props(
                words
                    .next()
                    .ok_or(ParseErrorKind::MissingArgument("properties"))?,
            )?;
            let value = parse_value(words)?;
            profile
                .services
                .last_mut()
                .ok_or(ParseErrorKind::NoService)?
                .characteristics
                .push(Characteristic {
                    uuid,
                    properties,
                    value,
                    descriptors: Vec::new(),
                });
        }
        "descriptor" => {
            let uuid = parse_uuid(words.next())?;
            let value = parse_value(words)?.unwrap_or_default();
            profile
                .services
                .last_mut()
                .and_then(|s| s.characteristics.last_mut())
                .ok_or(ParseErrorKind::NoCharacteristic)?
                .descriptors
                .push(Descriptor { uuid, value });
        }
        other => return Err(ParseErrorKind::UnknownDirective(other.to_owned())),
    }
    Ok(())
}

/// Parse the text format.
///
/// One directive per line; blank lines and lines starting with `#` are ignored; indentation is
/// free. Directives: `device-name TEXT`, `adv-name TEXT`, `adv-service UUID`,
/// `service UUID [primary|secondary]`, `characteristic UUID PROPS [value=HEX]` and
/// `descriptor UUID [value=HEX]`. `PROPS` is `none`, `0xNN`, or a comma list of
/// `broadcast read write-no-response write notify indicate signed-write extended`. A UUID is four
/// hex digits (the Bluetooth base UUID is implied) or the dashed 36-character form.
///
/// # Errors
/// The first malformed line, with its 1-based number.
pub fn parse(input: &str) -> Result<Profile, ParseError> {
    let mut profile = Profile::default();
    for (i, raw) in input.lines().enumerate() {
        let t = raw.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        line(&mut profile, raw).map_err(|kind| ParseError { line: i + 1, kind })?;
    }
    Ok(profile)
}

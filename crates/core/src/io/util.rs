//! Shared helpers for the file-format modules: image sniffing, base64, code pages, colours, units.

use crate::model::*;
use std::sync::Arc;

/// Detect an image format and pixel size from its header.
/// Returns (format, size) where format is "png", "jpeg", "gif", "bmp", "emf", "wmf", "tiff", "webp" or "svg".
pub fn image_info(b: &[u8]) -> (String, Option<(u32, u32)>) {
    if b.len() >= 24 && b.starts_with(b"\x89PNG\r\n\x1a\n") {
        let w = u32::from_be_bytes([b[16], b[17], b[18], b[19]]);
        let h = u32::from_be_bytes([b[20], b[21], b[22], b[23]]);
        return ("png".into(), Some((w, h)));
    }
    if b.len() >= 4 && b[0] == 0xFF && b[1] == 0xD8 {
        return ("jpeg".into(), jpeg_size(b));
    }
    if b.len() >= 10 && (b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a")) {
        let w = u16::from_le_bytes([b[6], b[7]]) as u32;
        let h = u16::from_le_bytes([b[8], b[9]]) as u32;
        return ("gif".into(), Some((w, h)));
    }
    if b.len() >= 26 && b.starts_with(b"BM") {
        let w = i32::from_le_bytes([b[18], b[19], b[20], b[21]]).unsigned_abs();
        let h = i32::from_le_bytes([b[22], b[23], b[24], b[25]]).unsigned_abs();
        return ("bmp".into(), Some((w, h)));
    }
    if b.len() >= 44 && b[0..4] == [1, 0, 0, 0] && &b[40..44] == b" EMF" {
        return ("emf".into(), None);
    }
    if b.len() >= 4 && (b[0..4] == [0xD7, 0xCD, 0xC6, 0x9A] || b[0..4] == [1, 0, 9, 0]) {
        return ("wmf".into(), None);
    }
    if b.len() >= 4 && (b.starts_with(b"II*\0") || b.starts_with(b"MM\0*")) {
        return ("tiff".into(), None);
    }
    if b.len() >= 12 && b.starts_with(b"RIFF") && &b[8..12] == b"WEBP" {
        return ("webp".into(), None);
    }
    let head = String::from_utf8_lossy(&b[..b.len().min(256)]).to_ascii_lowercase();
    if head.contains("<svg") || (head.starts_with("<?xml") && head.contains("svg")) {
        return ("svg".into(), None);
    }
    ("png".into(), None)
}

fn jpeg_size(b: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2;
    while i + 9 < b.len() {
        if b[i] != 0xFF {
            i += 1;
            continue;
        }
        let m = b[i + 1];
        if m == 0xFF {
            i += 1;
            continue;
        }
        if m == 0xD8 || m == 0x01 || (0xD0..=0xD7).contains(&m) {
            i += 2;
            continue;
        }
        let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        if matches!(m, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF) {
            let h = u16::from_be_bytes([b[i + 5], b[i + 6]]) as u32;
            let w = u16::from_be_bytes([b[i + 7], b[i + 8]]) as u32;
            return Some((w, h));
        }
        i += 2 + len;
    }
    None
}

/// MIME type for an image format string.
pub fn image_mime(format: &str) -> &'static str {
    match format {
        "png" => "image/png",
        "jpeg" | "jpg" => "image/jpeg",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "emf" => "image/x-emf",
        "wmf" => "image/x-wmf",
        "tiff" | "tif" => "image/tiff",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

/// File extension used when storing an image.
pub fn image_ext(format: &str) -> &str {
    match format {
        "jpeg" | "jpg" => "jpeg",
        "" => "png",
        f => f,
    }
}

/// Build an `ImageData` from raw bytes, sniffing format and size.
pub fn image_data(bytes: Vec<u8>) -> Arc<ImageData> {
    let (format, pixel_size) = image_info(&bytes);
    Arc::new(ImageData { bytes, format, pixel_size })
}

/// Natural display size (points) for an image at 96 dpi, capped to `max_w`.
pub fn natural_size(data: &ImageData, max_w: f32) -> (f32, f32) {
    let (w, h) = data.pixel_size.map(|(w, h)| (w as f32 * 0.75, h as f32 * 0.75)).unwrap_or((144.0, 108.0));
    let (w, h) = (w.max(1.0), h.max(1.0));
    if w > max_w {
        (max_w, h * max_w / w)
    } else {
        (w, h)
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    out
}

pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            c if c.is_ascii_whitespace() => continue,
            _ => return None,
        };
        buf = buf << 6 | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
}

/// Windows-1252 code points for bytes 0x80..=0x9F.
const CP1252_HIGH: [u16; 32] = [
    0x20AC, 0x0081, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0x008D,
    0x017D, 0x008F, 0x0090, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A,
    0x0153, 0x009D, 0x017E, 0x0178,
];

pub fn cp1252_char(b: u8) -> char {
    if (0x80..0xA0).contains(&b) {
        char::from_u32(CP1252_HIGH[(b - 0x80) as usize] as u32).unwrap_or('?')
    } else {
        b as char
    }
}

pub fn cp1252_decode(b: &[u8]) -> String {
    b.iter().map(|&c| cp1252_char(c)).collect()
}

/// Encode a char to a cp1252 byte if representable.
pub fn cp1252_encode(c: char) -> Option<u8> {
    let u = c as u32;
    if u < 0x80 || (0xA0..=0xFF).contains(&u) {
        return Some(u as u8);
    }
    CP1252_HIGH.iter().position(|&x| x as u32 == u).map(|i| 0x80 + i as u8)
}

/// Word's named highlight colours.
pub const HIGHLIGHTS: &[(&str, Color)] = &[
    ("yellow", Color::rgb(255, 255, 0)),
    ("green", Color::rgb(0, 255, 0)),
    ("cyan", Color::rgb(0, 255, 255)),
    ("magenta", Color::rgb(255, 0, 255)),
    ("blue", Color::rgb(0, 0, 255)),
    ("red", Color::rgb(255, 0, 0)),
    ("darkBlue", Color::rgb(0, 0, 128)),
    ("darkCyan", Color::rgb(0, 128, 128)),
    ("darkGreen", Color::rgb(0, 128, 0)),
    ("darkMagenta", Color::rgb(128, 0, 128)),
    ("darkRed", Color::rgb(128, 0, 0)),
    ("darkYellow", Color::rgb(128, 128, 0)),
    ("darkGray", Color::rgb(128, 128, 128)),
    ("lightGray", Color::rgb(192, 192, 192)),
    ("black", Color::rgb(0, 0, 0)),
    ("white", Color::rgb(255, 255, 255)),
];

pub fn highlight_color(name: &str) -> Option<Color> {
    HIGHLIGHTS.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, c)| *c)
}

/// Nearest named highlight colour.
pub fn highlight_name(c: Color) -> &'static str {
    let d = |a: Color, b: Color| {
        let dr = a.r as i32 - b.r as i32;
        let dg = a.g as i32 - b.g as i32;
        let db = a.b as i32 - b.b as i32;
        dr * dr + dg * dg + db * db
    };
    HIGHLIGHTS.iter().min_by_key(|(_, h)| d(*h, c)).map(|(n, _)| *n).unwrap_or("yellow")
}

/// CSS / common colour names.
pub fn named_color(name: &str) -> Option<Color> {
    let n = name.trim().to_ascii_lowercase();
    let c = match n.as_str() {
        "black" => Color::rgb(0, 0, 0),
        "white" => Color::rgb(255, 255, 255),
        "red" => Color::rgb(255, 0, 0),
        "green" => Color::rgb(0, 128, 0),
        "lime" => Color::rgb(0, 255, 0),
        "blue" => Color::rgb(0, 0, 255),
        "yellow" => Color::rgb(255, 255, 0),
        "cyan" | "aqua" => Color::rgb(0, 255, 255),
        "magenta" | "fuchsia" => Color::rgb(255, 0, 255),
        "gray" | "grey" => Color::rgb(128, 128, 128),
        "silver" => Color::rgb(192, 192, 192),
        "maroon" => Color::rgb(128, 0, 0),
        "olive" => Color::rgb(128, 128, 0),
        "navy" => Color::rgb(0, 0, 128),
        "purple" => Color::rgb(128, 0, 128),
        "teal" => Color::rgb(0, 128, 128),
        "orange" => Color::rgb(255, 165, 0),
        "darkblue" => Color::rgb(0, 0, 139),
        "darkred" => Color::rgb(139, 0, 0),
        "darkgreen" => Color::rgb(0, 100, 0),
        "lightgray" | "lightgrey" => Color::rgb(211, 211, 211),
        "darkgray" | "darkgrey" => Color::rgb(169, 169, 169),
        "pink" => Color::rgb(255, 192, 203),
        "brown" => Color::rgb(165, 42, 42),
        _ => return None,
    };
    Some(c)
}

/// Parse a CSS colour: `#rgb`, `#rrggbb`, `rgb(r,g,b)`, or a name.
pub fn css_color(s: &str) -> Option<Color> {
    let s = s.trim();
    if let Some(h) = s.strip_prefix('#') {
        if h.len() == 3 {
            let v: Vec<u8> = h.chars().filter_map(|c| c.to_digit(16)).map(|d| (d * 17) as u8).collect();
            if v.len() == 3 {
                return Some(Color::rgb(v[0], v[1], v[2]));
            }
        }
        return Color::from_hex(h);
    }
    let lower = s.to_ascii_lowercase();
    if lower.starts_with("rgb") {
        let inner = lower.split_once('(')?.1.trim_end_matches(')');
        let v: Vec<u8> = inner
            .split([',', ' ', '/'])
            .filter(|x| !x.trim().is_empty())
            .take(3)
            .filter_map(|x| {
                let x = x.trim();
                if let Some(p) = x.strip_suffix('%') {
                    p.parse::<f32>().ok().map(|f| (f * 2.55).round().clamp(0.0, 255.0) as u8)
                } else {
                    x.parse::<f32>().ok().map(|f| f.round().clamp(0.0, 255.0) as u8)
                }
            })
            .collect();
        if v.len() == 3 {
            return Some(Color::rgb(v[0], v[1], v[2]));
        }
        return None;
    }
    if lower == "transparent" || lower == "auto" || lower == "none" || lower == "windowtext" {
        return None;
    }
    named_color(&lower)
}

/// Parse a length with a unit (pt, px, in, cm, mm, pc, em) into points. Bare numbers are points.
pub fn parse_length(s: &str) -> Option<f32> {
    let s = s.trim();
    let idx = s.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+')).unwrap_or(s.len());
    let (num, unit) = s.split_at(idx);
    let v: f32 = num.parse().ok()?;
    let f = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "pt" => 1.0,
        "px" => 0.75,
        "in" => 72.0,
        "cm" => 72.0 / 2.54,
        "mm" => 72.0 / 25.4,
        "pc" => 12.0,
        "em" | "rem" => 12.0,
        "twip" => 0.05,
        _ => return None,
    };
    Some(v * f)
}

/// Format a float compactly (no trailing zeros).
pub fn fmt_num(v: f32) -> String {
    let r = (v * 1000.0).round() / 1000.0;
    if r == r.trunc() {
        format!("{}", r as i64)
    } else {
        let s = format!("{:.3}", r);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// Points → twips (rounded).
pub fn twips(pt: f32) -> i64 {
    (pt * 20.0).round() as i64
}

/// Points → EMU.
pub fn emu(pt: f32) -> i64 {
    (pt * 12700.0).round() as i64
}

/// Map common Symbol / Wingdings private-use bullet characters to Unicode.
pub fn map_symbol_bullet(text: &str, font: Option<&str>) -> Option<String> {
    let c = text.chars().next()?;
    let f = font.unwrap_or("").to_ascii_lowercase();
    let mapped = match (c as u32, f.as_str()) {
        (0xF0B7, _) | (0x00B7, "symbol") => '•',
        (0xF0A7, _) => '▪',
        (0xF0D8, _) => '➢',
        (0xF0FC, _) => '✓',
        (0xF076, _) => '❖',
        (0xF06E, _) => '■',
        (0xF0A8, _) => '◆',
        (0xF06F, "wingdings") | (0x006F, "wingdings") => '□',
        (0xF0B2, _) => '◇',
        (0xF02D, _) | (0xF0BE, _) => '–',
        _ => return None,
    };
    Some(mapped.to_string())
}

/// Current UTC time as (year, month, day, hour, minute, second).
pub fn now_utc() -> (i64, u32, u32, u32, u32, u32) {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    // Civil-from-days (Howard Hinnant).
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d, (rem / 3600) as u32, (rem % 3600 / 60) as u32, (rem % 60) as u32)
}

/// Current UTC time in W3CDTF / ISO 8601, e.g. `2026-10-19T16:00:00Z`.
pub fn now_iso8601() -> String {
    let (y, mo, d, h, mi, s) = now_utc();
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, mo, d, h, mi, s)
}

const MONTHS: [&str; 12] =
    ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

/// Format a date with a Word date picture (`d`, `dd`, `ddd`, `dddd`, `M`…`MMMM`, `yy`, `yyyy`, `h`, `H`, `mm`, `ss`, `am/pm`).
pub fn format_date(pic: &str, t: (i64, u32, u32, u32, u32, u32)) -> String {
    let (y, mo, d, h, mi, s) = t;
    // Day of week (Sakamoto).
    let dow = {
        let tt = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
        let yy = if mo < 3 { y - 1 } else { y };
        ((yy + yy / 4 - yy / 100 + yy / 400 + tt[(mo as usize - 1) % 12] + d as i64) % 7) as usize
    };
    let ampm = pic.to_ascii_lowercase().contains("am/pm");
    let chars: Vec<char> = pic.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' {
            i += 1;
            while i < chars.len() && chars[i] != '\'' {
                out.push(chars[i]);
                i += 1;
            }
            i += 1;
            continue;
        }
        if pic[pic.char_indices().nth(i).map(|x| x.0).unwrap_or(0)..].to_ascii_lowercase().starts_with("am/pm") {
            out.push_str(if h < 12 { "AM" } else { "PM" });
            i += 5;
            continue;
        }
        let mut n = 1;
        while i + n < chars.len() && chars[i + n] == c {
            n += 1;
        }
        let piece = match c {
            'd' => match n {
                1 => d.to_string(),
                2 => format!("{:02}", d),
                3 => DAYS[dow][..3].to_string(),
                _ => DAYS[dow].to_string(),
            },
            'M' => match n {
                1 => mo.to_string(),
                2 => format!("{:02}", mo),
                3 => MONTHS[(mo as usize + 11) % 12][..3].to_string(),
                _ => MONTHS[(mo as usize + 11) % 12].to_string(),
            },
            'y' => {
                if n <= 2 {
                    format!("{:02}", y % 100)
                } else {
                    y.to_string()
                }
            }
            'h' | 'H' => {
                let hh = if c == 'h' || ampm { (h + 11) % 12 + 1 } else { h };
                if n >= 2 {
                    format!("{:02}", hh)
                } else {
                    hh.to_string()
                }
            }
            'm' => format!("{:02}", mi),
            's' => format!("{:02}", s),
            _ => chars[i..i + n].iter().collect(),
        };
        out.push_str(&piece);
        i += n;
    }
    out
}

/// Format the current date with a Word date picture.
pub fn format_date_now(pic: &str) -> String {
    format_date(pic, now_utc())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b64_roundtrip() {
        for s in [&b""[..], b"a", b"ab", b"abc", b"abcd", b"\x00\xff\x10hello world"] {
            assert_eq!(base64_decode(&base64_encode(s)).unwrap(), s);
        }
        assert_eq!(base64_encode(b"Man"), "TWFu");
    }

    #[test]
    fn colors_and_lengths() {
        assert_eq!(css_color("#f00"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(css_color("rgb(1, 2, 3)"), Some(Color::rgb(1, 2, 3)));
        assert_eq!(parse_length("1in"), Some(72.0));
        assert_eq!(parse_length("16px"), Some(12.0));
        assert_eq!(highlight_name(Color::rgb(250, 250, 10)), "yellow");
        assert_eq!(fmt_num(1.5), "1.5");
        assert_eq!(fmt_num(2.0), "2");
        assert_eq!(format_date("d MMMM yyyy", (2026, 10, 19, 0, 0, 0)), "19 October 2026");
        assert_eq!(format_date("dddd, M/d/yy h:mm am/pm", (2026, 10, 19, 15, 5, 0)), "Monday, 10/19/26 3:05 PM");
    }
}

//! Small helpers: dates.

use std::time::{SystemTime, UNIX_EPOCH};

/// Days since 1970-01-01 → (year, month, day).
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Current UTC time as (year, month, day, hour, minute, second).
pub fn now_parts() -> (i64, u32, u32, u32, u32, u32) {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let s = secs.rem_euclid(86_400);
    (y, m, d, (s / 3600) as u32, ((s / 60) % 60) as u32, (s % 60) as u32)
}

/// Current UTC time in ISO 8601, e.g. `2026-10-19T16:00:00Z`.
pub fn now_iso() -> String {
    let (y, mo, d, h, mi, s) = now_parts();
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, mo, d, h, mi, s)
}

const MONTHS: [&str; 12] =
    ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

/// Format a date with a Word date picture (`d`, `dd`, `ddd`, `dddd`, `M`, `MM`, `MMM`, `MMMM`, `yy`, `yyyy`,
/// `H`, `HH`, `h`, `hh`, `mm`, `ss`, `am/pm`).
pub fn format_date(picture: &str, parts: (i64, u32, u32, u32, u32, u32)) -> String {
    let (y, mo, d, h, mi, s) = parts;
    let days = {
        // weekday from civil date (Zeller-ish via days since epoch)
        let mut lo = -1_000_000i64;
        let mut hi = 1_000_000i64;
        while lo < hi {
            let mid = (lo + hi) / 2;
            if civil_from_days(mid) < (y, mo, d) {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
    };
    let wd = ((days.rem_euclid(7)) + 4) % 7; // 1970-01-01 was a Thursday
    let pic = if picture.trim().is_empty() { "d MMMM yyyy" } else { picture };
    let c: Vec<char> = pic.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        let mut n = 1;
        while i + n < c.len() && c[i + n] == ch {
            n += 1;
        }
        let lower: String = c[i..].iter().take(5).collect::<String>().to_lowercase();
        if lower.starts_with("am/pm") {
            out.push_str(if h < 12 { "AM" } else { "PM" });
            i += 5;
            continue;
        }
        match ch {
            '\'' => {
                i += 1;
                while i < c.len() && c[i] != '\'' {
                    out.push(c[i]);
                    i += 1;
                }
                i += 1;
                continue;
            }
            'd' => out.push_str(&match n {
                1 => d.to_string(),
                2 => format!("{:02}", d),
                3 => DAYS[wd as usize][..3].to_string(),
                _ => DAYS[wd as usize].to_string(),
            }),
            'M' => out.push_str(&match n {
                1 => mo.to_string(),
                2 => format!("{:02}", mo),
                3 => MONTHS[(mo - 1) as usize][..3].to_string(),
                _ => MONTHS[(mo - 1) as usize].to_string(),
            }),
            'y' => out.push_str(&if n <= 2 { format!("{:02}", y % 100) } else { format!("{:04}", y) }),
            'H' => out.push_str(&if n == 1 { h.to_string() } else { format!("{:02}", h) }),
            'h' => {
                let hh = if h % 12 == 0 { 12 } else { h % 12 };
                out.push_str(&if n == 1 { hh.to_string() } else { format!("{:02}", hh) })
            }
            'm' => out.push_str(&if n == 1 { mi.to_string() } else { format!("{:02}", mi) }),
            's' => out.push_str(&if n == 1 { s.to_string() } else { format!("{:02}", s) }),
            _ => {
                for _ in 0..n {
                    out.push(ch);
                }
            }
        }
        i += n;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(format_date("dddd, d MMMM yyyy", (2026, 10, 19, 9, 5, 0)), "Monday, 19 October 2026");
        assert_eq!(format_date("MM/dd/yy h:mm am/pm", (2026, 10, 19, 15, 5, 0)), "10/19/26 3:05 PM");
    }
}

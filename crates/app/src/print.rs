//! Printing to a real printer.
//!
//! On macOS and Linux the selected pages are exported as a PDF and handed to
//! CUPS (`lp`), which every Mac uses for printing. On Windows each page is
//! drawn into a bitmap and sent to the printer through GDI.

use std::collections::HashMap;

use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, PixmapPaint, Rect, Stroke, StrokeDash, Transform};
use wordcore::fonts::{FaceId, FontDb};
use wordcore::layout::{LineStyle, Page, Shape};
use wordcore::Color;

/// The printers on this computer and the name of the default one.
pub fn printers() -> (Vec<String>, Option<String>) {
    imp::printers()
}

/// A print request, run on a background thread.
pub struct Job {
    pub printer: String,
    pub copies: u32,
    pub title: String,
    /// Pages to print, already laid out.
    pub pages: Vec<Page>,
    /// The same pages as a PDF (used by CUPS).
    #[cfg_attr(windows, allow(dead_code))]
    pub pdf: Vec<u8>,
}

static RESULT: std::sync::Mutex<Option<(String, Result<(), String>)>> = std::sync::Mutex::new(None);

/// Start printing on a background thread; the outcome is collected with [`take_result`].
pub fn start(job: Job, ctx: egui::Context) {
    std::thread::spawn(move || {
        let printer = job.printer.clone();
        let r = run(job);
        if let Ok(mut g) = RESULT.lock() {
            *g = Some((printer, r));
        }
        ctx.request_repaint();
    });
}

/// The outcome of the last finished print job, if any: (printer, result).
pub fn take_result() -> Option<(String, Result<(), String>)> {
    RESULT.lock().ok().and_then(|mut g| g.take())
}

/// Send a job to the printer. Returns a short error message on failure.
pub fn run(job: Job) -> Result<(), String> {
    if job.pages.is_empty() {
        return Err("There are no pages in that range.".into());
    }
    imp::run(job)
}

/// Parse a page range such as "1-3, 5" into 0-based page indexes, keeping order
/// and dropping pages outside `1..=count`.
pub fn parse_range(s: &str, count: usize) -> Option<Vec<usize>> {
    let mut out = Vec::new();
    for part in s.split([',', ';']) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (a, b) = match part.split_once('-') {
            Some((a, b)) => {
                let a: usize = a.trim().parse().ok()?;
                let b = b.trim();
                let b: usize = if b.is_empty() { count } else { b.parse().ok()? };
                (a, b)
            }
            None => {
                let n: usize = part.parse().ok()?;
                (n, n)
            }
        };
        if a == 0 || b < a {
            return None;
        }
        for p in a..=b.min(count) {
            out.push(p - 1);
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

// ---------------------------------------------------------------------------
// Rendering a page into a bitmap
// ---------------------------------------------------------------------------

fn paint(c: Color) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c.r, c.g, c.b, 255);
    p.anti_alias = true;
    p
}

struct GlyphPath<'a> {
    pb: &'a mut PathBuilder,
    /// Maps font units (y up) to page points (y down).
    m: [f32; 6],
}

impl GlyphPath<'_> {
    fn tf(&self, x: f32, y: f32) -> (f32, f32) {
        let m = &self.m;
        (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
    }
}

impl ttf_parser::OutlineBuilder for GlyphPath<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        let (a, b) = self.tf(x, y);
        self.pb.move_to(a, b);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let (a, b) = self.tf(x, y);
        self.pb.line_to(a, b);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (a, b) = self.tf(x1, y1);
        let (c, d) = self.tf(x, y);
        self.pb.quad_to(a, b, c, d);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (a, b) = self.tf(x1, y1);
        let (c, d) = self.tf(x2, y2);
        let (e, f) = self.tf(x, y);
        self.pb.cubic_to(a, b, c, d, e, f);
    }
    fn close(&mut self) {
        self.pb.close();
    }
}

/// Draw one page at `dpi` dots per inch onto a white bitmap.
pub fn render_page(page: &Page, db: &FontDb, dpi: f32) -> Option<Pixmap> {
    let k = dpi / 72.0;
    let w = (page.width * k).round().max(1.0) as u32;
    let h = (page.height * k).round().max(1.0) as u32;
    let mut pm = Pixmap::new(w, h)?;
    pm.fill(tiny_skia::Color::WHITE);
    let base = Transform::from_scale(k, k);
    let mut faces: HashMap<FaceId, Option<ttf_parser::Face>> = HashMap::new();
    for s in &page.shapes {
        match s {
            Shape::Rect { x, y, w, h, color } => {
                if let Some(r) = Rect::from_xywh(*x, *y, *w, *h) {
                    pm.fill_rect(r, &paint(*color), base, None);
                }
            }
            Shape::Line { x1, y1, x2, y2, width, color, style } => draw_line(&mut pm, base, *x1, *y1, *x2, *y2, *width, *color, *style),
            Shape::Text { x, y, text, face, size, color, spacing, fake_bold, fake_italic, .. } => {
                if text.is_empty() || *size <= 0.0 || (*face as usize) >= db.faces.len() {
                    continue;
                }
                let f = db.face(*face);
                let tf = faces.entry(*face).or_insert_with(|| ttf_parser::Face::parse(&f.data, f.index).ok());
                let Some(tf) = tf.as_ref() else { continue };
                let s = size / f.units_per_em;
                let skew = if *fake_italic { 0.2 } else { 0.0 };
                let mut pb = PathBuilder::new();
                let mut pen = *x;
                for c in text.chars().filter(|c| !c.is_control()) {
                    let mut gp = GlyphPath { pb: &mut pb, m: [s, 0.0, skew * s, -s, pen, *y] };
                    let _ = tf.outline_glyph(ttf_parser::GlyphId(f.glyph(c)), &mut gp);
                    pen += f.advance(c) * size + spacing;
                }
                if let Some(path) = pb.finish() {
                    let p = paint(*color);
                    pm.fill_path(&path, &p, FillRule::Winding, base, None);
                    if *fake_bold {
                        let st = Stroke { width: size * 0.025, ..Stroke::default() };
                        pm.stroke_path(&path, &p, &st, base, None);
                    }
                }
            }
            Shape::Watermark { cx, cy, text, face, size, color, angle } => {
                if text.is_empty() || *size <= 0.0 || (*face as usize) >= db.faces.len() {
                    continue;
                }
                let f = db.face(*face);
                let tf = faces.entry(*face).or_insert_with(|| ttf_parser::Face::parse(&f.data, f.index).ok());
                let Some(tf) = tf.as_ref() else { continue };
                let s = size / f.units_per_em;
                let wd: f32 = text.chars().filter(|c| !c.is_control()).map(|c| f.advance(c) * size).sum();
                let mut pb = PathBuilder::new();
                let mut pen = -wd / 2.0;
                let base_y = (f.ascent - f.descent) * size / 2.0;
                for c in text.chars().filter(|c| !c.is_control()) {
                    let mut gp = GlyphPath { pb: &mut pb, m: [s, 0.0, 0.0, -s, pen, base_y] };
                    let _ = tf.outline_glyph(ttf_parser::GlyphId(f.glyph(c)), &mut gp);
                    pen += f.advance(c) * size;
                }
                if let Some(path) = pb.finish() {
                    let t = base.pre_translate(*cx, *cy).pre_rotate(-*angle);
                    pm.fill_path(&path, &paint(*color), FillRule::Winding, t, None);
                }
            }
            Shape::Image { x, y, w, h, image } => {
                let decoded = image::load_from_memory(&image.bytes).ok().map(|i| i.to_rgba8());
                let pix = decoded.and_then(|img| {
                    let (iw, ih) = img.dimensions();
                    let mut data = img.into_raw();
                    for px in data.chunks_exact_mut(4) {
                        let a = px[3] as u16;
                        for c in &mut px[..3] {
                            *c = ((*c as u16 * a + 127) / 255) as u8;
                        }
                    }
                    Pixmap::from_vec(data, tiny_skia::IntSize::from_wh(iw, ih)?)
                });
                match pix {
                    Some(img) if *w > 0.0 && *h > 0.0 => {
                        let t = base.pre_translate(*x, *y).pre_scale(w / img.width() as f32, h / img.height() as f32);
                        let pp = PixmapPaint { quality: tiny_skia::FilterQuality::Bilinear, ..PixmapPaint::default() };
                        pm.draw_pixmap(0, 0, img.as_ref(), &pp, t, None);
                    }
                    _ => {
                        if let Some(r) = Rect::from_xywh(*x, *y, *w, *h) {
                            pm.fill_rect(r, &paint(Color { r: 217, g: 217, b: 217 }), base, None);
                            let path = PathBuilder::from_rect(r);
                            let st = Stroke { width: 0.5, ..Stroke::default() };
                            pm.stroke_path(&path, &paint(Color { r: 153, g: 153, b: 153 }), &st, base, None);
                        }
                    }
                }
            }
        }
    }
    Some(pm)
}

#[allow(clippy::too_many_arguments)]
fn draw_line(pm: &mut Pixmap, base: Transform, x1: f32, y1: f32, x2: f32, y2: f32, width: f32, c: Color, style: LineStyle) {
    let width = width.max(0.1);
    let (dx, dy) = (x2 - x1, y2 - y1);
    let len = (dx * dx + dy * dy).sqrt();
    if len <= 0.0 {
        return;
    }
    let p = paint(c);
    let seg = |pb: &mut PathBuilder, ax: f32, ay: f32, bx: f32, by: f32| {
        pb.move_to(ax, ay);
        pb.line_to(bx, by);
    };
    match style {
        LineStyle::Solid | LineStyle::Dotted | LineStyle::Dashed => {
            let mut pb = PathBuilder::new();
            seg(&mut pb, x1, y1, x2, y2);
            let dash = match style {
                LineStyle::Dotted => {
                    let d = width.max(1.0);
                    StrokeDash::new(vec![d, d], 0.0)
                }
                LineStyle::Dashed => StrokeDash::new(vec![(width * 3.0).max(3.0), (width * 2.0).max(2.0)], 0.0),
                _ => None,
            };
            if let Some(path) = pb.finish() {
                pm.stroke_path(&path, &p, &Stroke { width, dash, ..Stroke::default() }, base, None);
            }
        }
        LineStyle::Double => {
            let lw = (width * 0.6).max(0.25);
            let off = (width * 0.9).max(lw * 1.5);
            let (nx, ny) = (-dy / len * off, dx / len * off);
            let mut pb = PathBuilder::new();
            for s in [-1.0, 1.0] {
                seg(&mut pb, x1 + nx * s, y1 + ny * s, x2 + nx * s, y2 + ny * s);
            }
            if let Some(path) = pb.finish() {
                pm.stroke_path(&path, &p, &Stroke { width: lw, ..Stroke::default() }, base, None);
            }
        }
        LineStyle::Wavy => {
            let amp = (width * 1.2).max(0.8);
            let half = amp * 2.0;
            let mut pb = PathBuilder::new();
            pb.move_to(0.0, 0.0);
            let mut x = 0.0;
            let mut up = true;
            while x < len - 1e-3 {
                let step = half.min(len - x);
                // y down in page space, so "up" is negative.
                let a = if up { -amp } else { amp } * (step / half);
                pb.cubic_to(x + step * 0.35, a * 1.33, x + step * 0.65, a * 1.33, x + step, 0.0);
                x += step;
                up = !up;
            }
            if let Some(path) = pb.finish() {
                let (ux, uy) = (dx / len, dy / len);
                let t = base.pre_concat(Transform::from_row(ux, uy, -uy, ux, x1, y1));
                let st = Stroke { width, line_cap: tiny_skia::LineCap::Round, line_join: tiny_skia::LineJoin::Round, ..Stroke::default() };
                pm.stroke_path(&path, &p, &st, t, None);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// macOS / Linux: CUPS
// ---------------------------------------------------------------------------

#[cfg(not(windows))]
mod imp {
    use super::Job;
    use std::process::Command;

    fn lpstat(args: &[&str]) -> String {
        Command::new("lpstat").args(args).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
    }

    pub fn printers() -> (Vec<String>, Option<String>) {
        // `lpstat -e` lists destination names one per line.
        let mut list: Vec<String> = lpstat(&["-e"]).lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
        if list.is_empty() {
            // Older CUPS: "NAME accepting requests since ..."
            list = lpstat(&["-a"]).lines().filter_map(|l| l.split_whitespace().next().map(str::to_string)).collect();
        }
        list.dedup();
        // "system default destination: NAME" (wording can be translated, so
        // take the text after the last colon).
        let def = lpstat(&["-d"]).lines().next().and_then(|l| l.rsplit(':').next()).map(|s| s.trim().to_string()).filter(|s| list.contains(s));
        (list, def)
    }

    pub fn run(job: Job) -> Result<(), String> {
        let safe: String = job.title.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect();
        let tmp = std::env::temp_dir().join(format!("{} (print).pdf", safe.trim()));
        std::fs::write(&tmp, &job.pdf).map_err(|e| format!("Couldn't prepare the pages: {}", e))?;
        let out = Command::new("lp")
            .arg("-d")
            .arg(&job.printer)
            .arg("-n")
            .arg(job.copies.max(1).to_string())
            .arg("-o")
            .arg("collate=true")
            .arg("-t")
            .arg(&job.title)
            .arg(&tmp)
            .output()
            .map_err(|e| format!("Couldn't reach the printing system: {}", e))?;
        if out.status.success() {
            Ok(())
        } else {
            let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
            Err(if msg.is_empty() { "The printer didn't accept the document.".into() } else { msg })
        }
    }
}

// ---------------------------------------------------------------------------
// Windows: GDI
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod imp {
    use super::Job;
    use windows_sys::Win32::Graphics::Gdi::*;
    use windows_sys::Win32::Graphics::Printing::*;
    use windows_sys::Win32::Storage::Xps::*;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    unsafe fn from_wide(p: *const u16) -> String {
        if p.is_null() {
            return String::new();
        }
        let mut n = 0;
        while *p.add(n) != 0 {
            n += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
    }

    pub fn printers() -> (Vec<String>, Option<String>) {
        let mut list = Vec::new();
        unsafe {
            let flags = PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS;
            let (mut needed, mut count) = (0u32, 0u32);
            EnumPrintersW(flags, std::ptr::null(), 4, std::ptr::null_mut(), 0, &mut needed, &mut count);
            if needed > 0 {
                // u64 buffer keeps the structures aligned.
                let mut buf = vec![0u64; (needed as usize).div_ceil(8)];
                if EnumPrintersW(flags, std::ptr::null(), 4, buf.as_mut_ptr() as *mut u8, needed, &mut needed, &mut count) != 0 {
                    let infos = std::slice::from_raw_parts(buf.as_ptr() as *const PRINTER_INFO_4W, count as usize);
                    for i in infos {
                        list.push(from_wide(i.pPrinterName));
                    }
                }
            }
        }
        let mut def = None;
        unsafe {
            let mut n = 0u32;
            GetDefaultPrinterW(std::ptr::null_mut(), &mut n);
            if n > 0 {
                let mut b = vec![0u16; n as usize];
                if GetDefaultPrinterW(b.as_mut_ptr(), &mut n) != 0 {
                    def = Some(from_wide(b.as_ptr()));
                }
            }
        }
        if let Some(d) = &def {
            if !list.contains(d) {
                list.insert(0, d.clone());
            }
        }
        (list, def)
    }

    pub fn run(job: Job) -> Result<(), String> {
        let db = wordcore::fonts::FontDb::global();
        let name = wide(&job.printer);
        let driver = wide("WINSPOOL");
        unsafe {
            let hdc = CreateDCW(driver.as_ptr(), name.as_ptr(), std::ptr::null(), std::ptr::null());
            if hdc.is_null() {
                return Err(format!("Couldn't connect to the printer \"{}\".", job.printer));
            }
            let res = print_on(hdc, &job, db);
            DeleteDC(hdc);
            res
        }
    }

    unsafe fn print_on(hdc: HDC, job: &Job, db: &wordcore::fonts::FontDb) -> Result<(), String> {
        let dpi_x = GetDeviceCaps(hdc, LOGPIXELSX as _).max(72) as f32;
        let dpi_y = GetDeviceCaps(hdc, LOGPIXELSY as _).max(72) as f32;
        let off_x = GetDeviceCaps(hdc, PHYSICALOFFSETX as _) as f32;
        let off_y = GetDeviceCaps(hdc, PHYSICALOFFSETY as _) as f32;
        let paper_w = GetDeviceCaps(hdc, PHYSICALWIDTH as _) as f32;
        let paper_h = GetDeviceCaps(hdc, PHYSICALHEIGHT as _) as f32;
        // Drawing resolution: the printer's, capped to keep memory reasonable.
        let dpi = dpi_x.min(dpi_y).min(300.0);

        let title = wide(&job.title);
        let di = DOCINFOW { cbSize: std::mem::size_of::<DOCINFOW>() as i32, lpszDocName: title.as_ptr(), lpszOutput: std::ptr::null(), lpszDatatype: std::ptr::null(), fwType: 0 };
        if StartDocW(hdc, &di) <= 0 {
            return Err("The printer didn't accept the document.".into());
        }
        SetStretchBltMode(hdc, HALFTONE as _);
        SetBrushOrgEx(hdc, 0, 0, std::ptr::null_mut());
        for _ in 0..job.copies.max(1) {
            for page in &job.pages {
                let Some(pm) = super::render_page(page, db, dpi) else { continue };
                let (mut w, mut h, mut rgb) = (pm.width() as i32, pm.height() as i32, to_bgr(&pm));
                let (mut pw, mut ph) = (page.width, page.height);
                // A landscape page on portrait paper (or the reverse) is turned.
                if paper_w > 0.0 && paper_h > 0.0 && (pw > ph) != (paper_w > paper_h) {
                    rgb = rotate_bgr(&rgb, w, h);
                    std::mem::swap(&mut w, &mut h);
                    std::mem::swap(&mut pw, &mut ph);
                }
                // Page size in printer dots, shrunk if the paper is smaller.
                let mut dw = pw / 72.0 * dpi_x;
                let mut dh = ph / 72.0 * dpi_y;
                if paper_w > 0.0 && paper_h > 0.0 {
                    let fit = (paper_w / dw).min(paper_h / dh).min(1.0);
                    dw *= fit;
                    dh *= fit;
                }
                if StartPage(hdc) <= 0 {
                    EndDoc(hdc);
                    return Err("The printer stopped responding.".into());
                }
                let mut bmi: BITMAPINFO = std::mem::zeroed();
                bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
                bmi.bmiHeader.biWidth = w;
                bmi.bmiHeader.biHeight = -h; // top-down rows
                bmi.bmiHeader.biPlanes = 1;
                bmi.bmiHeader.biBitCount = 24;
                bmi.bmiHeader.biCompression = BI_RGB;
                StretchDIBits(hdc, -off_x as i32, -off_y as i32, dw.round() as i32, dh.round() as i32, 0, 0, w, h, rgb.as_ptr() as *const _, &bmi, DIB_RGB_COLORS, SRCCOPY);
                if EndPage(hdc) <= 0 {
                    EndDoc(hdc);
                    return Err("The printer stopped responding.".into());
                }
            }
        }
        EndDoc(hdc);
        Ok(())
    }

    /// 24-bit BGR rows padded to 4 bytes, as GDI expects.
    fn to_bgr(pm: &tiny_skia::Pixmap) -> Vec<u8> {
        let (w, h) = (pm.width() as usize, pm.height() as usize);
        let stride = (w * 3 + 3) & !3;
        let mut out = vec![255u8; stride * h];
        for (y, row) in pm.data().chunks_exact(w * 4).enumerate() {
            let o = &mut out[y * stride..y * stride + w * 3];
            for (px, d) in row.chunks_exact(4).zip(o.chunks_exact_mut(3)) {
                d[0] = px[2];
                d[1] = px[1];
                d[2] = px[0];
            }
        }
        out
    }

    /// Rotate padded BGR rows 90 degrees clockwise.
    fn rotate_bgr(src: &[u8], w: i32, h: i32) -> Vec<u8> {
        let (w, h) = (w as usize, h as usize);
        let s_stride = (w * 3 + 3) & !3;
        let (nw, nh) = (h, w);
        let d_stride = (nw * 3 + 3) & !3;
        let mut out = vec![255u8; d_stride * nh];
        for y in 0..h {
            for x in 0..w {
                let s = y * s_stride + x * 3;
                let (nx, ny) = (h - 1 - y, x);
                let d = ny * d_stride + nx * 3;
                out[d..d + 3].copy_from_slice(&src[s..s + 3]);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::parse_range;

    #[test]
    fn ranges() {
        assert_eq!(parse_range("1-3, 5", 10), Some(vec![0, 1, 2, 4]));
        assert_eq!(parse_range("2-", 4), Some(vec![1, 2, 3]));
        assert_eq!(parse_range("9", 4), None);
        assert_eq!(parse_range("x", 4), None);
        assert_eq!(parse_range("3-1", 4), None);
    }
}

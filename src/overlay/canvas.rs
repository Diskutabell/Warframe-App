//! A small drawing surface for the overlays: boxes with rounded corners, pictures and text in the
//! app's bundled fonts, into premultiplied RGBA pixels that any window system can show.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use ab_glyph::{Font, FontArc, FontVec, PxScale, ScaleFont, VariableFont, point};
use tiny_skia::{FillRule, Paint, Path, PathBuilder, Pixmap, PixmapPaint, PremultipliedColorU8, Stroke, Transform};

/// A colour as CSS writes it: 0-255 channels and an opacity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Colour(pub u8, pub u8, pub u8, pub f32);

impl Colour {
    pub const fn hex(rgb: u32) -> Colour {
        Colour((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, 1.0)
    }

    fn paint(self) -> Paint<'static> {
        let mut p = Paint::default();
        p.set_color_rgba8(self.0, self.1, self.2, (self.3.clamp(0.0, 1.0) * 255.0).round() as u8);
        p.anti_alias = true;
        p
    }
}

/// The bundled typefaces (web/public/fonts), as the main window uses them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Face {
    /// Rajdhani SemiBold: names, small figures.
    Label,
    /// Rajdhani Bold: prices, grades, the weapon.
    Display,
    /// Source Sans 3 Regular: running text.
    Text,
    /// Source Sans 3 Bold.
    TextBold,
}

struct Fonts {
    label: FontArc,
    display: FontArc,
    text: FontArc,
    text_bold: FontArc,
}

fn load(name: &str) -> FontVec {
    let file = crate::api::Web::get(&format!("fonts/{name}")).unwrap_or_else(|| panic!("font {name} is not embedded"));
    FontVec::try_from_vec(file.data.into_owned()).unwrap_or_else(|e| panic!("font {name}: {e}"))
}

fn fonts() -> &'static Fonts {
    static FONTS: OnceLock<Fonts> = OnceLock::new();
    FONTS.get_or_init(|| {
        let text = load("SourceSans3-Variable.ttf");
        let mut bold = load("SourceSans3-Variable.ttf");
        bold.set_variation(b"wght", 700.0);
        Fonts {
            label: FontArc::new(load("Rajdhani-SemiBold.ttf")),
            display: FontArc::new(load("Rajdhani-Bold.ttf")),
            text: FontArc::new(text),
            text_bold: FontArc::new(bold),
        }
    })
}

/// Text in one face, size and colour. `px` is the size as CSS means it (the em box).
#[derive(Clone, Copy, Debug)]
pub struct Style {
    pub face: Face,
    pub px: f32,
    pub colour: Colour,
}

impl Style {
    pub const fn new(face: Face, px: f32, colour: Colour) -> Style {
        Style { face, px, colour }
    }

    fn font(&self) -> &'static FontArc {
        let f = fonts();
        match self.face {
            Face::Label => &f.label,
            Face::Display => &f.display,
            Face::Text => &f.text,
            Face::TextBold => &f.text_bold,
        }
    }

    /// ab_glyph scales by the font's whole height; CSS sizes the em box.
    fn scale(&self) -> PxScale {
        let font = self.font();
        let em = font.units_per_em().unwrap_or(1000.0);
        PxScale::from(self.px * font.height_unscaled() / em)
    }

    pub fn ascent(&self) -> f32 {
        self.font().as_scaled(self.scale()).ascent()
    }

    /// The height of a line, as Pango gives it: ascent plus descent.
    pub fn line(&self) -> f32 {
        let f = self.font().as_scaled(self.scale());
        f.ascent() - f.descent()
    }

    pub fn width(&self, text: &str) -> f32 {
        let f = self.font().as_scaled(self.scale());
        let mut w = 0.0;
        let mut last = None;
        for c in text.chars() {
            let id = f.glyph_id(c);
            if let Some(prev) = last {
                w += f.kern(prev, id);
            }
            w += f.h_advance(id);
            last = Some(id);
        }
        w
    }

    /// `text`, cut down with an ellipsis until it fits in `max` pixels.
    pub fn fit(&self, text: &str, max: f32) -> String {
        if self.width(text) <= max {
            return text.to_string();
        }
        let mut chars: Vec<char> = text.chars().collect();
        while !chars.is_empty() {
            chars.pop();
            let cut: String = chars.iter().collect::<String>().trim_end().to_string() + "…";
            if self.width(&cut) <= max {
                return cut;
            }
        }
        String::new()
    }

    /// `text` broken into lines no wider than `max` pixels, at spaces where it can be.
    pub fn wrap(&self, text: &str, max: f32) -> Vec<String> {
        let mut lines = Vec::new();
        let mut line = String::new();
        for word in text.split_whitespace() {
            let next = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if self.width(&next) <= max || line.is_empty() {
                line = next;
            } else {
                lines.push(std::mem::replace(&mut line, word.to_string()));
            }
        }
        if !line.is_empty() {
            lines.push(line);
        }
        lines
    }
}

pub struct Canvas {
    pub pixmap: Pixmap,
}

impl Canvas {
    /// A transparent canvas; at least one pixel each way.
    pub fn new(width: u32, height: u32) -> Canvas {
        Canvas { pixmap: Pixmap::new(width.max(1), height.max(1)).expect("a canvas of at least 1x1") }
    }

    fn rounded(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<Path> {
        let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
        let mut pb = PathBuilder::new();
        pb.move_to(x + r, y);
        pb.line_to(x + w - r, y);
        pb.quad_to(x + w, y, x + w, y + r);
        pb.line_to(x + w, y + h - r);
        pb.quad_to(x + w, y + h, x + w - r, y + h);
        pb.line_to(x + r, y + h);
        pb.quad_to(x, y + h, x, y + h - r);
        pb.line_to(x, y + r);
        pb.quad_to(x, y, x + r, y);
        pb.close();
        pb.finish()
    }

    pub fn fill(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, colour: Colour) {
        if let Some(path) = Self::rounded(x, y, w, h, radius) {
            self.pixmap.fill_path(&path, &colour.paint(), FillRule::Winding, Transform::identity(), None);
        }
    }

    /// A band along the top of a box with rounded corners, square below (a coloured top border).
    pub fn fill_top(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, colour: Colour) {
        let r = radius.min(w / 2.0).min(h).max(0.0);
        let mut pb = PathBuilder::new();
        pb.move_to(x, y + h);
        pb.line_to(x, y + r);
        pb.quad_to(x, y, x + r, y);
        pb.line_to(x + w - r, y);
        pb.quad_to(x + w, y, x + w, y + r);
        pb.line_to(x + w, y + h);
        pb.close();
        if let Some(path) = pb.finish() {
            self.pixmap.fill_path(&path, &colour.paint(), FillRule::Winding, Transform::identity(), None);
        }
    }

    /// A one-pixel outline just inside the box.
    pub fn outline(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, colour: Colour) {
        if let Some(path) = Self::rounded(x + 0.5, y + 0.5, w - 1.0, h - 1.0, radius) {
            let stroke = Stroke { width: 1.0, ..Stroke::default() };
            self.pixmap.stroke_path(&path, &colour.paint(), &stroke, Transform::identity(), None);
        }
    }

    /// A picture at its own size.
    pub fn picture(&mut self, img: &Pixmap, x: f32, y: f32) {
        self.pixmap.draw_pixmap(
            x.round() as i32,
            y.round() as i32,
            img.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            None,
        );
    }

    /// `text` with its baseline at `y`, from `x`; where it ends.
    pub fn text(&mut self, style: Style, x: f32, y: f32, text: &str) -> f32 {
        let font = style.font();
        let scaled = font.as_scaled(style.scale());
        let Colour(r, g, b, a) = style.colour;
        let (width, height) = (self.pixmap.width() as i32, self.pixmap.height() as i32);
        let pixels = self.pixmap.pixels_mut();
        let mut pen = x;
        let mut last = None;
        for c in text.chars() {
            let id = scaled.glyph_id(c);
            if let Some(prev) = last {
                pen += scaled.kern(prev, id);
            }
            let glyph = id.with_scale_and_position(style.scale(), point(pen, y));
            if let Some(outline) = font.outline_glyph(glyph) {
                let bounds = outline.px_bounds();
                outline.draw(|gx, gy, coverage| {
                    let (px, py) = (bounds.min.x as i32 + gx as i32, bounds.min.y as i32 + gy as i32);
                    if px < 0 || py < 0 || px >= width || py >= height {
                        return;
                    }
                    let alpha = (coverage.clamp(0.0, 1.0) * a).min(1.0);
                    let dst = &mut pixels[(py * width + px) as usize];
                    let keep = 1.0 - alpha;
                    let mix = |src: u8, d: u8| (src as f32 * alpha + d as f32 * keep).round().min(255.0) as u8;
                    let na = (255.0 * alpha + dst.alpha() as f32 * keep).round().min(255.0) as u8;
                    let (nr, ng, nb) = (mix(r, dst.red()), mix(g, dst.green()), mix(b, dst.blue()));
                    if let Some(c) = PremultipliedColorU8::from_rgba(nr.min(na), ng.min(na), nb.min(na), na) {
                        *dst = c;
                    }
                });
            }
            pen += scaled.h_advance(id);
            last = Some(id);
        }
        pen
    }
}

/// A picture `px` pixels high (and as wide as its proportions make it), ready to draw.
fn scaled(img: image::RgbaImage, px: u32) -> Option<Pixmap> {
    let px = px.max(1);
    let w = ((img.width() as f64 * px as f64 / img.height().max(1) as f64).round() as u32).max(1);
    let img = image::imageops::resize(&img, w, px, image::imageops::FilterType::Lanczos3);
    let mut pixmap = Pixmap::new(w, px)?;
    for (dst, src) in pixmap.pixels_mut().iter_mut().zip(img.pixels()) {
        let [r, g, b, a] = src.0;
        let pre = |c: u8| ((c as u16 * a as u16 + 127) / 255) as u8;
        *dst = PremultipliedColorU8::from_rgba(pre(r), pre(g), pre(b), a)?;
    }
    Some(pixmap)
}

/// A bundled currency icon (`web/public/icons/<name>.png`), `px` pixels high.
pub fn icon(name: &'static str, px: u32) -> Option<Arc<Pixmap>> {
    type Icons = Mutex<HashMap<(&'static str, u32), Arc<Pixmap>>>;
    static ICONS: OnceLock<Icons> = OnceLock::new();
    let icons = ICONS.get_or_init(Default::default);
    if let Some(i) = icons.lock().ok()?.get(&(name, px)) {
        return Some(i.clone());
    }
    let file = crate::api::Web::get(&format!("icons/{name}.png"))?;
    let img = image::load_from_memory(&file.data).ok()?.to_rgba8();
    let icon = Arc::new(scaled(img, px)?);
    icons.lock().ok()?.insert((name, px), icon.clone());
    Some(icon)
}

/// An item picture from the art cache, `px` square, if it is there: an overlay never waits for a
/// download.
pub fn art(url: &str, px: u32) -> Option<Pixmap> {
    let art = crate::art::cached(url, 128)?;
    scaled(image::load_from_memory(&art.bytes).ok()?.to_rgba8(), px)
}

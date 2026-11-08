//! Pixel work for OCR, matching Pillow bit for bit.
//!
//! The reward matcher was tuned on what tesseract read from Pillow's output, so the variants here
//! reproduce Pillow's arithmetic exactly: ITU-R 601 luma with its rounding, `autocontrast`'s
//! truncating lookup table, and the two-pass fixed-point bicubic resampler. A "close enough"
//! resize changes what tesseract reads on marginal glyphs.

/// 8-bit RGB, row-major, 3 bytes per pixel.
#[derive(Clone)]
pub struct Rgb {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

/// 8-bit greyscale, row-major.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Gray {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

impl Rgb {
    pub fn from_image(img: image::RgbImage) -> Rgb {
        let (w, h) = img.dimensions();
        Rgb { width: w as usize, height: h as usize, data: img.into_raw() }
    }

    /// The pixels inside [x0, x1) x [y0, y1); the box must lie inside the image.
    pub fn crop(&self, x0: usize, y0: usize, x1: usize, y1: usize) -> Rgb {
        let (w, h) = (x1 - x0, y1 - y0);
        let mut data = Vec::with_capacity(w * h * 3);
        for y in y0..y1 {
            let row = (y * self.width + x0) * 3;
            data.extend_from_slice(&self.data[row..row + w * 3]);
        }
        Rgb { width: w, height: h, data }
    }

    /// Pillow's `convert("L")`: L = (R*19595 + G*38470 + B*7471 + 0x8000) >> 16.
    pub fn luma(&self) -> Gray {
        let data = self
            .data
            .chunks_exact(3)
            .map(|p| ((p[0] as u32 * 19595 + p[1] as u32 * 38470 + p[2] as u32 * 7471 + 0x8000) >> 16) as u8)
            .collect();
        Gray { width: self.width, height: self.height, data }
    }

    pub fn channel(&self, c: usize) -> Gray {
        let data = self.data.chunks_exact(3).map(|p| p[c]).collect();
        Gray { width: self.width, height: self.height, data }
    }

    /// How much redder each pixel is than it is green or blue: red text stands out, white and
    /// gold (red and green both high) do not.
    pub fn redness(&self) -> Gray {
        let data = self.data.chunks_exact(3).map(|p| p[0].saturating_sub(p[1].max(p[2]))).collect();
        Gray { width: self.width, height: self.height, data }
    }
}

impl Gray {
    /// Pillow's `ImageOps.autocontrast(img)` with no cutoff: stretch the darkest and brightest
    /// values present to 0 and 255. Pillow truncates (`int(...)`), it does not round.
    pub fn autocontrast(&self) -> Gray {
        let mut hist = [0usize; 256];
        for &v in &self.data {
            hist[v as usize] += 1;
        }
        let lo = hist.iter().position(|&n| n > 0).unwrap_or(0);
        let hi = hist.iter().rposition(|&n| n > 0).unwrap_or(0);
        if hi <= lo {
            return self.clone();
        }
        let scale = 255.0 / (hi - lo) as f64;
        let offset = -(lo as f64) * scale;
        let mut lut = [0u8; 256];
        for (i, slot) in lut.iter_mut().enumerate() {
            *slot = ((i as f64 * scale + offset) as i64).clamp(0, 255) as u8;
        }
        self.map(|v| lut[v as usize])
    }

    pub fn invert(&self) -> Gray {
        self.map(|v| 255 - v)
    }

    pub fn threshold(&self, above: u8) -> Gray {
        self.map(|v| if v > above { 255 } else { 0 })
    }

    /// Pillow's `ImageChops.subtract(self, other)`: clip(self - other) to 0..255.
    pub fn subtract(&self, other: &Gray) -> Gray {
        let data = self.data.iter().zip(&other.data).map(|(&a, &b)| a.saturating_sub(b)).collect();
        Gray { width: self.width, height: self.height, data }
    }

    fn map(&self, f: impl Fn(u8) -> u8) -> Gray {
        Gray { width: self.width, height: self.height, data: self.data.iter().map(|&v| f(v)).collect() }
    }

    /// Pillow's `resize((w, h))`, whose default filter is bicubic (a = -0.5): a horizontal pass
    /// into an 8-bit intermediate, then a vertical pass, both in 22-bit fixed point.
    pub fn resize_bicubic(&self, out_w: usize, out_h: usize) -> Gray {
        let need_h = out_w != self.width;
        let need_v = out_h != self.height;
        let (kh, bounds_h, ksize_h) = coefficients(self.width, out_w);
        let (kv, mut bounds_v, ksize_v) = coefficients(self.height, out_h);
        // Rows of the source the vertical pass will actually read.
        let first = bounds_v[0].0;
        let last = bounds_v[out_h - 1].0 + bounds_v[out_h - 1].1;

        let mut src = self.clone();
        if need_h {
            for b in &mut bounds_v {
                b.0 -= first;
            }
            let rows = last - first;
            let mut tmp = vec![0u8; out_w * rows];
            for y in 0..rows {
                let row = &self.data[(y + first) * self.width..(y + first + 1) * self.width];
                for x in 0..out_w {
                    let (xmin, xmax) = bounds_h[x];
                    let k = &kh[x * ksize_h..x * ksize_h + xmax];
                    let mut acc: i32 = 1 << (PRECISION_BITS - 1);
                    for (i, &w) in k.iter().enumerate() {
                        acc += row[xmin + i] as i32 * w;
                    }
                    tmp[y * out_w + x] = clip8(acc);
                }
            }
            src = Gray { width: out_w, height: rows, data: tmp };
        }
        if !need_v {
            return src;
        }
        let mut out = vec![0u8; src.width * out_h];
        for y in 0..out_h {
            let (ymin, ymax) = bounds_v[y];
            let k = &kv[y * ksize_v..y * ksize_v + ymax];
            for x in 0..src.width {
                let mut acc: i32 = 1 << (PRECISION_BITS - 1);
                for (i, &w) in k.iter().enumerate() {
                    acc += src.data[(ymin + i) * src.width + x] as i32 * w;
                }
                out[y * src.width + x] = clip8(acc);
            }
        }
        Gray { width: src.width, height: out_h, data: out }
    }

    /// Binary PGM, which tesseract reads from stdin as readily as PNG and costs nothing to encode.
    pub fn to_pgm(&self) -> Vec<u8> {
        let mut out = format!("P5\n{} {}\n255\n", self.width, self.height).into_bytes();
        out.extend_from_slice(&self.data);
        out
    }
}

const PRECISION_BITS: u32 = 32 - 8 - 2;

fn clip8(v: i32) -> u8 {
    if v >= (1 << PRECISION_BITS << 8) {
        255
    } else if v <= 0 {
        0
    } else {
        (v >> PRECISION_BITS) as u8
    }
}

fn bicubic(x: f64) -> f64 {
    const A: f64 = -0.5;
    let x = x.abs();
    if x < 1.0 {
        ((A + 2.0) * x - (A + 3.0)) * x * x + 1.0
    } else if x < 2.0 {
        (((x - 5.0) * x + 8.0) * x - 4.0) * A
    } else {
        0.0
    }
}

/// Pillow's `precompute_coeffs` + `normalize_coeffs_8bpc` for one axis: per output pixel, the
/// first source index, the tap count, and the fixed-point weights (`ksize` slots each).
fn coefficients(in_size: usize, out_size: usize) -> (Vec<i32>, Vec<(usize, usize)>, usize) {
    const SUPPORT: f64 = 2.0;
    let scale = in_size as f64 / out_size as f64;
    let filterscale = scale.max(1.0);
    let support = SUPPORT * filterscale;
    let ksize = support.ceil() as usize * 2 + 1;
    let mut weights = vec![0i32; out_size * ksize];
    let mut bounds = Vec::with_capacity(out_size);
    let mut k = vec![0f64; ksize];
    for xx in 0..out_size {
        let center = (xx as f64 + 0.5) * scale;
        let ss = 1.0 / filterscale;
        let xmin = ((center - support + 0.5) as i64).max(0) as usize;
        let xmax = ((center + support + 0.5) as i64).min(in_size as i64) as usize - xmin;
        let mut ww = 0.0;
        for (x, slot) in k.iter_mut().enumerate().take(xmax) {
            let w = bicubic((x as f64 + xmin as f64 - center + 0.5) * ss);
            *slot = w;
            ww += w;
        }
        for x in 0..ksize {
            let w = if x < xmax && ww != 0.0 {
                k[x] / ww
            } else if x < xmax {
                k[x]
            } else {
                0.0
            };
            let fixed = w * (1i64 << PRECISION_BITS) as f64;
            weights[xx * ksize + x] = if w < 0.0 { (-0.5 + fixed) as i32 } else { (0.5 + fixed) as i32 };
        }
        bounds.push((xmin, xmax));
    }
    (weights, bounds, ksize)
}

/// The images tesseract reads on the reward screen.
///
/// Reward names sit just above the vertical centre; ultrawide screens keep a 16:9 UI area. The
/// band is deliberately generous - the leftmost card sits near x=0.10 on a 4-reward screen, and
/// a clipped name only ever fuzzy-matches by luck.
///
/// The names are light blue and the item renders gold and white, and every render bleeds behind
/// the first line of the name under it. In luminance those are the same value, so a grey
/// threshold eats the overlapped letters ("Yareli Prime Chassis" came back as "Yarel asslIs").
/// Blue minus red separates them by hue: the text survives and the gold drops to zero. The grey
/// pair stays as the fallback for other themes; red minus blue also isolates gold text.
pub fn reward_variants(frame: &Rgb) -> [Gray; 6] {
    let (w, h) = (frame.width as i64, frame.height as i64);
    let ui_w = w.min(((h * 16) as f64 / 9.0) as i64);
    let x0 = (w - ui_w) / 2;
    let x_lo = (x0 + (ui_w as f64 * 0.07) as i64) as usize;
    let x_hi = (x0 + (ui_w as f64 * 0.93) as i64) as usize;
    let y_lo = (h as f64 * 0.27) as usize;
    let y_hi = (h as f64 * 0.56) as usize;
    let band_rgb = frame.crop(x_lo, y_lo, x_hi, y_hi);

    let grey = band_rgb.luma().autocontrast();
    let grey = grey.resize_bicubic(grey.width * 2, grey.height * 2);
    let blue = band_rgb.channel(2).subtract(&band_rgb.channel(0)).autocontrast();
    let blue = blue.resize_bicubic(blue.width * 2, blue.height * 2);
    let gold = band_rgb.channel(0).subtract(&band_rgb.channel(2)).autocontrast();
    let gold = gold.resize_bicubic(gold.width * 2, gold.height * 2);
    let inverted = grey.invert();
    let blue_hard = blue.threshold(120);
    let gold_hard = gold.threshold(120);
    [grey, inverted, blue, blue_hard, gold, gold_hard]
}

/// The images tesseract reads of the reward cards' name rows: dark text on white, enlarged 2x.
///
/// The names are drawn in the interface theme's colour. Blue minus red keeps the blue and cyan
/// themes' text and drops the gold item renders that reach into the first line; redness keeps
/// red themes' text (Stalker) and drops white and gold renders, which red minus blue keeps -
/// that one is for the gold themes; brightness alone keeps the white and grey ones. On real
/// frames each variant took ~0.18 s, against ~1 s for the six variants of the old wide band.
pub fn name_variants(band: &Rgb) -> [Gray; 4] {
    let blue = band.channel(2).subtract(&band.channel(0));
    let gold = band.channel(0).subtract(&band.channel(2));
    [blue, band.redness(), gold, band.luma()].map(|g| {
        let g = g.autocontrast().invert();
        g.resize_bicubic(g.width * 2, g.height * 2)
    })
}

/// The whole frame for a riven card, which can sit anywhere: grey, stretched, enlarged 1.5x.
pub fn riven_frame(frame: &Rgb) -> Gray {
    let grey = frame.luma().autocontrast();
    grey.resize_bicubic(grey.width * 3 / 2, grey.height * 3 / 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luma_matches_pillow_rounding() {
        let px = Rgb { width: 3, height: 1, data: vec![255, 255, 255, 255, 0, 0, 10, 200, 30] };
        assert_eq!(px.luma().data, vec![255, 76, 124]);
    }

    #[test]
    fn autocontrast_of_a_flat_image_is_a_no_op() {
        let g = Gray { width: 2, height: 1, data: vec![7, 7] };
        assert_eq!(g.autocontrast(), g);
    }

    #[test]
    fn identity_resize_returns_the_input() {
        let g = Gray { width: 3, height: 2, data: vec![1, 2, 3, 4, 5, 6] };
        assert_eq!(g.resize_bicubic(3, 2), g);
    }

    #[test]
    fn name_variants_are_dark_text_on_white_at_twice_the_size() {
        // Blue text on a dark background, one pixel each.
        let band = Rgb { width: 2, height: 1, data: vec![57, 105, 192, 20, 16, 24] };
        let [blue, red, gold, grey] = name_variants(&band);
        for v in [&blue, &red, &gold, &grey] {
            assert_eq!((v.width, v.height), (4, 2));
        }
        // The text pixel comes out darker than the background in the blue and grey images.
        assert!(blue.data[0] < blue.data[3]);
        assert!(grey.data[0] < grey.data[3]);
    }

    #[test]
    fn reward_variants_include_gold_text() {
        let mut frame = Rgb { width: 100, height: 100, data: vec![0; 100 * 100 * 3] };
        let i = (40 * frame.width + 50) * 3;
        frame.data[i..i + 3].copy_from_slice(&[200, 150, 40]);
        let variants = reward_variants(&frame);
        assert!(variants[4].data.iter().any(|&pixel| pixel > 0));
        assert!(variants[5].data.contains(&255));
    }
}

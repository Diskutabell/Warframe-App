//! Procedural 32x32 tray icon shared by the Linux (StatusNotifierItem) and Windows
//! (`Shell_NotifyIconW`) system trays. Keeping it in code avoids depending on a system icon
//! theme and guarantees identical rendering on both platforms.

pub const SIZE: u32 = 32;

/// Renders a 32x32 RGBA8 icon: a crisp matte-obsidian badge with a brushed Orokin gold border
/// and geometric diamond/lotus emblem that remains legible when scaled down to 16-18 px.
pub fn rgba() -> Vec<u8> {
    let n = SIZE as usize;
    let mut out = vec![0u8; n * n * 4];
    let center = SIZE as f32 * 0.5;

    for y in 0..n {
        for x in 0..n {
            // 2x2 supersampling keeps diagonal edges crisp at 18px in Quickshell / Windows tray.
            let mut acc = [0.0f32; 4];
            for sy in 0..2 {
                for sx in 0..2 {
                    let px = x as f32 + (sx as f32 + 0.5) * 0.5 - center;
                    let py = y as f32 + (sy as f32 + 0.5) * 0.5 - center;
                    let col = sample(px, py);
                    let a = col[3];
                    acc[0] += col[0] * a;
                    acc[1] += col[1] * a;
                    acc[2] += col[2] * a;
                    acc[3] += a;
                }
            }
            let a = acc[3] * 0.25;
            let idx = (y * n + x) * 4;
            if a > 1e-4 {
                out[idx] = ((acc[0] / acc[3]).clamp(0.0, 1.0) * 255.0).round() as u8;
                out[idx + 1] = ((acc[1] / acc[3]).clamp(0.0, 1.0) * 255.0).round() as u8;
                out[idx + 2] = ((acc[2] / acc[3]).clamp(0.0, 1.0) * 255.0).round() as u8;
                out[idx + 3] = (a.clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        }
    }
    out
}

/// Evaluates one point in icon space (`-15.5..=15.5` on both axes) as `[r, g, b, a]` in `0..=1`.
fn sample(x: f32, y: f32) -> [f32; 4] {
    let ax = x.abs();
    let ay = y.abs();
    // Rounded-diamond outer shield (L1 + L_inf blend gives crisp chamfered corners).
    let oct = ax.max(ay).max((ax + ay) * 0.72);
    if oct > 14.2 {
        return [0.0, 0.0, 0.0, 0.0];
    }

    // Matte obsidian backdrop matching the app window (#0f1116).
    let mut color = [0.075, 0.086, 0.114, 1.0];

    // Brushed gold rim.
    if oct > 12.1 {
        color = [0.812, 0.690, 0.420, 1.0];
    }

    // Center lotus/diamond core: a tall vertical diamond flanked by two angled petals.
    let core = (ax / 4.4) + (ay / 9.2);
    let inner_cut = (ax / 2.2) + (ay / 4.2);
    if core <= 1.0 && inner_cut >= 1.0 {
        // Warm highlight gold (#edd596).
        color = [0.929, 0.835, 0.588, 1.0];
    }

    // Left and right swept petals.
    let wing_x = ax - 5.2;
    let wing_y = y + 1.2 - wing_x * 0.42;
    if (3.8..=10.6).contains(&ax) && wing_y.abs() <= 2.4 - (ax - 6.8).abs() * 0.36 {
        color = [0.812, 0.690, 0.420, 1.0];
    }

    // Small lotus teal accent pip at the heart (#5cc4be).
    if ax + ay <= 2.4 {
        color = [0.361, 0.769, 0.745, 1.0];
    }

    color
}

/// Big-endian ARGB32 (`[a, r, g, b]`) expected by D-Bus `StatusNotifierItem.IconPixmap`.
#[cfg(unix)]
pub fn argb32_be() -> Vec<u8> {
    let src = rgba();
    let mut out = Vec::with_capacity(src.len());
    for px in src.chunks_exact(4) {
        out.extend_from_slice(&[px[3], px[0], px[1], px[2]]);
    }
    out
}

/// Little-endian BGRA32 (`[b, g, r, a]`) expected by Win32 32-bit `CreateDIBSection` icons.
#[cfg(any(windows, test))]
pub fn bgra32() -> Vec<u8> {
    let src = rgba();
    let mut out = Vec::with_capacity(src.len());
    for px in src.chunks_exact(4) {
        out.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_icon_has_expected_dimensions_and_opaque_emblem() {
        let px = rgba();
        assert_eq!(px.len(), (SIZE * SIZE * 4) as usize);
        // Far corners outside the chamfered shield are transparent.
        assert_eq!(px[3], 0);
        // Center pixel is opaque teal pip.
        let mid = ((16 * SIZE + 16) * 4) as usize;
        assert_eq!(px[mid + 3], 255);
        assert!(px[mid + 1] > 150 && px[mid + 2] > 150);
        assert_eq!(bgra32().len(), px.len());
        #[cfg(unix)]
        assert_eq!(argb32_be().len(), px.len());
    }
}

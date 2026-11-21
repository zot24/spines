use crate::domain::Palette;

/// Spine colors from cover pixels. Same buckets as bookshelf `palette.ts` at 163a6445.
pub fn palette_from_rgba(rgba: &[u8]) -> Palette {
    #[derive(Clone, Copy)]
    struct Bucket {
        count: u64,
        sum: [f64; 3],
    }
    let mut buckets = std::collections::HashMap::<u32, Bucket>::new();
    let mut i = 0;
    while i + 3 < rgba.len() {
        let a = if i + 3 < rgba.len() { rgba[i + 3] } else { 255 };
        if a >= 128 {
            let px = [rgba[i] as f64, rgba[i + 1] as f64, rgba[i + 2] as f64];
            let key = ((rgba[i] >> 4) as u32) << 8
                | ((rgba[i + 1] >> 4) as u32) << 4
                | ((rgba[i + 2] >> 4) as u32);
            let bucket = buckets.entry(key).or_insert(Bucket {
                count: 0,
                sum: [0.0, 0.0, 0.0],
            });
            bucket.count += 1;
            bucket.sum[0] += px[0];
            bucket.sum[1] += px[1];
            bucket.sum[2] += px[2];
        }
        i += 4;
    }
    let mut colors: Vec<(u64, [f64; 3])> = buckets
        .values()
        .map(|b| {
            let rgb = [
                b.sum[0] / b.count as f64,
                b.sum[1] / b.count as f64,
                b.sum[2] / b.count as f64,
            ];
            (b.count, rgb)
        })
        .collect();
    colors.sort_by_key(|color| std::cmp::Reverse(color.0));
    let total: u64 = colors.iter().map(|c| c.0).sum();
    if total == 0 {
        return Palette {
            bg: hex(INK),
            fg: hex(PAPER),
            accent: hex(PAPER),
        };
    }
    let bg = colors
        .iter()
        .max_by(|a, b| {
            score(a.0, a.1)
                .partial_cmp(&score(b.0, b.1))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|c| c.1)
        .unwrap_or(INK);
    let fg = if contrast(bg, PAPER) >= contrast(bg, INK) {
        PAPER
    } else {
        INK
    };
    let accent = colors
        .iter()
        .filter(|c| c.0 as f64 / total as f64 >= 0.01 && contrast(c.1, bg) >= 3.0)
        .max_by(|a, b| {
            (saturation(a.1) * a.0 as f64)
                .partial_cmp(&(saturation(b.1) * b.0 as f64))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|c| c.1)
        .unwrap_or(fg);
    Palette {
        bg: hex(bg),
        fg: hex(fg),
        accent: hex(accent),
    }
}

const INK: [f64; 3] = [20.0, 18.0, 16.0];
const PAPER: [f64; 3] = [246.0, 242.0, 232.0];

const FALLBACKS: [(&str, &str, &str); 6] = [
    ("#f2c230", "#141210", "#141210"),
    ("#1f4fa8", "#f6f2e8", "#f2c230"),
    ("#e8e4da", "#141210", "#c8321e"),
    ("#2b3a2e", "#e9d9a6", "#e9d9a6"),
    ("#d8436f", "#141210", "#f6f2e8"),
    ("#2fe06a", "#141210", "#141210"),
];

pub fn fallback_palette(calibre_id: i64) -> Palette {
    let i = (calibre_id.rem_euclid(FALLBACKS.len() as i64)) as usize;
    let (bg, fg, accent) = FALLBACKS[i];
    Palette {
        bg: bg.to_string(),
        fg: fg.to_string(),
        accent: accent.to_string(),
    }
}

fn score(count: u64, rgb: [f64; 3]) -> f64 {
    count as f64 * (1.0 + 2.0 * saturation(rgb))
}

fn saturation(rgb: [f64; 3]) -> f64 {
    let max = rgb[0].max(rgb[1]).max(rgb[2]);
    if max == 0.0 {
        0.0
    } else {
        (max - rgb[0].min(rgb[1]).min(rgb[2])) / max
    }
}

fn channel(c: f64) -> f64 {
    let s = c / 255.0;
    if s <= 0.03928 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

fn luminance(rgb: [f64; 3]) -> f64 {
    0.2126 * channel(rgb[0]) + 0.7152 * channel(rgb[1]) + 0.0722 * channel(rgb[2])
}

fn contrast(a: [f64; 3], b: [f64; 3]) -> f64 {
    let (hi, lo) = {
        let la = luminance(a);
        let lb = luminance(b);
        if la >= lb {
            (la, lb)
        } else {
            (lb, la)
        }
    };
    (hi + 0.05) / (lo + 0.05)
}

fn hex(rgb: [f64; 3]) -> String {
    format!(
        "#{:02x}{:02x}{:02x}",
        rgb[0].round().clamp(0.0, 255.0) as u8,
        rgb[1].round().clamp(0.0, 255.0) as u8,
        rgb[2].round().clamp(0.0, 255.0) as u8
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solid_red_keeps_red_and_uses_ink_for_text() {
        let px = [255u8, 0, 0, 255];
        let p = palette_from_rgba(&px);
        assert_eq!(p.bg, "#ff0000");
        assert_eq!(p.fg, "#141210");
        assert_eq!(p.accent, "#141210");
    }

    #[test]
    fn empty_pixels_use_ink_and_paper() {
        let p = palette_from_rgba(&[]);
        assert_eq!(p.bg, "#141210");
        assert_eq!(p.fg, "#f6f2e8");
        assert_eq!(p.accent, "#f6f2e8");
    }

    #[test]
    fn fallback_follows_calibre_id() {
        let p = fallback_palette(1);
        assert_eq!(p.bg, "#1f4fa8");
        assert_eq!(p.fg, "#f6f2e8");
        assert_eq!(p.accent, "#f2c230");
    }
}

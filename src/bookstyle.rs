use crate::domain::SpineStyle;

const FONTS: [&str; 5] = ["serif", "condensed", "mono", "display", "grotesk"];
const MARKS: [&str; 4] = ["dot", "bar", "initials", "ring"];
const GLYPH: [f64; 5] = [0.5, 0.42, 0.62, 0.4, 0.55];
const SHELF: f64 = 360.0;
const CHROME: f64 = 120.0;

/// FNV-1a mixed the way bookshelf `spine.ts` mixes it, over UTF-16 code units.
pub fn mix(s: &str) -> u32 {
    let mut h: u32 = 2_166_136_261;
    for unit in s.encode_utf16() {
        h ^= u32::from(unit);
        h = h.wrapping_mul(16_777_619);
    }
    h
}

pub fn spine_title(title: &str) -> String {
    let bytes = title.as_bytes();
    let mut end = title.len();
    let seps = [": ", "; ", " / ", ", or ", ", or,"];
    for sep in seps {
        if let Some(i) = title.find(sep) {
            end = end.min(i);
        }
    }
    let _ = bytes;
    let cut = title[..end].trim();
    if cut.is_empty() {
        title.to_string()
    } else {
        cut.to_string()
    }
}

pub fn spine_style(id: i64, title: &str, pages: Option<i32>) -> SpineStyle {
    let h = mix(&format!("{id}:{title}"));
    let width = if let Some(pages) = pages.filter(|n| *n > 0) {
        let raw = 20.0 + (pages as f64).sqrt() * 1.9;
        raw.round().clamp(30.0, 78.0) as i32
    } else {
        34 + (h % 30) as i32
    };
    let height = 0.86 + f64::from((h >> 8) % 15) / 100.0;
    let font_i = ((h >> 12) % 5) as usize;
    let font = FONTS[font_i];
    let shown = spine_title(title);
    let size_for = |lines: f64| {
        let glyph = GLYPH[font_i];
        let len = shown.chars().count().max(1) as f64;
        (width as f64 * if lines == 1.0 { 0.62 } else { 0.36 })
            .min(((height * SHELF - CHROME) * lines) / (len * glyph))
            .min(40.0)
    };
    let one = size_for(1.0);
    let two = size_for(2.0);
    let lines: u8 = if one < 15.0 && two > one { 2 } else { 1 };
    let title_size = size_for(f64::from(lines)).max(10.0).round() as i32;
    let caps = font == "display" || font == "condensed" || (h >> 16).is_multiple_of(3);
    SpineStyle {
        width,
        height,
        font: font.to_string(),
        caps,
        title_size,
        lines,
        mark: MARKS[((h >> 20) % 4) as usize].to_string(),
        title: shown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persuasion_matches_the_bookshelf_hash() {
        let s = spine_style(1, "Persuasion", Some(187));
        assert_eq!(s.width, 46);
        assert_eq!((s.height * 100.0).round() as i32, 86);
        assert_eq!(s.font, "grotesk");
        assert!(!s.caps);
        assert_eq!(s.title_size, 29);
        assert_eq!(s.lines, 1);
        assert_eq!(s.mark, "bar");
        assert_eq!(s.title, "Persuasion");
    }

    #[test]
    fn a_title_without_pages_uses_the_hash_width() {
        let s = spine_style(7, "A Tale of Two Cities", None);
        assert_eq!(s.width, 37);
        assert_eq!((s.height * 100.0).round() as i32, 95);
        assert_eq!(s.font, "grotesk");
        assert!(s.caps);
        assert_eq!(s.title_size, 20);
        assert_eq!(s.lines, 1);
        assert_eq!(s.mark, "ring");
        assert_eq!(s.title, "A Tale of Two Cities");
    }

    #[test]
    fn subtitle_after_a_colon_is_cut() {
        assert_eq!(
            spine_title("Frankenstein: The Modern Prometheus"),
            "Frankenstein"
        );
    }
}

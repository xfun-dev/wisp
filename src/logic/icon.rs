//! The tray icon (RULES.md U10, B1): the 一缕 glyph, tinted by the connection
//! state. scripts/brand.swift renders its alpha masks from the logo's path data
//! into assets/tray-glyph.bin: for each size, one byte of side length, then the
//! side² coverage bytes.

const MASKS: &[u8] = include_bytes!("../assets/tray-glyph.bin");

/// Every mask in the file, as (side, coverage bytes).
fn masks() -> impl Iterator<Item = (u32, &'static [u8])> {
    let mut rest = MASKS;
    std::iter::from_fn(move || {
        let (&side, tail) = rest.split_first()?;
        let len = side as usize * side as usize;
        let (mask, next) = tail.split_at(len.min(tail.len()));
        rest = next;
        Some((side as u32, mask))
    })
}

/// The mask for a tray icon of `size` px: an exact match, else the smallest larger
/// one (Windows scales it down), else the largest there is.
pub fn mask(size: u32) -> (u32, &'static [u8]) {
    masks()
        .filter(|(side, _)| *side >= size)
        .min_by_key(|(side, _)| *side)
        .or_else(|| masks().max_by_key(|(side, _)| *side))
        .expect("tray-glyph.bin holds at least one mask")
}

/// Premultiplied BGRA pixels (0xAARRGGBB as little-endian u32), top row first, of
/// the glyph in `rgb`.
pub fn tinted(mask: &[u8], (r, g, b): (u8, u8, u8)) -> Vec<u32> {
    mask.iter()
        .map(|&a| {
            let ch = |v: u8| (v as u32 * a as u32 + 127) / 255;
            (a as u32) << 24 | ch(r) << 16 | ch(g) << 8 | ch(b)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_glyph_comes_in_every_tray_size_and_tints_premultiplied() {
        let sides: Vec<u32> = masks().map(|(s, _)| s).collect();
        assert_eq!(sides, [16, 20, 24, 32, 40, 48], "the sizes 100%-300% display scales ask for");
        for (side, m) in masks() {
            assert_eq!(m.len(), (side * side) as usize, "{side}px mask is whole");
            assert!(m.iter().filter(|&&a| a > 200).count() > (side * side / 10) as usize, "{side}px: the W is there");
            assert_eq!(m[0], 0, "{side}px: corner is transparent");
        }
        assert_eq!(mask(20).0, 20);
        assert_eq!(mask(18).0, 20, "no 18: the next larger");
        assert_eq!(mask(64).0, 48, "nothing larger: the largest");

        let px = tinted(&[0, 128, 255], (0x2F, 0xD5, 0xBE));
        assert_eq!(px[0], 0);
        assert_eq!(px[2], 0xFF2F_D5BE);
        let (a, r) = (px[1] >> 24, px[1] >> 16 & 0xFF);
        assert!(a == 128 && r <= a, "premultiplied");
    }
}

use noto_sans_mono_bitmap::{
    FontWeight, RasterHeight, RasterizedChar, get_raster, get_raster_width,
};

pub const FONT_WEIGHT: FontWeight = FontWeight::Regular;
pub const FONT_HEIGHT: RasterHeight = RasterHeight::Size16;
pub const FONT_WIDTH: usize = get_raster_width(FONT_WEIGHT, FONT_HEIGHT);
pub const FONT_HEIGHT_PIXELS: usize = FONT_HEIGHT.val();
const FALLBACK_CHAR: char = '?';

/// Return the rasterized glyph for an ASCII character, falling back to `?`.
#[must_use]
pub fn glyph(character: char) -> RasterizedChar {
    get_raster(character, FONT_WEIGHT, FONT_HEIGHT)
        .or_else(|| get_raster(FALLBACK_CHAR, FONT_WEIGHT, FONT_HEIGHT))
        .expect("fallback glyph must exist in bundled font")
}

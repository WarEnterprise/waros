use noto_sans_mono_bitmap::{get_raster, get_raster_width, FontWeight, RasterHeight};

pub const FONT_WEIGHT: FontWeight = FontWeight::Regular;
pub const FONT_HEIGHT: RasterHeight = RasterHeight::Size16;
pub const FONT_WIDTH: usize = get_raster_width(FONT_WEIGHT, FONT_HEIGHT);
pub const FONT_HEIGHT_PIXELS: usize = FONT_HEIGHT.val();
const FALLBACK_CHAR: char = '?';

/// A fully embedded glyph raster used by the framebuffer console.
#[derive(Clone, Copy)]
pub struct Glyph {
    raster: [[u8; FONT_WIDTH]; FONT_HEIGHT_PIXELS],
}

impl Glyph {
    /// Return the glyph's per-pixel alpha mask.
    #[must_use]
    pub const fn raster(&self) -> &[[u8; FONT_WIDTH]; FONT_HEIGHT_PIXELS] {
        &self.raster
    }
}

/// Return the rasterized glyph for an ASCII character, falling back to a built-in placeholder.
#[must_use]
pub fn glyph(character: char) -> Glyph {
    get_raster(character, FONT_WEIGHT, FONT_HEIGHT)
        .or_else(|| get_raster(FALLBACK_CHAR, FONT_WEIGHT, FONT_HEIGHT))
        .map_or_else(fallback_glyph, Glyph::from_rasterized)
}

impl Glyph {
    fn from_rasterized(rasterized: noto_sans_mono_bitmap::RasterizedChar) -> Self {
        let mut raster = [[0u8; FONT_WIDTH]; FONT_HEIGHT_PIXELS];

        for (row_index, row) in rasterized
            .raster()
            .iter()
            .enumerate()
            .take(FONT_HEIGHT_PIXELS)
        {
            for (column_index, value) in row.iter().copied().enumerate().take(FONT_WIDTH) {
                raster[row_index][column_index] = value;
            }
        }

        Self { raster }
    }
}

fn fallback_glyph() -> Glyph {
    let mut raster = [[0u8; FONT_WIDTH]; FONT_HEIGHT_PIXELS];

    for (row_index, row) in raster.iter_mut().enumerate() {
        for (column_index, pixel) in row.iter_mut().enumerate() {
            let border = row_index == 0
                || row_index + 1 == FONT_HEIGHT_PIXELS
                || column_index == 0
                || column_index + 1 == FONT_WIDTH;
            *pixel = if border { u8::MAX } else { 0 };
        }
    }

    Glyph { raster }
}

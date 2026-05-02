use ab_glyph::{Font, FontArc, Glyph, GlyphId, OutlinedGlyph, PxScale, ScaleFont, point};
use cc_w_types::{
    SceneTextHorizontalAlign, SceneTextLabel, SceneTextStyle, SceneTextVerticalAlign,
};
use std::collections::HashMap;
use std::fmt;

const GLYPH_SCALE_QUANTIZATION: f32 = 64.0;

#[derive(Debug)]
pub enum TextError {
    InvalidFont,
    AtlasFull {
        width: u32,
        height: u32,
        requested_width: u32,
        requested_height: u32,
    },
}

impl fmt::Display for TextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFont => f.write_str("invalid font bytes"),
            Self::AtlasFull {
                width,
                height,
                requested_width,
                requested_height,
            } => write!(
                f,
                "text atlas {width}x{height} cannot fit glyph {requested_width}x{requested_height}",
            ),
        }
    }
}

impl std::error::Error for TextError {}

pub type TextResult<T> = Result<T, TextError>;

#[derive(Clone)]
pub struct TextFont {
    font: FontArc,
}

impl TextFont {
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> TextResult<Self> {
        let font = FontArc::try_from_vec(bytes.into()).map_err(|_| TextError::InvalidFont)?;
        Ok(Self { font })
    }

    pub fn font(&self) -> &FontArc {
        &self.font
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GlyphRasterKey {
    pub glyph_id: u16,
    pub size_px_64: u32,
}

impl GlyphRasterKey {
    pub fn new(glyph_id: GlyphId, size_px: f32) -> Self {
        Self {
            glyph_id: glyph_id.0,
            size_px_64: quantize_size_px(size_px),
        }
    }

    pub fn size_px(self) -> f32 {
        self.size_px_64 as f32 / GLYPH_SCALE_QUANTIZATION
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PixelRect {
    pub min_x: f32,
    pub min_y: f32,
    pub max_x: f32,
    pub max_y: f32,
}

impl PixelRect {
    pub fn width(self) -> f32 {
        self.max_x - self.min_x
    }

    pub fn height(self) -> f32 {
        self.max_y - self.min_y
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AtlasRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl AtlasRect {
    pub fn uv_rect(self, atlas_width: u32, atlas_height: u32) -> UvRect {
        let inv_w = 1.0 / atlas_width.max(1) as f32;
        let inv_h = 1.0 / atlas_height.max(1) as f32;
        UvRect {
            min_u: self.x as f32 * inv_w,
            min_v: self.y as f32 * inv_h,
            max_u: (self.x + self.width) as f32 * inv_w,
            max_v: (self.y + self.height) as f32 * inv_h,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UvRect {
    pub min_u: f32,
    pub min_v: f32,
    pub max_u: f32,
    pub max_v: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AtlasGlyph {
    pub key: GlyphRasterKey,
    pub atlas_rect: AtlasRect,
    pub uv_rect: UvRect,
    pub plane_rect: PixelRect,
    pub advance_px: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LaidOutGlyph {
    pub key: GlyphRasterKey,
    pub glyph_id: u16,
    pub quad: PixelRect,
    pub uv_rect: UvRect,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextLayout {
    pub glyphs: Vec<LaidOutGlyph>,
    pub bounds: PixelRect,
    pub text_bounds: PixelRect,
}

#[derive(Clone, Debug)]
pub struct TextAtlas {
    width: u32,
    height: u32,
    sdf_radius_px: u32,
    mode: TextAtlasMode,
    pixels: Vec<u8>,
    glyphs: HashMap<GlyphRasterKey, AtlasGlyph>,
    cursor_x: u32,
    cursor_y: u32,
    row_height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextAtlasMode {
    Sdf,
    AlphaMask,
}

impl TextAtlas {
    pub fn new(width: u32, height: u32, sdf_radius_px: u32) -> Self {
        Self::with_mode(width, height, sdf_radius_px, TextAtlasMode::Sdf)
    }

    pub fn new_alpha_mask(width: u32, height: u32, padding_px: u32) -> Self {
        Self::with_mode(width, height, padding_px, TextAtlasMode::AlphaMask)
    }

    fn with_mode(width: u32, height: u32, sdf_radius_px: u32, mode: TextAtlasMode) -> Self {
        let len = width as usize * height as usize;
        Self {
            width,
            height,
            sdf_radius_px,
            mode,
            pixels: vec![0; len],
            glyphs: HashMap::new(),
            cursor_x: 0,
            cursor_y: 0,
            row_height: 0,
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn sdf_radius_px(&self) -> u32 {
        self.sdf_radius_px
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn glyphs(&self) -> &HashMap<GlyphRasterKey, AtlasGlyph> {
        &self.glyphs
    }

    pub fn glyph(&self, key: GlyphRasterKey) -> Option<&AtlasGlyph> {
        self.glyphs.get(&key)
    }

    pub fn ensure_label_glyphs(
        &mut self,
        font: &TextFont,
        label: &SceneTextLabel,
    ) -> TextResult<()> {
        self.ensure_text_glyphs(font, &label.text, &label.style)
    }

    pub fn ensure_text_glyphs(
        &mut self,
        font: &TextFont,
        text: &str,
        style: &SceneTextStyle,
    ) -> TextResult<()> {
        let scale = PxScale::from(style.size_px);
        let scaled = font.font.as_scaled(scale);
        for ch in text.chars().filter(|ch| !ch.is_control()) {
            let glyph_id = font.font.glyph_id(ch);
            let key = GlyphRasterKey::new(glyph_id, style.size_px);
            if !self.glyphs.contains_key(&key) {
                let advance_px = scaled.h_advance(glyph_id);
                let glyph = Glyph {
                    id: glyph_id,
                    scale,
                    position: point(0.0, 0.0),
                };
                self.insert_rasterized_glyph(key, advance_px, scaled.outline_glyph(glyph))?;
            }
        }
        Ok(())
    }

    fn insert_rasterized_glyph(
        &mut self,
        key: GlyphRasterKey,
        advance_px: f32,
        outline: Option<OutlinedGlyph>,
    ) -> TextResult<()> {
        let radius = self.sdf_radius_px;
        let padding = radius + 1;
        let (bitmap, bitmap_width, bitmap_height, plane_rect) = match outline {
            Some(outline) => {
                let bounds = outline.px_bounds();
                let width = bounds.width().ceil().max(0.0) as u32 + padding * 2;
                let height = bounds.height().ceil().max(0.0) as u32 + padding * 2;
                let mut bitmap = vec![0_u8; width as usize * height as usize];
                outline.draw(|x, y, coverage| {
                    let px = x + padding;
                    let py = y + padding;
                    if px < width && py < height {
                        bitmap[(py * width + px) as usize] = (coverage * 255.0).round() as u8;
                    }
                });
                (
                    bitmap,
                    width,
                    height,
                    PixelRect {
                        min_x: bounds.min.x - padding as f32,
                        min_y: bounds.min.y - padding as f32,
                        max_x: bounds.max.x + padding as f32,
                        max_y: bounds.max.y + padding as f32,
                    },
                )
            }
            None => {
                let width = advance_px.ceil().max(1.0) as u32;
                let height = 1;
                (
                    vec![0_u8; width as usize],
                    width,
                    height,
                    PixelRect {
                        min_x: 0.0,
                        min_y: 0.0,
                        max_x: width as f32,
                        max_y: 0.0,
                    },
                )
            }
        };
        let storage_bitmap = match self.mode {
            TextAtlasMode::Sdf => {
                build_sdf_from_alpha_mask(&bitmap, bitmap_width, bitmap_height, radius)
            }
            TextAtlasMode::AlphaMask => bitmap,
        };
        self.insert_glyph_bitmap(
            key,
            advance_px,
            plane_rect,
            bitmap_width,
            bitmap_height,
            &storage_bitmap,
        )
    }

    fn insert_glyph_bitmap(
        &mut self,
        key: GlyphRasterKey,
        advance_px: f32,
        plane_rect: PixelRect,
        width: u32,
        height: u32,
        bitmap: &[u8],
    ) -> TextResult<()> {
        let atlas_rect = self.allocate(width, height)?;
        for row in 0..height {
            let dst = ((atlas_rect.y + row) * self.width + atlas_rect.x) as usize;
            let src = (row * width) as usize;
            self.pixels[dst..dst + width as usize]
                .copy_from_slice(&bitmap[src..src + width as usize]);
        }
        let atlas_glyph = AtlasGlyph {
            key,
            atlas_rect,
            uv_rect: atlas_rect.uv_rect(self.width, self.height),
            plane_rect,
            advance_px,
        };
        self.glyphs.insert(key, atlas_glyph);
        Ok(())
    }

    fn allocate(&mut self, width: u32, height: u32) -> TextResult<AtlasRect> {
        if width > self.width || height > self.height {
            return Err(TextError::AtlasFull {
                width: self.width,
                height: self.height,
                requested_width: width,
                requested_height: height,
            });
        }
        if self.cursor_x + width > self.width {
            self.cursor_x = 0;
            self.cursor_y += self.row_height;
            self.row_height = 0;
        }
        if self.cursor_y + height > self.height {
            return Err(TextError::AtlasFull {
                width: self.width,
                height: self.height,
                requested_width: width,
                requested_height: height,
            });
        }
        let rect = AtlasRect {
            x: self.cursor_x,
            y: self.cursor_y,
            width,
            height,
        };
        self.cursor_x += width;
        self.row_height = self.row_height.max(height);
        Ok(rect)
    }
}

pub fn layout_label(
    font: &TextFont,
    atlas: &mut TextAtlas,
    label: &SceneTextLabel,
) -> TextResult<TextLayout> {
    atlas.ensure_label_glyphs(font, label)?;
    layout_text(
        font,
        atlas,
        &label.text,
        &label.style,
        label.horizontal_align,
        label.vertical_align,
        label.screen_offset_px.x as f32,
        label.screen_offset_px.y as f32,
    )
}

pub fn layout_text(
    font: &TextFont,
    atlas: &TextAtlas,
    text: &str,
    style: &SceneTextStyle,
    horizontal_align: SceneTextHorizontalAlign,
    vertical_align: SceneTextVerticalAlign,
    offset_x_px: f32,
    offset_y_px: f32,
) -> TextResult<TextLayout> {
    let scale = PxScale::from(style.size_px);
    let scaled = font.font.as_scaled(scale);
    let metrics = LayoutMetrics {
        width: measure_text(font, text, style.size_px),
        ascent: scaled.ascent(),
        descent: scaled.descent(),
    };
    let alignment = layout_alignment(metrics, horizontal_align, vertical_align);
    let mut glyphs = Vec::new();
    let mut caret_x = 0.0;
    let mut previous = None;

    for ch in text.chars().filter(|ch| !ch.is_control()) {
        let glyph_id = font.font.glyph_id(ch);
        if let Some(previous) = previous {
            caret_x += scaled.kern(previous, glyph_id);
        }
        let key = GlyphRasterKey::new(glyph_id, style.size_px);
        if let Some(atlas_glyph) = atlas.glyph(key) {
            let origin_x = caret_x + alignment.x + offset_x_px;
            let origin_y = alignment.y + offset_y_px;
            glyphs.push(LaidOutGlyph {
                key,
                glyph_id: glyph_id.0,
                quad: PixelRect {
                    min_x: origin_x + atlas_glyph.plane_rect.min_x,
                    min_y: origin_y + atlas_glyph.plane_rect.min_y,
                    max_x: origin_x + atlas_glyph.plane_rect.max_x,
                    max_y: origin_y + atlas_glyph.plane_rect.max_y,
                },
                uv_rect: atlas_glyph.uv_rect,
            });
        }
        caret_x += scaled.h_advance(glyph_id);
        previous = Some(glyph_id);
    }

    let text_bounds = PixelRect {
        min_x: alignment.x + offset_x_px,
        min_y: -metrics.ascent + alignment.y + offset_y_px,
        max_x: metrics.width + alignment.x + offset_x_px,
        max_y: -metrics.descent + alignment.y + offset_y_px,
    };
    let bounds = glyphs
        .iter()
        .fold(text_bounds, |bounds, glyph| union_rect(bounds, glyph.quad));
    Ok(TextLayout {
        glyphs,
        bounds,
        text_bounds,
    })
}

pub fn measure_text(font: &TextFont, text: &str, size_px: f32) -> f32 {
    let scaled = font.font.as_scaled(PxScale::from(size_px));
    let mut width = 0.0;
    let mut previous = None;
    for ch in text.chars().filter(|ch| !ch.is_control()) {
        let glyph_id = font.font.glyph_id(ch);
        if let Some(previous) = previous {
            width += scaled.kern(previous, glyph_id);
        }
        width += scaled.h_advance(glyph_id);
        previous = Some(glyph_id);
    }
    width
}

pub fn build_sdf_from_alpha_mask(alpha: &[u8], width: u32, height: u32, radius_px: u32) -> Vec<u8> {
    let len = width as usize * height as usize;
    if width == 0 || height == 0 || alpha.len() < len {
        return Vec::new();
    }
    let radius = radius_px.max(1) as f32;
    let mut sdf = vec![0_u8; len];

    for y in 0..height {
        for x in 0..width {
            let idx = (y * width + x) as usize;
            let inside = alpha[idx] >= 128;
            let nearest = nearest_opposite_distance(alpha, width, height, x, y, inside);
            let signed = if inside { nearest } else { -nearest };
            let normalized = (0.5 + signed / radius * 0.5).clamp(0.0, 1.0);
            sdf[idx] = (normalized * 255.0).round() as u8;
        }
    }

    sdf
}

fn nearest_opposite_distance(
    alpha: &[u8],
    width: u32,
    height: u32,
    x: u32,
    y: u32,
    inside: bool,
) -> f32 {
    let mut nearest_sq = f32::INFINITY;
    for other_y in 0..height {
        for other_x in 0..width {
            let idx = (other_y * width + other_x) as usize;
            if (alpha[idx] >= 128) != inside {
                let dx = other_x as f32 - x as f32;
                let dy = other_y as f32 - y as f32;
                nearest_sq = nearest_sq.min(dx * dx + dy * dy);
            }
        }
    }
    if nearest_sq.is_finite() {
        nearest_sq.sqrt()
    } else {
        width.max(height) as f32
    }
}

#[derive(Clone, Copy, Debug)]
struct LayoutMetrics {
    width: f32,
    ascent: f32,
    descent: f32,
}

#[derive(Clone, Copy, Debug)]
struct LayoutAlignment {
    x: f32,
    y: f32,
}

fn layout_alignment(
    metrics: LayoutMetrics,
    horizontal_align: SceneTextHorizontalAlign,
    vertical_align: SceneTextVerticalAlign,
) -> LayoutAlignment {
    let x = match horizontal_align {
        SceneTextHorizontalAlign::Left => 0.0,
        SceneTextHorizontalAlign::Center => -metrics.width * 0.5,
        SceneTextHorizontalAlign::Right => -metrics.width,
    };
    let y = match vertical_align {
        SceneTextVerticalAlign::Baseline => 0.0,
        SceneTextVerticalAlign::Top => metrics.ascent,
        SceneTextVerticalAlign::Middle => (metrics.ascent + metrics.descent) * 0.5,
        SceneTextVerticalAlign::Bottom => metrics.descent,
    };
    LayoutAlignment { x, y }
}

fn quantize_size_px(size_px: f32) -> u32 {
    (size_px.max(0.0) * GLYPH_SCALE_QUANTIZATION).round() as u32
}

fn union_rect(a: PixelRect, b: PixelRect) -> PixelRect {
    PixelRect {
        min_x: a.min_x.min(b.min_x),
        min_y: a.min_y.min(b.min_y),
        max_x: a.max_x.max(b.max_x),
        max_y: a.max_y.max(b.max_y),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::hash_map::Entry;

    impl TextAtlas {
        fn insert_test_bitmap(
            &mut self,
            key: GlyphRasterKey,
            width: u32,
            height: u32,
        ) -> TextResult<AtlasRect> {
            let bitmap = vec![255_u8; width as usize * height as usize];
            match self.glyphs.entry(key) {
                Entry::Vacant(_) => {
                    self.insert_glyph_bitmap(
                        key,
                        width as f32,
                        PixelRect {
                            min_x: 0.0,
                            min_y: 0.0,
                            max_x: width as f32,
                            max_y: height as f32,
                        },
                        width,
                        height,
                        &bitmap,
                    )?;
                    Ok(self.glyphs[&key].atlas_rect)
                }
                Entry::Occupied(entry) => Ok(entry.get().atlas_rect),
            }
        }
    }

    #[test]
    fn alignment_places_bounds_relative_to_anchor() {
        let metrics = LayoutMetrics {
            width: 40.0,
            ascent: 12.0,
            descent: -4.0,
        };

        let center_middle = layout_alignment(
            metrics,
            SceneTextHorizontalAlign::Center,
            SceneTextVerticalAlign::Middle,
        );
        assert_eq!(center_middle.x, -20.0);
        assert_eq!(center_middle.y, 4.0);

        let right_bottom = layout_alignment(
            metrics,
            SceneTextHorizontalAlign::Right,
            SceneTextVerticalAlign::Bottom,
        );
        assert_eq!(right_bottom.x, -40.0);
        assert_eq!(right_bottom.y, -4.0);

        let left_top = layout_alignment(
            metrics,
            SceneTextHorizontalAlign::Left,
            SceneTextVerticalAlign::Top,
        );
        assert_eq!(left_top.x, 0.0);
        assert_eq!(left_top.y, 12.0);
    }

    #[test]
    fn atlas_packing_wraps_rows_without_overlap() {
        let mut atlas = TextAtlas::new(8, 8, 2);
        let first = atlas
            .insert_test_bitmap(
                GlyphRasterKey {
                    glyph_id: 1,
                    size_px_64: 12 * 64,
                },
                5,
                3,
            )
            .unwrap();
        let second = atlas
            .insert_test_bitmap(
                GlyphRasterKey {
                    glyph_id: 2,
                    size_px_64: 12 * 64,
                },
                3,
                4,
            )
            .unwrap();
        let third = atlas
            .insert_test_bitmap(
                GlyphRasterKey {
                    glyph_id: 3,
                    size_px_64: 12 * 64,
                },
                4,
                2,
            )
            .unwrap();

        assert_eq!(
            first,
            AtlasRect {
                x: 0,
                y: 0,
                width: 5,
                height: 3
            }
        );
        assert_eq!(
            second,
            AtlasRect {
                x: 5,
                y: 0,
                width: 3,
                height: 4
            }
        );
        assert_eq!(
            third,
            AtlasRect {
                x: 0,
                y: 4,
                width: 4,
                height: 2
            }
        );
    }

    #[test]
    fn sdf_output_is_non_empty_and_finite() {
        let alpha = [
            0, 0, 0, 0, 0, //
            0, 255, 255, 255, 0, //
            0, 255, 255, 255, 0, //
            0, 255, 255, 255, 0, //
            0, 0, 0, 0, 0,
        ];
        let sdf = build_sdf_from_alpha_mask(&alpha, 5, 5, 4);
        assert_eq!(sdf.len(), 25);
        assert!(sdf.iter().any(|value| *value > 128));
        assert!(sdf.iter().any(|value| *value < 128));
        assert!(sdf.iter().all(|value| f32::from(*value).is_finite()));
    }
}

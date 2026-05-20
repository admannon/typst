use std::ops::Range;
use std::sync::Arc;

use bytemuck::TransparentWrapper;
use krilla::surface::{Location, Surface};
use krilla::text::GlyphId;
use typst_library::diag::{SourceResult, bail};
use typst_library::text::{Font, FontFlags, FontStyle, Glyph, TextItem};
use typst_library::visualize::FillRule;
use typst_syntax::Span;
use typst_utils::defer;

use crate::convert::{FrameContext, GlobalContext};
use crate::util::{AbsExt, TransformExt, display_font};
use crate::{paint, tags};

#[typst_macros::time(name = "handle text")]
pub(crate) fn handle_text(
    fc: &mut FrameContext,
    t: &TextItem,
    surface: &mut Surface,
    gc: &mut GlobalContext,
) -> SourceResult<()> {
    let mut handle = tags::text(gc, fc, surface, t);
    let surface = handle.surface();

    let font = convert_font(gc, t.font.clone())?;
    let fill = paint::convert_fill(
        gc,
        &t.fill,
        FillRule::NonZero,
        true,
        surface,
        fc.state(),
        None,
    )?;
    let stroke = if let Some(stroke) = t.stroke.as_ref() {
        Some(paint::convert_stroke(gc, stroke, true, surface, fc.state(), None)?)
    } else {
        None
    };
    let text = t.text.as_str();
    let size = t.size;
    let glyphs: &[PdfGlyph] = TransparentWrapper::wrap_slice(t.glyphs.as_slice());

    surface.push_transform(&fc.state().transform().to_krilla());
    let mut surface = defer(surface, |s| s.pop());
    surface.set_fill(Some(fill));
    surface.set_stroke(stroke);
    surface.draw_glyphs(
        krilla::geom::Point::from_xy(0.0, 0.0),
        glyphs,
        font.clone(),
        text,
        size.to_f32(),
        false,
    );

    Ok(())
}

fn convert_font(
    gc: &mut GlobalContext,
    typst_font: Font,
) -> SourceResult<krilla::text::Font> {
    if let Some(font) = gc.fonts_forward.get(&typst_font) {
        Ok(font.clone())
    } else {
        let font = build_font(typst_font.clone())?;

        gc.fonts_forward.insert(typst_font.clone(), font.clone());
        gc.fonts_backward.insert(font.clone(), typst_font.clone());

        Ok(font)
    }
}

#[comemo::memoize]
fn build_font(typst_font: Font) -> SourceResult<krilla::text::Font> {
    let font_data: Arc<dyn AsRef<[u8]> + Send + Sync> =
        Arc::new(typst_font.data().clone());

    let font = if typst_font.info().flags.contains(FontFlags::VARIABLE) {
        let coords = variation_coords(&typst_font);
        krilla::text::Font::new_variable(font_data.into(), typst_font.index(), &coords)
    } else {
        krilla::text::Font::new(font_data.into(), typst_font.index())
    };

    match font {
        Some(f) => Ok(f),
        None => {
            bail!(
                Span::detached(),
                "failed to process {}",
                display_font(Some(&typst_font)),
            )
        }
    }
}

fn variation_coords(typst_font: &Font) -> Vec<(krilla::text::Tag, f32)> {
    let variant = typst_font.info().variant;
    let mut coords = vec![
        (krilla::text::Tag::new(b"wght"), variant.weight.to_number() as f32),
        (
            krilla::text::Tag::new(b"wdth"),
            (variant.stretch.to_ratio().get() * 100.0) as f32,
        ),
    ];

    let has_ital = typst_font
        .ttf()
        .variation_axes()
        .into_iter()
        .any(|axis| axis.tag.to_bytes() == *b"ital");
    let has_slnt = typst_font
        .ttf()
        .variation_axes()
        .into_iter()
        .any(|axis| axis.tag.to_bytes() == *b"slnt");

    coords.extend(style_coords(variant.style, has_ital, has_slnt));
    coords
}

fn style_coords(
    style: FontStyle,
    has_ital: bool,
    has_slnt: bool,
) -> Option<(krilla::text::Tag, f32)> {
    match style {
        FontStyle::Normal => None,
        FontStyle::Italic if has_ital => Some((krilla::text::Tag::new(b"ital"), 1.0)),
        FontStyle::Italic if has_slnt => Some((krilla::text::Tag::new(b"slnt"), -10.0)),
        FontStyle::Oblique if has_slnt => Some((krilla::text::Tag::new(b"slnt"), -10.0)),
        _ => None,
    }
}

#[derive(Debug, TransparentWrapper)]
#[repr(transparent)]
struct PdfGlyph(Glyph);

impl krilla::text::Glyph for PdfGlyph {
    #[inline(always)]
    fn glyph_id(&self) -> GlyphId {
        GlyphId::new(self.0.id as u32)
    }

    #[inline(always)]
    fn text_range(&self) -> Range<usize> {
        self.0.range.start as usize..self.0.range.end as usize
    }

    #[inline(always)]
    fn x_advance(&self, size: f32) -> f32 {
        // Don't use `Em::at`, because it contains an expensive check whether the result is finite.
        self.0.x_advance.get() as f32 * size
    }

    #[inline(always)]
    fn x_offset(&self, size: f32) -> f32 {
        // Don't use `Em::at`, because it contains an expensive check whether the result is finite.
        self.0.x_offset.get() as f32 * size
    }

    #[inline(always)]
    fn y_offset(&self, size: f32) -> f32 {
        // Don't use `Em::at`, because it contains an expensive check whether the result is finite.
        self.0.y_offset.get() as f32 * size
    }

    #[inline(always)]
    fn y_advance(&self, size: f32) -> f32 {
        // Don't use `Em::at`, because it contains an expensive check whether the result is finite.
        self.0.y_advance.get() as f32 * size
    }

    fn location(&self) -> Option<Location> {
        Some(self.0.span.0.into_raw())
    }
}

#[cfg(test)]
mod tests {
    use super::style_coords;
    use typst_library::text::FontStyle;

    #[test]
    fn italic_prefers_ital_axis() {
        let coords = style_coords(FontStyle::Italic, true, true);
        assert_eq!(coords, Some((krilla::text::Tag::new(b"ital"), 1.0)));
    }

    #[test]
    fn italic_falls_back_to_slnt_axis() {
        let coords = style_coords(FontStyle::Italic, false, true);
        assert_eq!(coords, Some((krilla::text::Tag::new(b"slnt"), -10.0)));
    }

    #[test]
    fn oblique_uses_slnt_axis() {
        let coords = style_coords(FontStyle::Oblique, false, true);
        assert_eq!(coords, Some((krilla::text::Tag::new(b"slnt"), -10.0)));
    }

    #[test]
    fn no_axis_for_unsupported_style() {
        let coords = style_coords(FontStyle::Italic, false, false);
        assert_eq!(coords, None);
    }
}

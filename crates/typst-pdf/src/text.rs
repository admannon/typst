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
        let coords = variation_coords(typst_font.info().variant);
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

fn variation_coords(
    variant: typst_library::text::FontVariant,
) -> Vec<(krilla::text::Tag, f32)> {
    let mut coords = vec![
        (krilla::text::Tag::new(b"wght"), variant.weight.to_number() as f32),
        (
            krilla::text::Tag::new(b"wdth"),
            (variant.stretch.to_ratio().get() * 100.0) as f32,
        ),
    ];

    if variant.style == FontStyle::Italic {
        coords.push((krilla::text::Tag::new(b"ital"), 1.0));
    }

    coords
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
    use super::variation_coords;
    use typst_library::layout::Ratio;
    use typst_library::text::{FontStretch, FontStyle, FontVariant, FontWeight};

    #[test]
    fn includes_weight_and_width_axes() {
        let coords = variation_coords(FontVariant {
            style: FontStyle::Normal,
            weight: FontWeight::from_number(630),
            stretch: FontStretch::from_ratio(Ratio::new(0.75)),
        });

        assert_eq!(
            coords,
            vec![
                (krilla::text::Tag::new(b"wght"), 630.0),
                (krilla::text::Tag::new(b"wdth"), 75.0),
            ]
        );
    }

    #[test]
    fn includes_ital_axis_for_italic() {
        let coords = variation_coords(FontVariant::new(
            FontStyle::Italic,
            FontWeight::REGULAR,
            FontStretch::NORMAL,
        ));

        assert!(coords.contains(&(krilla::text::Tag::new(b"ital"), 1.0)));
    }
}

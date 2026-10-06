// SPDX-License-Identifier: AGPL-3.0-or-later
//! Burns a licence credit into an artist photo.
//!
//! **This is what makes serving Wikimedia photos to third-party clients
//! lawful.** Commons images are free to reuse commercially, but most carry a
//! CC BY or CC BY-SA licence requiring attribution wherever the work appears.
//! The Subsonic schema has no attribution field, and a client is free to
//! render the picture and nothing else — so a credit that lives beside the
//! image in some other field is a credit that may never be shown.
//!
//! Painting it into the pixels makes the attribution inseparable from the
//! work, which satisfies the licence in any client, present or future,
//! including ones that only ever fetch `getCoverArt`.
//!
//! Two consequences worth knowing:
//!
//! - The result is an *adaptation* under CC BY-SA, so callers must also record
//!   that the image was modified. `music::get_artist_image` does this in the
//!   attribution it stores and returns.
//! - Failure is fatal by design. [`burn`] returning `None` means the image
//!   must not be used at all — serving it uncredited is the exact outcome this
//!   module exists to prevent.

use ab_glyph::{FontRef, PxScale};
use anyhow::{Context, Result};
use image::{ExtendedColorType, ImageEncoder, Rgba, RgbaImage};
use imageproc::drawing::{draw_text_mut, text_size};

/// The same face the generated audiobook covers use, embedded for the same
/// reason: rendering must not depend on fonts installed on the host.
static FONT: &[u8] = include_bytes!("../../assets/fonts/PTSerif-Regular.ttf");

/// Builds the one-line credit a licence requires.
///
/// Returns `None` when there is nothing to assert. That is not the same as an
/// image being free to use uncredited — a Commons file whose metadata carries
/// neither an author nor a licence is one we cannot describe accurately, so
/// the caller drops it rather than inventing a credit for it.
pub fn credit_line(author: Option<&str>, license: Option<&str>) -> Option<String> {
    let author = author.map(str::trim).filter(|s| !s.is_empty());
    let license = license.map(str::trim).filter(|s| !s.is_empty());

    match (author, license) {
        (Some(author), Some(license)) => Some(format!("{author} · {license} · Wikimedia Commons")),
        (Some(author), None) => Some(format!("{author} · Wikimedia Commons")),
        (None, Some(license)) => Some(format!("{license} · Wikimedia Commons")),
        (None, None) => None,
    }
}

/// Draws `credit` along the bottom of the image and re-encodes it as JPEG.
///
/// Always JPEG out, whatever came in: these are photographs, the credit makes
/// the result a new file regardless, and one output format keeps the stored
/// content type honest.
pub fn burn(bytes: &[u8], credit: &str) -> Result<Vec<u8>> {
    let image = image::load_from_memory(bytes).context("decode artist image for watermarking")?;
    let mut canvas: RgbaImage = image.to_rgba8();
    let (width, height) = canvas.dimensions();

    let font = FontRef::try_from_slice(FONT).context("load watermark font")?;

    // Relative to width so the credit stays legible whatever Commons returned,
    // with a floor because a very small image would otherwise render it
    // unreadably — an unreadable credit is not a credit.
    let scale = PxScale::from((width as f32 * 0.030).max(13.0));
    let margin = (width as f32 * 0.02).max(6.0) as i32;

    let lines = wrap(credit, &font, scale, width as i32 - margin * 2);
    let line_height = scale.y as i32 + (scale.y * 0.2) as i32;
    let band_height = lines.len() as i32 * line_height + margin * 2;
    let band_top = (height as i32 - band_height).max(0);

    darken_band(&mut canvas, band_top, height as i32);

    let mut y = band_top + margin;
    for line in &lines {
        draw_text_mut(
            &mut canvas,
            Rgba([255u8, 255, 255, 255]),
            margin,
            y,
            scale,
            &font,
            line,
        );
        y += line_height;
    }

    // JPEG carries no alpha channel, so flatten before encoding; the alpha
    // only ever mattered while blending the band.
    let rgb = image::DynamicImage::ImageRgba8(canvas).to_rgb8();
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 88)
        .write_image(&rgb, width, height, ExtendedColorType::Rgb8)
        .context("encode watermarked artist image")?;
    Ok(out)
}

/// Alpha-blends a dark band behind the text rather than painting over it, so
/// the photograph still reads through the credit.
fn darken_band(canvas: &mut RgbaImage, top: i32, bottom: i32) {
    let span = (bottom - top).max(1) as f32;
    for y in top.max(0)..bottom {
        // Slightly stronger towards the bottom, so the first line of a
        // two-line credit does not sit on a flat grey slab.
        let t = (y - top) as f32 / span;
        let alpha = (0.45 + t * 0.25).min(0.75);
        for x in 0..canvas.width() {
            let px = canvas.get_pixel_mut(x, y as u32);
            for channel in 0..3 {
                px.0[channel] = (px.0[channel] as f32 * (1.0 - alpha)) as u8;
            }
        }
    }
}

/// Greedy word wrap on real glyph metrics — an author field on Commons can be
/// a whole institution's name and will not fit on one line.
fn wrap(text: &str, font: &FontRef, scale: PxScale, max_width: i32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();

    for word in text.split_whitespace() {
        let candidate = if current.is_empty() {
            word.to_string()
        } else {
            format!("{current} {word}")
        };
        let (w, _) = text_size(scale, font, &candidate);
        if w as i32 > max_width && !current.is_empty() {
            lines.push(std::mem::take(&mut current));
            current = word.to_string();
        } else {
            current = candidate;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }

    // A credit long enough to bury the photograph is truncated rather than
    // allowed to grow without limit.
    if lines.len() > 2 {
        lines.truncate(2);
        if let Some(last) = lines.last_mut() {
            last.push('…');
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_a_credit_from_author_and_licence() {
        assert_eq!(
            credit_line(Some("Koh Hasebe"), Some("CC BY-SA 4.0")).as_deref(),
            Some("Koh Hasebe · CC BY-SA 4.0 · Wikimedia Commons")
        );
    }

    #[test]
    fn still_credits_when_only_one_half_is_known() {
        assert_eq!(
            credit_line(None, Some("Public domain")).as_deref(),
            Some("Public domain · Wikimedia Commons")
        );
        assert_eq!(
            credit_line(Some("Jane Doe"), None).as_deref(),
            Some("Jane Doe · Wikimedia Commons")
        );
    }

    /// Nothing to assert means the image cannot be described accurately, and
    /// the caller must drop it rather than show it bare.
    #[test]
    fn refuses_to_invent_a_credit_from_nothing() {
        assert_eq!(credit_line(None, None), None);
        assert_eq!(credit_line(Some("  "), Some("")), None);
    }

    fn test_image(width: u32, height: u32) -> Vec<u8> {
        let canvas = RgbaImage::from_pixel(width, height, Rgba([120u8, 120, 120, 255]));
        let rgb = image::DynamicImage::ImageRgba8(canvas).to_rgb8();
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90)
            .write_image(&rgb, width, height, ExtendedColorType::Rgb8)
            .unwrap();
        out
    }

    #[test]
    fn burning_a_credit_keeps_the_image_dimensions() {
        let burned = burn(&test_image(640, 480), "Jane Doe · CC BY-SA 4.0 · Wikimedia Commons").unwrap();
        let decoded = image::load_from_memory(&burned).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (640, 480));
    }

    /// The band has to actually change pixels — a watermark that blends into
    /// the photograph credits nobody.
    #[test]
    fn the_credit_darkens_the_bottom_of_the_image() {
        let burned = burn(&test_image(640, 480), "Jane Doe · CC BY-SA 4.0").unwrap();
        let decoded = image::load_from_memory(&burned).unwrap().to_rgb8();
        let top = decoded.get_pixel(320, 10).0[0] as i32;
        let bottom = decoded.get_pixel(320, 470).0[0] as i32;
        assert!(bottom < top - 20, "bottom {bottom} should be darker than top {top}");
    }

    #[test]
    fn a_very_long_author_wraps_rather_than_running_off_the_edge() {
        let font = FontRef::try_from_slice(FONT).unwrap();
        let long = "The Board of Trustees of a Very Long Institutional Name Indeed · CC BY-SA 4.0 · Wikimedia Commons";
        let lines = wrap(long, &font, PxScale::from(19.0), 600);
        assert!(lines.len() > 1);
        assert!(lines.len() <= 2, "should cap at two lines, got {}", lines.len());
    }

    /// Bytes that are not an image at all must fail, not produce a blank
    /// credited rectangle.
    #[test]
    fn refuses_bytes_that_are_not_an_image() {
        assert!(burn(b"<html>404</html>", "Jane Doe").is_err());
    }
}

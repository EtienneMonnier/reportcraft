//! Pictures as egui textures for the canvas and the preview: decoded once, scaled down to at
//! most `MAX_SIDE_PX` a side, and dropped once no frame has drawn them for a while.

use std::collections::HashMap;

use egui::{Color32, ColorImage, Context, Id, Rect, TextureHandle, TextureOptions};
use reportcraft_core::image::{ImageData, Picture};

/// Longest side of a texture: enough for a page on screen, small enough for any GPU.
const MAX_SIDE_PX: u32 = 1024;
/// Frames a texture is kept without being drawn.
const KEEP_PASSES: u64 = 120;

#[derive(Clone, Default)]
struct Cache {
    /// By the address and length of the bytes; the entry holds the bytes, so the address can't
    /// be reused for other ones while it is cached.
    map: HashMap<(usize, usize), Entry>,
}

#[derive(Clone)]
struct Entry {
    _data: ImageData,
    texture: Option<TextureHandle>,
    used: u64,
}

fn cache_id() -> Id {
    Id::unique("reportcraft: picture textures")
}

/// The texture of a picture; `None` when it can't be decoded.
pub(crate) fn texture(ctx: &Context, data: &ImageData) -> Option<TextureHandle> {
    let pass = ctx.cumulative_pass_nr();
    let key = (data.0.as_ptr() as usize, data.0.len());
    let cached = ctx.data_mut(|d| {
        let cache = d.get_temp_mut_or_default::<Cache>(cache_id());
        cache.map.get_mut(&key).map(|e| {
            e.used = pass;
            e.texture.clone()
        })
    });
    if let Some(t) = cached {
        return t;
    }
    // Decoded and uploaded outside `data_mut`: loading a texture locks the context too.
    let texture = Picture::of(data)
        .ok()
        .and_then(|p| reportcraft_layout::pixels::decode(&p))
        .map(|px| ctx.load_texture(format!("picture {key:?}"), scaled(&px), TextureOptions::LINEAR));
    ctx.data_mut(|d| {
        let cache = d.get_temp_mut_or_default::<Cache>(cache_id());
        cache.map.retain(|_, e| pass.saturating_sub(e.used) <= KEEP_PASSES);
        cache.map.insert(key, Entry { _data: data.clone(), texture: texture.clone(), used: pass });
    });
    texture
}

/// Draws a picture stretched over `rect`; `false` when it can't be decoded.
pub(crate) fn paint(painter: &egui::Painter, rect: Rect, data: &ImageData) -> bool {
    let Some(t) = texture(painter.ctx(), data) else { return false };
    let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    painter.image(t.id(), rect, uv, Color32::WHITE);
    true
}

/// The pixels as an egui image, sampled down to `MAX_SIDE_PX` a side (nearest pixel).
fn scaled(px: &reportcraft_layout::pixels::Rgba) -> ColorImage {
    let longest = px.width.max(px.height).max(1);
    let (w, h) = if longest <= MAX_SIDE_PX {
        (px.width, px.height)
    } else {
        let side = |n: u32| u32::try_from(u64::from(n) * u64::from(MAX_SIDE_PX) / u64::from(longest)).unwrap_or(1).max(1);
        (side(px.width), side(px.height))
    };
    let mut out = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h {
        let sy = u64::from(y) * u64::from(px.height) / u64::from(h);
        for x in 0..w {
            let sx = u64::from(x) * u64::from(px.width) / u64::from(w);
            let at = usize::try_from((sy * u64::from(px.width) + sx) * 4).unwrap_or(usize::MAX);
            out.extend_from_slice(px.pixels.get(at..at.saturating_add(4)).unwrap_or(&[0, 0, 0, 0]));
        }
    }
    ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_pictures_are_sampled_down_keeping_their_shape() {
        let px = reportcraft_layout::pixels::Rgba { width: 3000, height: 1500, pixels: vec![7; 3000 * 1500 * 4] };
        let img = scaled(&px);
        assert_eq!(img.size, [1024, 512]);
        let small = reportcraft_layout::pixels::Rgba { width: 2, height: 1, pixels: vec![255, 0, 0, 255, 0, 0, 255, 255] };
        let img = scaled(&small);
        assert_eq!(img.size, [2, 1]);
        assert_eq!(img.pixels.first(), Some(&Color32::RED));
    }

    #[test]
    fn textures_are_made_once_and_damaged_pictures_have_none() {
        let ctx = Context::default();
        let png =
            reportcraft_layout::pixels::encode_png(&reportcraft_layout::pixels::Rgba { width: 1, height: 1, pixels: vec![1, 2, 3, 255] }).unwrap();
        let data = ImageData(png.clone().into());
        let a = texture(&ctx, &data).unwrap();
        let b = texture(&ctx, &data).unwrap();
        assert_eq!(a.id(), b.id());
        let cut = ImageData(png.get(..png.len() - 16).unwrap().into());
        assert!(texture(&ctx, &cut).is_none());
    }
}

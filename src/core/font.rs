use prelude::*;
use core::{layer, Layer, Point, Rect, rendercontext, RenderContext};
use Color;
use rusttype;
use glium;
use font_loader::system_fonts;

use std::borrow::Cow;

/// A struct used to filter the result of [`Font::query_specific()`](struct.Font.html#method.query_specific)
/// or to describe a [`Font`](struct.Font.html) to be created from a system font
/// via [`Font::from_info()`](struct.Font.html#method.from_info).
#[derive(Clone)]
pub struct FontInfo {
    pub italic      : bool,
    pub oblique     : bool,
    pub bold        : bool,
    pub monospace   : bool,
    pub family      : String,
    pub size        : f32,
}

impl Default for FontInfo {
    fn default() -> FontInfo {
        FontInfo {
            italic      : false,
            oblique     : false,
            bold        : false,
            monospace   : false,
            family      : "".to_string(),
            size        : 10.0,
        }
   }
}

pub struct FontCache {
    cache   : Mutex<rusttype::gpu_cache::Cache>,
    queue   : Mutex<Vec<(rusttype::Rect<u32>, Vec<u8>)>>,
    dirty   : AtomicBool,
}

impl FontCache {
    pub fn new(width: u32, height: u32, scale_tolerance: f32, position_tolerance: f32) -> FontCache {
        FontCache {
            cache: Mutex::new(rusttype::gpu_cache::Cache::new(width, height, scale_tolerance, position_tolerance)),
            queue: Mutex::new(Vec::new()),
            dirty: AtomicBool::new(false),
        }
    }

    pub fn queue(self: &Self, font_id: usize, glyphs: &[rusttype::PositionedGlyph]) {

        let mut cache = self.cache.lock().unwrap();
        let mut queue = self.queue.lock().unwrap();
        let mut dirties = false;

        for glyph in glyphs {
            cache.queue_glyph(font_id, glyph.clone());
        }

        cache.cache_queued(|rect, data| {
            queue.push((rect, data.to_vec()));
            dirties = true;
        }).unwrap();

        if dirties {
            self.dirty.store(dirties, Ordering::Relaxed);
        }
    }

    pub fn update(self: &Self, texture: &mut glium::texture::Texture2d) {

        if self.dirty.load(Ordering::Relaxed) {
            let mut queue = self.queue.lock().unwrap();
            for &(ref rect, ref data) in queue.deref() {
                texture.main_level().write(
                    glium::Rect {
                        left: rect.min.x,
                        bottom: rect.min.y,
                        width: rect.width(),
                        height: rect.height()
                    },
                    glium::texture::RawImage2d {
                        data: Cow::Borrowed(&data),
                        width: rect.width(),
                        height: rect.height(),
                        format: glium::texture::ClientFormat::U8
                    }
                );
            }
            queue.clear();
            self.dirty.store(false, Ordering::Relaxed);
        }
    }

    pub fn rect_for(self: &Self, font_id: usize, glyph: &rusttype::PositionedGlyph) -> Option<(Rect, Point, Point)> {
        let cache = self.cache.lock().unwrap();
        if let Ok(Some((uv_rect, screen_rect))) = cache.rect_for(font_id, glyph) {
            let uv = Rect::new(uv_rect.min.x, uv_rect.min.y, uv_rect.max.x, uv_rect.max.y);
            let pos = Point::new(screen_rect.min.x as f32, screen_rect.min.y as f32);
            let dim = Point::new((screen_rect.max.x - screen_rect.min.x) as f32, (screen_rect.max.y - screen_rect.min.y) as f32);
            Some((uv, pos, dim))
        } else {
            None
        }
    }
}

static FONT_COUNTER: AtomicUsize = ATOMIC_USIZE_INIT;

/// A font used for writing on a [`Layer`](struct.Layer.html).
///
/// Fonts can be created from files, registered system fonts or existing font objects.
/// When creating fonts from system fonts, a [`FontInfo`](struct.FontInfo.html) structure can be
/// used to define requirements for the font, e.g. "any available monospace font".
///
/// In addition to the usual properties of a font, radiant also assigns a fixed color and size
/// to each font object. Instead of modifying these properties, you can clone a new font
/// with modified values using [`Font::with_color()`](struct.Font.html#method.with_color) and/or [`Font::with_size()`](struct.Font.html#method.with_size).
#[derive(Clone)]
pub struct Font {
    data    : Vec<u8>,
    font_id : usize,
    size    : f32,
    color   : Color,
    context : RenderContext,
}

impl Font {

    /// Creates a font instance from a file
    pub fn from_file(context: &RenderContext, file: &str) -> Font {
        let mut f = File::open(Path::new(file)).unwrap();
        let mut font_data = Vec::new();
        f.read_to_end(&mut font_data).unwrap();
        create_font(context, font_data, 12.0)
    }

    /// Creates a new font instance from given FontInfo struct
    pub fn from_info(context: &RenderContext, info: FontInfo) -> Font {
        let (font_data, _) = system_fonts::get(&build_property(&info)).unwrap();
        create_font(context, font_data, info.size)
    }

    /// Returns the names of all available system fonts
    pub fn query_all() -> Vec<String> {
        system_fonts::query_all()
    }

    /// Returns the names of all available system fonts with the given properties (e.g. monospace)
    pub fn query_specific(info: FontInfo) -> Vec<String> {
        system_fonts::query_specific(&mut build_property(&info))
    }

    /// Returns a new font instance with given size
    pub fn with_size(self: &Self, size: f32) -> Font {
        let mut font = (*self).clone();
        font.size = size;
        font
    }

    /// Returns a new font instance with given color
    pub fn with_color(self: &Self, color: Color) -> Font {
        let mut font = (*self).clone();
        font.color = color;
        font
    }

    /// Write to given layer
    pub fn write(self: &Self, layer: &Layer, text: &str, x: f32, y: f32) -> &Font {
        write(self, layer, text, x, y, 0.0, &self.color, 0.0, 1.0, 1.0);
        self
    }

    /// Write to given layer. Breaks lines after max_width pixels.
    pub fn write_wrapped(self: &Self, layer: &Layer, text: &str, x: f32, y: f32, max_width: f32) -> &Font {
        write(self, layer, text, x, y, max_width, &self.color, 0.0, 1.0, 1.0);
        self
    }

    /// Write to given layer. Breaks lines after max_width pixels and applies given rotation and scaling.
    pub fn write_transformed(self: &Self, layer: &Layer, text: &str, x: f32, y: f32, max_width: f32, rotation: f32, scale_x: f32, scale_y: f32) -> &Font {
        write(self, layer, text, x, y, max_width, &self.color, rotation, scale_x, scale_y);
        self
    }

}

/// creates a new cache texture for the renderer.
pub fn create_cache_texture(display: &glium::Display, width: u32, height: u32) -> glium::texture::Texture2d {
    glium::texture::Texture2d::with_format(
        display,
        glium::texture::RawImage2d {
            data: Cow::Owned(vec![128u8; width as usize * height as usize]),
            width: width,
            height: height,
            format: glium::texture::ClientFormat::U8
        },
        glium::texture::UncompressedFloatFormat::U8,
        glium::texture::MipmapsOption::NoMipmap
    ).unwrap()
}

/// creates a new unique font
fn create_font(context: &RenderContext, font_data: Vec<u8>, size: f32) -> Font {
    Font {
        data    : font_data,
        font_id : FONT_COUNTER.fetch_add(1, Ordering::Relaxed),
        size    : size,
        color   : Color::white(),
        context : context.clone(),
    }
}

/// write text to given layer using given font
fn write(font: &Font, layer: &Layer, text: &str, x: f32, y: f32, max_width: f32, color: &Color, rotation: f32, scale_x: f32, scale_y: f32) {

    // !todo probably expensive, but rusttype is completely opaque. would be nice to be able to store Font::info outside of a "may or may not own" container
    let rt_font = rusttype::FontCollection::from_bytes(&font.data[..]).into_font().unwrap();

    let bucket_id = 0;
    let glyphs = layout_paragraph(&rt_font, rusttype::Scale::uniform(font.size), max_width, &text);
    let context = rendercontext::lock(&font.context);

    context.font_cache.queue(font.font_id, &glyphs);

    let anchor = Point::new(0.0, 0.0);
    let scale = Point::new(scale_x, scale_y);
    let cos_rot = rotation.cos();
    let sin_rot = rotation.sin();

    for glyph in &glyphs {
        if let Some((uv, pos, dim)) = context.font_cache.rect_for(font.font_id, glyph) {
            let dist_x = pos.x * scale_x;
            let dist_y = pos.y * scale_y;
            let offset_x = x + dist_x * cos_rot - dist_y * sin_rot;
            let offset_y = y + dist_x * sin_rot + dist_y * cos_rot;
            layer::add_rect(layer, bucket_id, 0, uv, Point::new(offset_x, offset_y), anchor, dim, *color, rotation, scale);
        }
    }
}

/// layout a paragraph of glyphs
fn layout_paragraph<'a>(font: &'a rusttype::Font, scale: rusttype::Scale, width: f32, text: &str) -> Vec<rusttype::PositionedGlyph<'a>> {

    use unicode_normalization::UnicodeNormalization;
    let mut result = Vec::new();
    let v_metrics = font.v_metrics(scale);
    let advance_height = v_metrics.ascent - v_metrics.descent + v_metrics.line_gap;
    let mut caret = rusttype::point(0.0, v_metrics.ascent);
    let mut last_glyph_id = None;

    for c in text.nfc() {
        if c.is_control() {
            match c {
                '\r' => {
                    caret = rusttype::point(0.0, caret.y + advance_height);
                }
                '\n' => {},
                _ => {}
            }
            continue;
        }

        let base_glyph = if let Some(glyph) = font.glyph(c) {
            glyph
        } else {
            continue;
        };

        if let Some(id) = last_glyph_id.take() {
            caret.x += font.pair_kerning(scale, id, base_glyph.id());
        }

        last_glyph_id = Some(base_glyph.id());
        let mut glyph = base_glyph.scaled(scale).positioned(caret);

        if let Some(bb) = glyph.pixel_bounding_box() {
            if width > 0.0 && bb.max.x > width as i32 {
                caret = rusttype::point(0.0, caret.y + advance_height);
                glyph = glyph.into_unpositioned().positioned(caret);
                last_glyph_id = None;
            }
        }

        caret.x += glyph.unpositioned().h_metrics().advance_width;
        result.push(glyph);
    }
    result
}

/// builds a FontProperty for the underlying system_fonts library
fn build_property(info: &FontInfo) -> system_fonts::FontProperty {
    let mut property = system_fonts::FontPropertyBuilder::new();
    if info.family != "" {
        property = property.family(&info.family);
    }
    if info.italic {
        property = property.italic();
    }
    if info.oblique {
        property = property.oblique();
    }
    if info.bold {
        property = property.bold();
    }
    if info.monospace {
        property = property.monospace();
    }
    property.build()
}

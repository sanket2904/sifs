//! Images for the tests, drawn here: lines of text set with Core Text in a
//! bitmap, written by ImageIO as PNG, JPEG, HEIC, TIFF or GIF with the
//! properties a test needs. No image is committed.

use objc2_core_foundation::{
    CFAttributedString, CFBoolean, CFDictionary, CFNumber, CFRetained, CFString, CFType, CFURL, CGFloat, CGPoint, CGRect,
    CGSize,
};
use objc2_core_graphics::{CGBitmapContextCreate, CGBitmapContextCreateImage, CGColorSpace, CGContext, CGImage, CGImageAlphaInfo};
use objc2_core_text::{kCTFontAttributeName, kCTForegroundColorFromContextAttributeName, CTFont, CTLine};
use objc2_image_io::*;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Format {
    Png,
    Jpeg,
    Heic,
    Tiff,
    Gif,
}

impl Format {
    fn uti(self) -> &'static str {
        match self {
            Format::Png => "public.png",
            Format::Jpeg => "public.jpeg",
            Format::Heic => "public.heic",
            Format::Tiff => "public.tiff",
            Format::Gif => "com.compuserve.gif",
        }
    }
}

enum Value {
    Text(String),
    Number(f64),
}

/// The properties written with an image.
#[derive(Default)]
pub(super) struct Meta {
    exif: Vec<(&'static CFString, Value)>,
    tiff: Vec<(&'static CFString, Value)>,
    gps: Vec<(&'static CFString, Value)>,
    orientation: Option<f64>,
}

impl Meta {
    pub(super) fn taken(mut self, date: &str, offset: Option<&str>) -> Meta {
        self.exif.push((unsafe { kCGImagePropertyExifDateTimeOriginal }, Value::Text(date.into())));
        if let Some(offset) = offset {
            self.exif.push((unsafe { kCGImagePropertyExifOffsetTimeOriginal }, Value::Text(offset.into())));
        }
        self
    }

    pub(super) fn digitized(mut self, date: &str, offset: Option<&str>) -> Meta {
        self.exif.push((unsafe { kCGImagePropertyExifDateTimeDigitized }, Value::Text(date.into())));
        if let Some(offset) = offset {
            self.exif.push((unsafe { kCGImagePropertyExifOffsetTimeDigitized }, Value::Text(offset.into())));
        }
        self
    }

    pub(super) fn camera(mut self, make: &str, model: &str) -> Meta {
        self.tiff.push((unsafe { kCGImagePropertyTIFFMake }, Value::Text(make.into())));
        self.tiff.push((unsafe { kCGImagePropertyTIFFModel }, Value::Text(model.into())));
        self
    }

    pub(super) fn gps(mut self, latitude: f64, north_south: &str, longitude: f64, east_west: &str) -> Meta {
        self.gps.push((unsafe { kCGImagePropertyGPSLatitude }, Value::Number(latitude)));
        self.gps.push((unsafe { kCGImagePropertyGPSLatitudeRef }, Value::Text(north_south.into())));
        self.gps.push((unsafe { kCGImagePropertyGPSLongitude }, Value::Number(longitude)));
        self.gps.push((unsafe { kCGImagePropertyGPSLongitudeRef }, Value::Text(east_west.into())));
        self
    }

    pub(super) fn orientation(mut self, orientation: u8) -> Meta {
        self.orientation = Some(orientation as f64);
        self
    }

    fn dictionary(&self) -> CFRetained<CFDictionary<CFString, CFType>> {
        let mut pairs: Vec<(&CFString, CFRetained<CFType>)> = Vec::new();
        for (key, section) in [
            (unsafe { kCGImagePropertyExifDictionary }, &self.exif),
            (unsafe { kCGImagePropertyTIFFDictionary }, &self.tiff),
            (unsafe { kCGImagePropertyGPSDictionary }, &self.gps),
        ] {
            if !section.is_empty() {
                let values: Vec<(&CFString, CFRetained<CFType>)> = section.iter().map(|(k, v)| (*k, value(v))).collect();
                pairs.push((key, up(&*dictionary(&values))));
            }
        }
        if let Some(orientation) = self.orientation {
            pairs.push((unsafe { kCGImagePropertyOrientation }, up(&*CFNumber::new_f64(orientation))));
        }
        dictionary(&pairs)
    }
}

fn value(value: &Value) -> CFRetained<CFType> {
    match value {
        Value::Text(text) => up(&*CFString::from_str(text)),
        Value::Number(n) => up(&*CFNumber::new_f64(*n)),
    }
}

fn up(value: &impl AsRef<CFType>) -> CFRetained<CFType> {
    CFRetained::from(value.as_ref())
}

fn dictionary(pairs: &[(&CFString, CFRetained<CFType>)]) -> CFRetained<CFDictionary<CFString, CFType>> {
    let keys: Vec<&CFString> = pairs.iter().map(|(k, _)| *k).collect();
    let values: Vec<&CFType> = pairs.iter().map(|(_, v)| &**v).collect();
    CFDictionary::from_slices(&keys, &values)
}

/// Black lines of text on white, as a screenshot has them.
pub(super) fn text(path: &Path, format: Format, size: (usize, usize), lines: &[&str], meta: &Meta) {
    pages(path, format, size, &[lines], meta);
}

/// A picture of several pages or frames, each with its lines.
pub(super) fn pages(path: &Path, format: Format, size: (usize, usize), pages: &[&[&str]], meta: &Meta) {
    let contexts: Vec<_> = pages
        .iter()
        .map(|lines| {
            let context = canvas(size, (1.0, 1.0, 1.0));
            write_lines(&context, size, lines, (0.0, 0.0, 0.0));
            context
        })
        .collect();
    save(path, format, &contexts, meta);
}

/// Lines stored upside down, as a camera held upside down stores them;
/// orientation 3 in meta says to turn them back.
pub(super) fn upside_down(path: &Path, lines: &[&str], meta: &Meta) {
    let size = (1200, 150 * lines.len());
    let context = canvas(size, (1.0, 1.0, 1.0));
    CGContext::translate_ctm(Some(&context), size.0 as f64, size.1 as f64);
    CGContext::rotate_ctm(Some(&context), std::f64::consts::PI);
    write_lines(&context, size, lines, (0.0, 0.0, 0.0));
    save(path, Format::Jpeg, &[context], meta);
}

/// A street, more or less, with a green sign in white letters.
pub(super) fn photo(path: &Path, format: Format, lines: &[&str], meta: &Meta) {
    let size = (1600, 1200);
    let context = canvas(size, (0.55, 0.75, 0.95));
    fill(&context, (0.0, 0.0, 1600.0, 420.0), (0.35, 0.35, 0.37));
    fill(&context, (0.0, 420.0, 1600.0, 160.0), (0.30, 0.55, 0.25));
    fill(&context, (760.0, 0.0, 40.0, 700.0), (0.5, 0.5, 0.5));
    fill(&context, (300.0, 600.0, 1000.0, 380.0), (0.05, 0.40, 0.20));
    let sign = draw_lines(lines, 96.0);
    let mut y = 600.0 + 380.0 - 140.0;
    for line in sign {
        CGContext::set_text_position(Some(&context), 360.0, y);
        CGContext::set_rgb_fill_color(Some(&context), 1.0, 1.0, 1.0, 1.0);
        unsafe { line.draw(&context) };
        y -= 140.0;
    }
    save(path, format, &[context], meta);
}

fn canvas((width, height): (usize, usize), (r, g, b): (f64, f64, f64)) -> CFRetained<CGContext> {
    let space = CGColorSpace::new_device_rgb().unwrap();
    let info = CGImageAlphaInfo::NoneSkipLast.0;
    let context = unsafe { CGBitmapContextCreate(std::ptr::null_mut(), width, height, 8, 0, Some(&space), info) }.unwrap();
    fill(&context, (0.0, 0.0, width as f64, height as f64), (r, g, b));
    context
}

fn fill(context: &CGContext, (x, y, w, h): (f64, f64, f64, f64), (r, g, b): (f64, f64, f64)) {
    CGContext::set_rgb_fill_color(Some(context), r, g, b, 1.0);
    CGContext::fill_rect(Some(context), CGRect::new(CGPoint::new(x, y), CGSize::new(w, h)));
}

fn draw_lines(lines: &[&str], size: CGFloat) -> Vec<CFRetained<CTLine>> {
    let font = unsafe { CTFont::with_name(&CFString::from_str("Helvetica"), size, std::ptr::null()) };
    let from_context: &CFType = CFBoolean::new(true).as_ref();
    let attributes = dictionary(&[
        (unsafe { kCTFontAttributeName }, up(&*font)),
        (unsafe { kCTForegroundColorFromContextAttributeName }, up(from_context)),
    ]);
    lines
        .iter()
        .map(|line| {
            let string =
                unsafe { CFAttributedString::new(None, Some(&CFString::from_str(line)), Some(attributes.as_opaque())) }.unwrap();
            unsafe { CTLine::with_attributed_string(&string) }
        })
        .collect()
}

/// Lines from the top, each in a band of the picture's height.
fn write_lines(context: &CGContext, (width, height): (usize, usize), lines: &[&str], (r, g, b): (f64, f64, f64)) {
    if lines.is_empty() {
        return;
    }
    let band = height as f64 / lines.len() as f64;
    let size = (band * 0.5).min(width as f64 / 12.0).min(72.0);
    for (i, line) in draw_lines(lines, size).iter().enumerate() {
        let y = height as f64 - band * (i as f64 + 1.0) + (band - size) / 2.0;
        CGContext::set_text_position(Some(context), size / 2.0, y);
        CGContext::set_rgb_fill_color(Some(context), r, g, b, 1.0);
        unsafe { line.draw(context) };
    }
}

fn save(path: &Path, format: Format, contexts: &[CFRetained<CGContext>], meta: &Meta) {
    let url = CFURL::from_file_path(path).unwrap();
    let uti = CFString::from_str(format.uti());
    let destination = unsafe { CGImageDestination::with_url(&url, &uti, contexts.len(), None) }.unwrap();
    for context in contexts {
        let image: CFRetained<CGImage> = CGBitmapContextCreateImage(Some(context)).unwrap();
        unsafe { destination.add_image(&image, Some(meta.dictionary().as_opaque())) };
    }
    assert!(unsafe { destination.finalize() }, "wrote {}", path.display());
}

//! Thumbnail and preview generation, and the on-disk cache for both.
//!
//! Thumbnails are the small images in the grid. Previews are screen-sized
//! versions the viewer shows instead of very large originals, so a fitted
//! 6000×4000 photo costs a few MB of memory instead of ~100 MB.
//!
//! Each one is made once and saved in the app's cache folder. Its file name is
//! a hash of the source path, size and modified time, so an edited image
//! automatically gets a fresh one.

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::{CompressionType, FilterType as PngFilter, PngEncoder};
use image::imageops::FilterType;
use image::metadata::Orientation;
use image::{DynamicImage, ExtendedColorType, GrayImage, ImageDecoder, ImageEncoder, ImageReader, RgbImage};
use std::error::Error;
use std::fs::{self, File};
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::UNIX_EPOCH;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// Longest side of a thumbnail, in pixels. Large enough to stay sharp at the
/// biggest grid size on a high-DPI screen.
pub const THUMB_SIZE: u32 = 400;

/// Allowed preview sizes (longest side). The viewer picks the screen-sized one (1920 and up);
/// boards use the smallest that is sharp enough for how big an image is shown, so a board full
/// of medium-sized images doesn't load a screen-sized version of each.
pub const PREVIEW_SIZES: &[u32] = &[1024, 1920, 2560, 3840];

/// Decoding a very large image takes a lot of memory for a moment (width × height × 4 bytes:
/// about 140 MB for a 7000 × 5000 PNG). Images that need more than this are decoded one at a
/// time, so several of them arriving together (a board of huge images, say) can't add up to
/// gigabytes. Smaller images still run side by side on the other workers.
const BIG_DECODE_BYTES: u64 = 64 * 1024 * 1024;
static BIG_DECODE: Mutex<()> = Mutex::new(());

/// Bump this to invalidate every cached thumbnail (e.g. after changing how they are made).
const CACHE_VERSION: u32 = 1;

pub struct Thumbnail {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
}

pub enum Rendered {
    Image(Thumbnail),
    /// The original file is already small enough (or is animated), so show it as is.
    UseOriginal,
}

/// Makes (or fetches from cache) a version of `source` no larger than `size` pixels on its longest side.
pub fn render(cache_dir: &Path, source: &Path, size: u32) -> Result<Rendered> {
    if size > THUMB_SIZE {
        // Previews: never shrink animated GIFs, and never re-encode images that already fit.
        let is_gif = source.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("gif"));
        let (w, h) = dimensions(source)?;
        if is_gif || w.max(h) <= size {
            return Ok(Rendered::UseOriginal);
        }
    }
    get_or_create(cache_dir, source, size).map(Rendered::Image)
}

/// Width and height of an image as displayed (i.e. after EXIF rotation), read from its header only.
pub fn dimensions(source: &Path) -> Result<(u32, u32)> {
    let (w, h, orientation) = if is_jpeg(source) {
        let mut decoder = jpeg_decoder::Decoder::new(BufReader::new(File::open(source)?));
        decoder.read_info()?;
        let info = decoder.info().ok_or("missing JPEG header")?;
        let orientation = decoder.exif_data().and_then(Orientation::from_exif_chunk);
        (info.width as u32, info.height as u32, orientation)
    } else {
        let mut decoder = ImageReader::open(source)?.with_guessed_format()?.into_decoder()?;
        let (w, h) = decoder.dimensions();
        (w, h, decoder.orientation().ok())
    };
    let sideways = matches!(
        orientation,
        Some(Orientation::Rotate90 | Orientation::Rotate270 | Orientation::Rotate90FlipH | Orientation::Rotate270FlipH)
    );
    Ok(if sideways { (h, w) } else { (w, h) })
}

fn is_jpeg(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ["jpg", "jpeg", "jfif"].iter().any(|j| e.eq_ignore_ascii_case(j)))
}

fn get_or_create(cache_dir: &Path, source: &Path, size: u32) -> Result<Thumbnail> {
    let (dir, key) = cache_location(cache_dir, source, size)?;

    for (ext, mime) in [("jpg", "image/jpeg"), ("png", "image/png")] {
        if let Ok(bytes) = fs::read(dir.join(format!("{key}.{ext}"))) {
            return Ok(Thumbnail { bytes, mime });
        }
    }

    let img = {
        // Held until the full-size pixels are gone again (after resizing). A worker that panicked
        // on a broken file while holding it doesn't block the others (the panic is caught in lib.rs).
        let _turn = (decoded_bytes(source, size) > BIG_DECODE_BYTES)
            .then(|| BIG_DECODE.lock().unwrap_or_else(PoisonError::into_inner));
        let img = decode_reduced(source, size)?;
        if img.width() > size || img.height() > size {
            // Previews are viewed up close, so use a sharper (slightly slower) filter for them.
            let filter = if size > THUMB_SIZE { FilterType::CatmullRom } else { FilterType::Triangle };
            img.resize(size, size, filter)
        } else {
            img
        }
    };
    let quality = if size > THUMB_SIZE { 90 } else { 85 };
    let (bytes, ext, mime) = encode(&img, quality)?;

    // Saving to the cache is best effort: if it fails we still return the thumbnail.
    // Write to a temporary file first so a half-written thumbnail is never read back.
    if fs::create_dir_all(&dir).is_ok() {
        let tmp = dir.join(format!("{key}.{ext}.tmp"));
        if fs::write(&tmp, &bytes).is_ok() {
            let _ = fs::rename(&tmp, dir.join(format!("{key}.{ext}")));
        }
    }

    Ok(Thumbnail { bytes, mime })
}

fn cache_location(cache_dir: &Path, source: &Path, size: u32) -> Result<(PathBuf, String)> {
    let meta = fs::metadata(source)?;
    let modified = meta.modified()?.duration_since(UNIX_EPOCH)?.as_nanos();
    let id = format!("{}|{}|{}|{}|{}", source.display(), meta.len(), modified, size, CACHE_VERSION);
    let key = format!("{:016x}", xxhash_rust::xxh3::xxh3_64(id.as_bytes()));
    // Spread files over 256 sub-folders so no single folder gets huge.
    Ok((cache_dir.join(&key[..2]), key))
}

/// Roughly how much memory decoding `source` towards `target` size takes, from its header.
/// JPEGs are decoded at 1/2, 1/4 or 1/8 size where that is still big enough (see below).
fn decoded_bytes(source: &Path, target: u32) -> u64 {
    let Ok((w, h)) = dimensions(source) else { return 0 };
    let (mut w, mut h) = (u64::from(w), u64::from(h));
    if is_jpeg(source) {
        for _ in 0..3 {
            if w.max(h) / 2 < u64::from(target) {
                break;
            }
            (w, h) = (w / 2, h / 2);
        }
    }
    w * h * 4
}

/// Decodes an image, already reduced towards `target` size where the format allows it.
fn decode_reduced(source: &Path, target: u32) -> Result<DynamicImage> {
    if is_jpeg(source) {
        if let Ok(img) = decode_jpeg_reduced(source, target) {
            return Ok(img);
        }
        // Unusual JPEGs (e.g. CMYK) fall through to the general decoder below.
    }

    let mut decoder = ImageReader::open(source)?.with_guessed_format()?.into_decoder()?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut img = DynamicImage::from_decoder(decoder)?;
    img.apply_orientation(orientation);
    Ok(img)
}

/// JPEGs can be decoded directly at 1/2, 1/4 or 1/8 size, which skips most of the work.
fn decode_jpeg_reduced(source: &Path, target: u32) -> Result<DynamicImage> {
    let mut decoder = jpeg_decoder::Decoder::new(BufReader::new(File::open(source)?));
    decoder.read_info()?;
    let info = decoder.info().ok_or("missing JPEG header")?;
    let orientation = decoder
        .exif_data()
        .and_then(Orientation::from_exif_chunk)
        .unwrap_or(Orientation::NoTransforms);

    let (w, h) = (info.width as f32, info.height as f32);
    let factor = (target as f32 / w.max(h)).min(1.0);
    let (w, h) = decoder.scale(((w * factor).ceil() as u16).max(1), ((h * factor).ceil() as u16).max(1))?;
    let pixels = decoder.decode()?;

    let (w, h) = (w as u32, h as u32);
    let mut img = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => DynamicImage::ImageRgb8(RgbImage::from_raw(w, h, pixels).ok_or("bad size")?),
        jpeg_decoder::PixelFormat::L8 => DynamicImage::ImageLuma8(GrayImage::from_raw(w, h, pixels).ok_or("bad size")?),
        _ => return Err("unsupported JPEG pixel format".into()),
    };
    img.apply_orientation(orientation);
    Ok(img)
}

/// Saves as JPEG, or as PNG when the image actually uses transparency.
fn encode(img: &DynamicImage, quality: u8) -> Result<(Vec<u8>, &'static str, &'static str)> {
    let mut out = Vec::new();
    if img.color().has_alpha() {
        let rgba = img.to_rgba8();
        if rgba.pixels().any(|p| p[3] < 255) {
            PngEncoder::new_with_quality(&mut out, CompressionType::Fast, PngFilter::Adaptive).write_image(
                rgba.as_raw(),
                rgba.width(),
                rgba.height(),
                ExtendedColorType::Rgba8,
            )?;
            return Ok((out, "png", "image/png"));
        }
    }
    let mut encoder = JpegEncoder::new_with_quality(&mut out, quality);
    match img {
        // The usual case (photos): already in the form the encoder wants, so no copy is made.
        DynamicImage::ImageRgb8(rgb) => encoder.encode_image(rgb)?,
        other => encoder.encode_image(&other.to_rgb8())?,
    }
    Ok((out, "jpg", "image/jpeg"))
}

pub fn mime_for(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("bmp") => "image/bmp",
        Some("avif") => "image/avif",
        _ => "image/jpeg",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb, Rgba};

    fn temp_folder(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("reflib-thumbs-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        folder
    }

    fn longest_side(rendered: Rendered) -> Option<(u32, &'static str)> {
        match rendered {
            Rendered::Image(thumb) => {
                let img = image::load_from_memory(&thumb.bytes).unwrap();
                Some((img.width().max(img.height()), thumb.mime))
            }
            Rendered::UseOriginal => None,
        }
    }

    #[test]
    fn every_size_is_made_no_bigger_than_asked_and_small_images_are_left_alone() {
        let folder = temp_folder("sizes");
        let cache = folder.join("cache");
        let photo = folder.join("photo.jpg");
        ImageBuffer::<Rgb<u8>, _>::from_pixel(3000, 2000, Rgb([200, 100, 50])).save(&photo).unwrap();
        let small = folder.join("small.png");
        ImageBuffer::<Rgb<u8>, _>::from_pixel(800, 600, Rgb([1, 2, 3])).save(&small).unwrap();

        assert_eq!(longest_side(render(&cache, &photo, THUMB_SIZE).unwrap()), Some((400, "image/jpeg")));
        for &size in PREVIEW_SIZES {
            let made = longest_side(render(&cache, &photo, size).unwrap());
            // A preview bigger than the image itself would be pointless: the original is used.
            assert_eq!(made, (size < 3000).then_some((size, "image/jpeg")), "preview size {size}");
            assert_eq!(longest_side(render(&cache, &small, size).unwrap()), None, "already fits in {size}");
        }
        // The grid always gets a thumbnail, and the second time it comes from the cache.
        assert_eq!(longest_side(render(&cache, &small, THUMB_SIZE).unwrap()), Some((400, "image/jpeg")));
        assert_eq!(longest_side(render(&cache, &small, THUMB_SIZE).unwrap()), Some((400, "image/jpeg")));
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn see_through_images_stay_see_through() {
        let folder = temp_folder("alpha");
        let clear = folder.join("clear.png");
        ImageBuffer::<Rgba<u8>, _>::from_pixel(900, 500, Rgba([10, 20, 30, 0])).save(&clear).unwrap();
        // Has an alpha channel, but nothing in it is see-through: a JPEG is smaller.
        let solid = folder.join("solid.png");
        ImageBuffer::<Rgba<u8>, _>::from_pixel(900, 500, Rgba([10, 20, 30, 255])).save(&solid).unwrap();
        let cache = folder.join("cache");
        assert_eq!(longest_side(render(&cache, &clear, THUMB_SIZE).unwrap()), Some((400, "image/png")));
        assert_eq!(longest_side(render(&cache, &solid, THUMB_SIZE).unwrap()), Some((400, "image/jpeg")));
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn only_really_big_decodes_wait_their_turn() {
        let folder = temp_folder("big");
        let (jpeg, png) = (folder.join("big.jpg"), folder.join("big.png"));
        ImageBuffer::<Rgb<u8>, _>::from_pixel(6000, 4000, Rgb([9, 9, 9])).save(&jpeg).unwrap();
        ImageBuffer::<Rgb<u8>, _>::from_pixel(6000, 4000, Rgb([9, 9, 9])).save(&png).unwrap();
        // A JPEG is decoded at 1/8 size for a thumbnail (750 × 500), so it's small...
        assert!(decoded_bytes(&jpeg, THUMB_SIZE) < BIG_DECODE_BYTES);
        // ...but a PNG always has to be decoded whole (6000 × 4000 × 4 bytes = 96 MB).
        assert!(decoded_bytes(&png, THUMB_SIZE) > BIG_DECODE_BYTES);
        // For a 3840 preview the JPEG can't be reduced (half size would be too small).
        assert!(decoded_bytes(&jpeg, 3840) > BIG_DECODE_BYTES);
        fs::remove_dir_all(&folder).unwrap();
    }
}

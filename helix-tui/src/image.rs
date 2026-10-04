//! Inline images through the kitty graphics protocol or sixel.

use std::{
    fmt::{self, Write as _},
    path::Path,
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc,
    },
};

use base64::Engine as _;
use helix_view::{editor::ImageProtocolConfig, graphics::Rect};
use image::{
    codecs::png::{CompressionType, FilterType as PngFilter, PngEncoder},
    imageops::FilterType,
    ImageEncoder as _,
};
pub use image::RgbaImage;

/// Images bigger than this (in pixels, per dimension) are downscaled when loaded.
const MAX_IMAGE_SIZE: u32 = 1024;

/// Deletes every kitty image placement on screen.
pub const KITTY_DELETE_ALL: &str = "\x1b_Ga=d,d=A,q=2\x1b\\";

/// An image drawn over `area` of a [`crate::buffer::Buffer`]. The covered cells should be blank.
#[derive(Clone)]
pub struct Image {
    pub area: Rect,
    pub pixels: Arc<RgbaImage>,
}

impl PartialEq for Image {
    fn eq(&self, other: &Self) -> bool {
        self.area == other.area && Arc::ptr_eq(&self.pixels, &other.pixels)
    }
}

impl Eq for Image {}

impl fmt::Debug for Image {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Image")
            .field("area", &self.area)
            .field("dimensions", &self.pixels.dimensions())
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Kitty,
    Sixel,
}

/// The active protocol: 0 for none, otherwise `Protocol as u8 + 1`.
static PROTOCOL: AtomicU8 = AtomicU8::new(0);

/// The graphics protocol used to draw images, if any.
pub fn protocol() -> Option<Protocol> {
    match PROTOCOL.load(Ordering::Relaxed) {
        1 => Some(Protocol::Kitty),
        2 => Some(Protocol::Sixel),
        _ => None,
    }
}

/// Sets the protocol from the config. `sixel` tells whether the terminal reported sixel support.
pub fn set_protocol(config: ImageProtocolConfig, sixel: bool) {
    let protocol = match config {
        ImageProtocolConfig::Auto => detect(sixel),
        ImageProtocolConfig::Kitty => Some(Protocol::Kitty),
        ImageProtocolConfig::Sixel => Some(Protocol::Sixel),
        ImageProtocolConfig::Disabled => None,
    };
    log::debug!("Image protocol: {protocol:?}");
    PROTOCOL.store(protocol.map_or(0, |p| p as u8 + 1), Ordering::Relaxed);
}

// ponytail: kitty is detected from the environment since termina doesn't parse kitty graphics
// query responses; query the terminal once it does.
fn detect(sixel: bool) -> Option<Protocol> {
    let var = |key| std::env::var(key).unwrap_or_default();
    // Multiplexers (which inherit the variables) would need passthrough for the kitty protocol,
    // tmux handles sixel itself.
    let multiplexed = std::env::var_os("TMUX").is_some() || std::env::var_os("ZELLIJ").is_some();
    let kitty = std::env::var_os("KITTY_WINDOW_ID").is_some()
        || var("TERM") == "xterm-kitty"
        || var("TERM_PROGRAM") == "ghostty";
    if kitty && !multiplexed {
        Some(Protocol::Kitty)
    } else if sixel {
        Some(Protocol::Sixel)
    } else {
        None
    }
}

/// Decodes the image at `path`, or `None` if it isn't a supported image.
pub fn load(path: &Path) -> Option<RgbaImage> {
    image::ImageFormat::from_path(path).ok()?;
    let image = image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()?;
    let image = if image.width() > MAX_IMAGE_SIZE || image.height() > MAX_IMAGE_SIZE {
        image.thumbnail(MAX_IMAGE_SIZE, MAX_IMAGE_SIZE)
    } else {
        image
    };
    Some(image.into_rgba8())
}

/// Escape sequence drawing `image` scaled to fit and centered in its area. `cell` is the cell
/// size in pixels.
pub fn encode(protocol: Protocol, image: &Image, cell: (u32, u32)) -> String {
    let (cell_w, cell_h) = cell;
    let area = image.area;
    let (w, h) = image.pixels.dimensions();
    let max_w = area.width as u32 * cell_w;
    // Sixel draws in bands of 6 pixels: round down so the last band stays inside the area.
    let max_h = area.height as u32 * cell_h / 6 * 6;
    let scale = (max_w as f64 / w as f64).min(max_h as f64 / h as f64);
    let (w, h) = (
        ((w as f64 * scale) as u32).max(1),
        ((h as f64 * scale) as u32).max(1),
    );
    let (cols, rows) = (w.div_ceil(cell_w), h.div_ceil(cell_h));
    let x = area.x + area.width.saturating_sub(cols as u16) / 2;
    let y = area.y + area.height.saturating_sub(rows as u16) / 2;

    let pixels = image::imageops::resize(&*image.pixels, w, h, FilterType::Nearest);
    let mut out = format!("\x1b[{};{}H", y + 1, x + 1);
    match protocol {
        Protocol::Kitty => encode_kitty(&mut out, &pixels),
        Protocol::Sixel => encode_sixel(&mut out, &pixels),
    }
    out
}

/// Transmits and displays `image` at its pixel size.
fn encode_kitty(out: &mut String, image: &RgbaImage) {
    let mut png = Vec::new();
    let (w, h) = image.dimensions();
    let encoder =
        PngEncoder::new_with_quality(&mut png, CompressionType::Fast, PngFilter::Adaptive);
    if let Err(err) = encoder.write_image(image, w, h, image::ExtendedColorType::Rgba8) {
        log::error!("failed to encode image: {err}");
        out.clear();
        return;
    }
    let data = base64::engine::general_purpose::STANDARD.encode(png);
    // The payload must be sent in chunks of at most 4096 bytes.
    let mut chunks = data.as_bytes().chunks(4096).peekable();
    let mut first = true;
    while let Some(chunk) = chunks.next() {
        let more = chunks.peek().is_some() as u8;
        if first {
            // C=1: don't move the cursor, q=2: suppress responses.
            let _ = write!(out, "\x1b_Ga=T,f=100,q=2,C=1,m={more};");
            first = false;
        } else {
            let _ = write!(out, "\x1b_Gm={more};");
        }
        // base64 is ASCII.
        out.push_str(std::str::from_utf8(chunk).unwrap());
        out.push_str("\x1b\\");
    }
}

/// Sixel with a 6x6x6 color cube and ordered dithering. Transparent pixels are left untouched.
fn encode_sixel(out: &mut String, image: &RgbaImage) {
    const BAYER: [[u32; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
    const TRANSPARENT: u8 = u8::MAX;
    let (w, h) = image.dimensions();
    let indices: Vec<u8> = image
        .enumerate_pixels()
        .map(|(x, y, pixel)| {
            let [r, g, b, a] = pixel.0;
            if a < 128 {
                return TRANSPARENT;
            }
            let threshold = (2 * BAYER[y as usize % 4][x as usize % 4] + 1) * 255 / 2;
            let level = |v: u8| ((v as u32 * 80 + threshold) / 4080).min(5) as u8;
            level(r) * 36 + level(g) * 6 + level(b)
        })
        .collect();

    // P2=1: transparent background. Raster attributes: 1:1 pixel aspect ratio and the size.
    let _ = write!(out, "\x1bP0;1q\"1;1;{w};{h}");
    for i in 0..216u32 {
        let percent = |level: u32| level * 20;
        let _ = write!(
            out,
            "#{i};2;{};{};{}",
            percent(i / 36),
            percent(i / 6 % 6),
            percent(i % 6)
        );
    }

    let push_run = |out: &mut String, byte: u8, len: u32| {
        let ch = (b'?' + byte) as char;
        if len > 3 {
            let _ = write!(out, "!{len}{ch}");
        } else {
            (0..len).for_each(|_| out.push(ch));
        }
    };
    let (w, h) = (w as usize, h as usize);
    for band in (0..h).step_by(6) {
        let band_height = (h - band).min(6);
        let rows = &indices[band * w..(band + band_height) * w];
        let mut used = [false; 216];
        for &index in rows {
            if index != TRANSPARENT {
                used[index as usize] = true;
            }
        }
        let mut first = true;
        for color in (0..216u8).filter(|&color| used[color as usize]) {
            if !first {
                // Graphics carriage return: overlay the next color on the same band.
                out.push('$');
            }
            first = false;
            let _ = write!(out, "#{color}");
            let (mut run_byte, mut run_len) = (0u8, 0u32);
            for x in 0..w {
                let byte = (0..band_height)
                    .fold(0u8, |byte, dy| byte | ((rows[dy * w + x] == color) as u8) << dy);
                if byte == run_byte || run_len == 0 {
                    run_byte = byte;
                    run_len += 1;
                } else {
                    push_run(out, run_byte, run_len);
                    (run_byte, run_len) = (byte, 1);
                }
            }
            // Trailing empty pixels don't need to be drawn.
            if run_byte != 0 {
                push_run(out, run_byte, run_len);
            }
        }
        // Graphics new line: move to the next band.
        out.push('-');
    }
    out.push_str("\x1b\\");
}

//! SSD title bar text: shaping, measurement, and tail-ellipsis truncation,
//! plus rasterization onto a CPU pixel buffer via cosmic-text.
//!
//! The system font scan is slow, so the shared `FontSystem` is warmed once on a
//! background thread at startup (see `warm_fonts`) rather than lazily on the
//! render thread, where it would freeze the event loop. `SwashCache` stays
//! thread-local — it's only ever touched while rasterizing.
//!
//! With no fonts installed — a hermetic build sandbox, or a minimal system —
//! every function here degrades to empty output instead of panicking, so the
//! SSD simply renders a textless title bar.

use std::cell::RefCell;
use std::sync::{Mutex, OnceLock};

use cosmic_text::{
    Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache, Weight, Wrap,
};

use crate::config::FontWeight;

// `FontSystem::new()` scans every system font face (~1s in release per
// cosmic-text's docs); done lazily on first use it froze the single-threaded
// event loop the instant a title bar rendered. Warmed on a background thread
// instead (see `warm_fonts`); until it lands, text functions degrade to empty
// output.
static FONT_SYSTEM: OnceLock<Mutex<FontSystem>> = OnceLock::new();

thread_local! {
    // Only ever touched on the render thread, during rasterization.
    static SWASH_CACHE: RefCell<SwashCache> = RefCell::new(SwashCache::new());
}

/// Ellipsis appended to truncated titles.
const ELLIPSIS: char = '…';

// Callbacks awaiting the scan. Guarding on scan *started* (a non-empty queue)
// rather than on `FONT_SYSTEM` being set matters: the scan takes ~1s, and a
// caller arriving during it must not spawn a redundant scan thread (the
// multi-instance test harness hit a stampede of them).
static PENDING_ON_LOADED: Mutex<Vec<Box<dyn FnOnce() + Send>>> = Mutex::new(Vec::new());

/// Start scanning system fonts on a background thread. Idempotent; call once at
/// startup to keep the scan cost off the event-loop thread. `on_loaded` runs on
/// the worker thread once fonts are available (hence `Send`).
pub fn warm_fonts(on_loaded: impl FnOnce() + Send + 'static) {
    {
        let mut pending = PENDING_ON_LOADED.lock().unwrap();
        // The scan thread publishes FONT_SYSTEM *before* draining the queue
        // under this same lock, so a callback pushed while it is unset is
        // always drained, and one pushed after can't be — run it inline.
        if FONT_SYSTEM.get().is_some() {
            drop(pending);
            on_loaded();
            return;
        }
        pending.push(Box::new(on_loaded));
        if pending.len() > 1 {
            return;
        }
    }

    let spawned = std::thread::Builder::new()
        .name("driftwm-font-warm".into())
        .spawn(|| {
            let _ = FONT_SYSTEM.set(Mutex::new(FontSystem::new()));
            let callbacks = std::mem::take(&mut *PENDING_ON_LOADED.lock().unwrap());
            for callback in callbacks {
                callback();
            }
        });
    if let Err(err) = spawned {
        tracing::warn!("failed to spawn font-warm thread: {err}");
        // The queued callbacks would otherwise never fire.
        let callbacks = std::mem::take(&mut *PENDING_ON_LOADED.lock().unwrap());
        for callback in callbacks {
            callback();
        }
    }
}

/// Whether the background font scan has finished. Cheap enough to poll every
/// frame so a textless title bar re-renders with text once the scan lands.
pub fn fonts_ready() -> bool {
    FONT_SYSTEM.get().is_some()
}

/// Run `f` with the shared `FontSystem`, or `None` if the background scan hasn't
/// finished — callers degrade to empty output rather than block.
fn with_font_system<R>(f: impl FnOnce(&mut FontSystem) -> R) -> Option<R> {
    let mut fs = FONT_SYSTEM
        .get()?
        .lock()
        .expect("font system mutex poisoned");
    Some(f(&mut fs))
}

/// Whether the font database holds at least one font. False until the scan
/// finishes, and on systems with no fonts installed (hermetic build sandboxes);
/// shaping in that state panics ("no default font found") deep inside
/// cosmic-text, so the public functions below short-circuit instead.
fn fonts_available() -> bool {
    with_font_system(|fs| fs.db().faces().next().is_some()).unwrap_or(false)
}

/// Map the generic CSS family names to cosmic-text's generic `Family` variants
/// (which resolve via fontconfig aliases); any other string is a concrete face
/// name. `Family::Name("monospace")` would NOT resolve — it looks for a face
/// literally named "monospace" and silently falls back to the default font.
fn family_of(name: &str) -> Family<'_> {
    match name {
        "monospace" => Family::Monospace,
        "sans-serif" => Family::SansSerif,
        "serif" => Family::Serif,
        "cursive" => Family::Cursive,
        "fantasy" => Family::Fantasy,
        _ => Family::Name(name),
    }
}

fn weight_of(weight: FontWeight) -> Weight {
    match weight {
        FontWeight::Thin => Weight::THIN,
        FontWeight::ExtraLight => Weight::EXTRA_LIGHT,
        FontWeight::Light => Weight::LIGHT,
        FontWeight::Normal => Weight::NORMAL,
        FontWeight::Medium => Weight::MEDIUM,
        FontWeight::SemiBold => Weight::SEMIBOLD,
        FontWeight::Bold => Weight::BOLD,
        FontWeight::ExtraBold => Weight::EXTRA_BOLD,
        FontWeight::Black => Weight::BLACK,
    }
}

/// Shape `text` as a single unwrapped line.
fn shape_line(
    fs: &mut FontSystem,
    text: &str,
    font: &str,
    size: f32,
    weight: FontWeight,
) -> Buffer {
    let mut buffer = Buffer::new(fs, Metrics::new(size, size * 1.25));
    buffer.set_wrap(fs, Wrap::None);
    // No width constraint → never wraps; generous height fits the single line.
    buffer.set_size(fs, None, Some(size * 4.0));
    let attrs = Attrs::new()
        .family(family_of(font))
        .weight(weight_of(weight));
    buffer.set_text(fs, text, &attrs, Shaping::Advanced, None);
    buffer.shape_until_scroll(fs, false);
    buffer
}

/// Pixel width of `text` shaped at `size`. `0` for empty text.
pub fn measure(text: &str, font: &str, size: f32, weight: FontWeight) -> i32 {
    if text.is_empty() || !fonts_available() {
        return 0;
    }
    with_font_system(|fs| {
        shape_line(fs, text, font, size, weight)
            .layout_runs()
            .map(|run| run.line_w)
            .fold(0.0_f32, f32::max)
            .ceil() as i32
    })
    .unwrap_or(0)
}

/// Fit `text` into `max_width` pixels, tail-truncating with an ellipsis if it
/// overflows. Returns the (possibly truncated) string and its pixel width, or
/// an empty string when not even the ellipsis fits.
pub fn fit_text(
    text: &str,
    font: &str,
    size: f32,
    weight: FontWeight,
    max_width: i32,
) -> (String, i32) {
    if !fonts_available() {
        return (String::new(), 0);
    }
    let full = measure(text, font, size, weight);
    if full <= max_width {
        return (text.to_string(), full);
    }

    let ellipsis = ELLIPSIS.to_string();
    if measure(&ellipsis, font, size, weight) > max_width {
        return (String::new(), 0);
    }

    // Largest character prefix whose `prefix + …` still fits. `lo` always fits
    // (a zero-char prefix is just the ellipsis), `hi` never fits.
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo + 1 < hi {
        let mid = (lo + hi) / 2;
        let candidate: String = chars[..mid].iter().collect::<String>() + &ellipsis;
        if measure(&candidate, font, size, weight) <= max_width {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let result: String = chars[..lo].iter().collect::<String>() + &ellipsis;
    let width = measure(&result, font, size, weight);
    (result, width)
}

/// Alpha-composite `text` onto an `Abgr8888` pixel buffer, vertically centered
/// within `buf_h`. `origin_x` is the left pen position in buffer pixels.
#[allow(clippy::too_many_arguments)]
pub fn rasterize_into(
    pixels: &mut [u8],
    buf_w: i32,
    buf_h: i32,
    text: &str,
    font: &str,
    size: f32,
    weight: FontWeight,
    color: [u8; 4],
    origin_x: i32,
) {
    if text.is_empty() || !fonts_available() {
        return;
    }
    let base = Color::rgba(color[0], color[1], color[2], color[3]);
    let color_alpha = color[3] as f64 / 255.0;

    with_font_system(|fs| {
        SWASH_CACHE.with_borrow_mut(|cache| {
            let mut buffer = shape_line(fs, text, font, size, weight);
            // Center the line vertically using its actual glyph metrics.
            let (ascent, descent) = buffer
                .line_layout(fs, 0)
                .and_then(|lines| lines.first())
                .map(|line| (line.max_ascent, line.max_descent))
                .unwrap_or((size * 0.8, size * 0.2));
            let baseline = ((buf_h as f32 - (ascent + descent)) / 2.0 + ascent).round() as i32;
            for run in buffer.layout_runs() {
                for glyph in run.glyphs {
                    let pg = glyph.physical((origin_x as f32, baseline as f32), 1.0);
                    cache.with_pixels(fs, pg.cache_key, base, |cx, cy, gcolor| {
                        let x = pg.x + cx;
                        let y = pg.y + cy;
                        if x < 0 || y < 0 || x >= buf_w || y >= buf_h {
                            return;
                        }
                        let a = gcolor.a() as f64 / 255.0 * color_alpha;
                        if a <= 0.0 {
                            return;
                        }
                        let idx = ((y * buf_w + x) * 4) as usize;
                        let inv = 1.0 - a;
                        pixels[idx] = (gcolor.r() as f64 * a + pixels[idx] as f64 * inv) as u8;
                        pixels[idx + 1] =
                            (gcolor.g() as f64 * a + pixels[idx + 1] as f64 * inv) as u8;
                        pixels[idx + 2] =
                            (gcolor.b() as f64 * a + pixels[idx + 2] as f64 * inv) as u8;
                        pixels[idx + 3] = (pixels[idx + 3] as f64
                            + a * 255.0 * (1.0 - pixels[idx + 3] as f64 / 255.0))
                            as u8;
                    });
                }
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const FONT: &str = "sans-serif";

    /// Load fonts synchronously: tests have no background warm thread to await.
    fn ensure_fonts() {
        let _ = FONT_SYSTEM.set(Mutex::new(FontSystem::new()));
    }

    #[test]
    fn measure_nonempty_is_positive() {
        // Skipped where no fonts exist (hermetic build sandboxes): there is
        // nothing to shape against, and the functions degrade to empty output.
        ensure_fonts();
        if !fonts_available() {
            return;
        }
        assert!(measure("Hello", FONT, 13.0, FontWeight::Normal) > 0);
    }

    #[test]
    fn measure_empty_is_zero() {
        assert_eq!(measure("", FONT, 13.0, FontWeight::Normal), 0);
    }

    #[test]
    fn fit_text_returns_full_when_it_fits() {
        ensure_fonts();
        if !fonts_available() {
            return;
        }
        let (s, w) = fit_text("Hi", FONT, 13.0, FontWeight::Normal, 100_000);
        assert_eq!(s, "Hi");
        assert!(w > 0);
    }

    #[test]
    fn fit_text_ellipsizes_when_too_wide() {
        ensure_fonts();
        if !fonts_available() {
            return;
        }
        let long = "A very long window title that will never fit in here";
        let full = measure(long, FONT, 13.0, FontWeight::Normal);
        let max = full / 2;
        let (s, w) = fit_text(long, FONT, 13.0, FontWeight::Normal, max);
        assert!(s.ends_with('…'));
        assert!(s.chars().count() < long.chars().count());
        assert!(w <= max);
    }

    #[test]
    fn fit_text_empty_when_ellipsis_does_not_fit() {
        ensure_fonts();
        if !fonts_available() {
            return;
        }
        let (s, w) = fit_text("Title", FONT, 13.0, FontWeight::Normal, 1);
        assert_eq!(s, "");
        assert_eq!(w, 0);
    }
}

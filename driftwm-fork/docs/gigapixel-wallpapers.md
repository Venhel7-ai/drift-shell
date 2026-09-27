# Gigapixel wallpapers

driftwm's canvas is infinite, so the background can be far larger than one
screen — a gigapixel image becomes the canvas itself, something you pan over and
zoom out to take in. It has to be a **tiled pyramidal TIFF**, because an ordinary
PNG/JPG is uploaded as a single GPU texture and maxes out around 8K–16K pixels
per side. Tiled (cut into small squares) and pyramidal (stored at several
progressively smaller copies) lets driftwm load only the tiles in view and switch
to a smaller copy as you zoom out, so it never holds the whole image at full
resolution.

## Converting an image

The simplest route is [libvips](https://www.libvips.org/):

```bash
vips tiffsave input.jpg output.tif --tile --pyramid --bigtiff --compression=deflate
```

Then point your config at the result:

```toml
[background]
type = "tile"
path = "~/Pictures/output.tif"
```

`--compression=deflate` is lossless — preferable for a wallpaper you'll zoom
right into. `--compression=jpeg` is smaller but lossy, and needs `--rgbjpeg`
alongside it: without that flag libvips stores the tiles as YCbCr, which driftwm
can't decode.

### Alternative: GDAL

If your source is already a GeoTIFF or you work with GIS tools, GDAL produces an
equivalent tiled pyramid (a Cloud-Optimized GeoTIFF _is_ a tiled pyramidal TIFF):

```bash
gdal_translate -of GTiff -co TILED=YES -co COMPRESS=DEFLATE input.tif output.tif
gdaladdo -r average output.tif 2 4 8 16 32
```

## Requirements

A file that misses any of these falls back to the dot grid, with the reason on
the error bar:

- **RGB8 or RGBA8 pixels.** 16-bit, grayscale, CMYK, and YCbCr sources are
  rejected — convert to 8-bit RGB before tiling.
- **`type = "tile"`.** TIFF isn't supported in `wallpaper` mode, which reads
  PNG/JPEG only.
- **Tiled, not stripped**, which is what `--tile` / `-co TILED=YES` above
  produces.

`mirror_tile` is a no-op on pyramidal TIFFs — pre-mirror the source if you want
that look.

## What to expect

- **The first frames are blank.** Tiles load lazily, so roughly the first 5–10
  frames after startup or a config reload render nothing while the visible set
  fills in. That's normal, not a failure.
- **The image is centered on canvas (0, 0)** and repeats outward from there.
  `home-toggle` (`Mod+A` by default) brings you back to its center.
- **`[background] cache_budget_mb` governs sharpness.** It caps how much of the
  image is held on the GPU (128 MB by default, LRU-evicted). Too low and revisited
  areas stay blurry while the finer tiles reload; raise it on a large or HiDPI
  display, lower it on a memory-constrained machine.

## Where to find images

You need a single downloadable file — a large JPEG, PNG, or TIFF past ~16K on a
side — not a zoom viewer that only streams tiles. Beyond that it's down to taste:
the canvas tiles infinitely, but on something this large the repeat is far
off-screen, so a non-seamless edge rarely matters.

Some sources:

- **World & satellite maps** — NASA's
  [Blue Marble](https://science.nasa.gov/earth/earth-observatory/the-blue-marble-true-color-global-imagery-at-1km-resolution/)
  is a public-domain whole-Earth image up to 43200 × 21600 px (TIFF/JPG). Maps
  suit the canvas nicely — panning around one feels like exploring.
- **Wikimedia Commons** — [Large images](https://commons.wikimedia.org/wiki/Category:Large_images)
  and [Gigapixel images](https://commons.wikimedia.org/wiki/Category:Gigapixel_images):
  a big pool of maps, panoramas, and scans in the 16K–40K range, each with its
  license on its own page (many public domain or CC). If a download only gives you a
  thumbnail, see [downloading very large files](https://commons.wikimedia.org/wiki/Commons:Very_high-resolution_file_downloads).
- **Astronomy** — ESA/Hubble's [Andromeda mosaic](https://esahubble.org/images/heic2501a/)
  is 42208 × 9870 px under CC BY 4.0 (credit "ESA/Hubble"); NASA imagery is public
  domain.

Then run your pick through the conversion step above.

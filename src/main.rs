use anyhow::{anyhow, bail, Context};
use image::{Bgra, DynamicImage, ImageBuffer, Rgb, Rgba};
use imageproc::map::map_pixels;
use std::{convert::TryInto, env::args_os};
use xcb::{
    ffi::XCB_IMAGE_FORMAT_Z_PIXMAP, get_geometry, get_image, get_property, intern_atom, Pixmap,
    ATOM_PIXMAP,
};

const RGBA_DEPTH: u8 = 32;
const RGB_DEPTH: u8 = 24;

type BgraImage = ImageBuffer<Bgra<u8>, Vec<u8>>;

// Image grabbing logic based on https://github.com/neXromancers/shotgun and
// https://www.apriorit.com/dev-blog/672-lin-how-to-take-multi-monitor-screenshots-on-linux
// Pixmap grabbing based on https://github.com/polybar/polybar

fn main() -> anyhow::Result<()> {
    // Skip argv[0]
    let mut args = args_os().fuse().skip(1);
    let out_file = args.next().unwrap_or_else(|| "bg.png".into());

    // Fuse needed since first .next() might've already been None
    if args.next() != None {
        bail!("Too many arguments.");
    }

    let (c, _) = &xcb::Connection::connect(None)?;

    let root = c
        .get_setup()
        .roots()
        .next()
        .ok_or_else(|| anyhow!("No screen???"))?
        .root();

    let bg_atom = intern_atom(c, true, "_XROOTPMAP_ID")
        .get_reply()
        .context("Failed to get background atom ID")?
        .atom();

    let prop = get_property(c, false, root, bg_atom, ATOM_PIXMAP, 0, 1)
        .get_reply()
        .context("Failed to get background pixmap")?;

    // This is what Polybar does and it works
    if prop.format() != 32 {
        bail!("Unexpected pixmap reply format: {}", prop.format());
    }
    if prop.value_len() != 1 {
        bail!("Unexpected pixmap reply length: {}", prop.value_len());
    }

    let pixmap: Pixmap = prop.value()[0];
    let geometry = get_geometry(c, pixmap)
        .get_reply()
        .context("Failed to grab background geometry")?;

    let image = get_image(
        c,
        XCB_IMAGE_FORMAT_Z_PIXMAP.try_into().unwrap(),
        pixmap,
        geometry.x(),
        geometry.y(),
        geometry.width(),
        geometry.height(),
        !0, // All planes; X doesn't about extra bits
    )
    .get_reply()
    .context("Failed to grab background contents")?;

    let raw_image = BgraImage::from_raw(
        geometry.width().into(),
        geometry.height().into(),
        image.data().into(),
    )
    .ok_or_else(|| anyhow!("Failed to create image"))?;

    let image = match image.depth() {
        RGBA_DEPTH => {
            DynamicImage::ImageRgba8(map_pixels(&raw_image, |_, _, Bgra([b, g, r, a])| {
                Rgba([r, g, b, a])
            }))
        }
        RGB_DEPTH => DynamicImage::ImageRgb8(map_pixels(&raw_image, |_, _, Bgra([b, g, r, _])| {
            Rgb([r, g, b])
        })),
        depth => bail!("Unsupported pixel depth: {}", depth),
    };

    image.save(out_file).context("Failed to save image")?;

    Ok(())
}

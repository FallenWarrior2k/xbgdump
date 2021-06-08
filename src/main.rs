use anyhow::{bail, Context};
use image::{
    buffer::ConvertBuffer, pnm::PNMSubtype, Bgra, DynamicImage, ImageBuffer, ImageOutputFormat,
};
use std::{borrow::Cow, env::args_os, ffi::OsStr, io::stdout};
use x11rb::{
    connection::Connection,
    protocol::xproto::{AtomEnum, ConnectionExt, ImageFormat, Pixmap},
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
    let out_file: Cow<OsStr> = args
        .next()
        .map(Into::into)
        .unwrap_or(OsStr::new("bg.png").into());

    // Fuse needed since first .next() might've already been None
    if args.next() != None || out_file == OsStr::new("--help") {
        println!("USAGE: xbgdump [<outfile>.png|<outfile>.pam|-]");
        println!(
            "xbgdump saves the current X11 background to the specified file (or stdout for -)."
        );
        return Ok(());
    }

    let (c, screen_num) = x11rb::connect(None)?;
    let root = c.setup().roots[screen_num].root;

    let bg_atom = c
        .intern_atom(true, b"_XROOTPMAP_ID")
        .context("Failed to create cookie to retrieve background atom ID.")?
        .reply()
        .context("Failed to get background atom ID.")?
        .atom;

    let prop = c
        .get_property(false, root, bg_atom, AtomEnum::PIXMAP, 0, 1)
        .context("Failed to create cookie to get background pixmap.")?
        .reply()
        .context("Failed to get background pixmap.")?;

    // This is what Polybar does and it works
    let mut value_iter = prop
        .value32()
        .with_context(|| format!("Unexpected pixmap reply format {}.", prop.format))?;
    let pixmap: Pixmap = value_iter.next().context("No background pixmap set.")?;
    if value_iter.next() != None {
        bail!("Too many values in pixmap reply.");
    }

    let geometry = c
        .get_geometry(pixmap)
        .context("Failed to create cookie to retrieve background geometry.")?
        .reply()
        .context("Failed to grab background geometry.")?;

    let image_x = c
        .get_image(
            ImageFormat::Z_PIXMAP,
            pixmap,
            geometry.x,
            geometry.y,
            geometry.width,
            geometry.height,
            !0, // All planes; X doesn't about extra bits
        )
        .context("Failed to create cookie to retrieve background contents.")?
        .reply()
        .context("Failed to grab background contents.")?;

    let bgra = BgraImage::from_raw(geometry.width.into(), geometry.height.into(), image_x.data)
        .context("Failed to create image.")?;

    let processed_image = match image_x.depth {
        // I haven't actually tested this; it's just conjecture from 24-bit being BGR0
        RGBA_DEPTH => DynamicImage::ImageRgba8(bgra.convert()),
        RGB_DEPTH => DynamicImage::ImageRgb8(bgra.convert()),
        depth => bail!("Unsupported pixel depth {}.", depth),
    };

    if out_file == OsStr::new("-") {
        processed_image
            .write_to(
                &mut stdout(),
                ImageOutputFormat::Pnm(PNMSubtype::ArbitraryMap),
            )
            .context("Failed to write image.")?;
    } else {
        processed_image
            .save(out_file)
            .context("Failed to save image.")?;
    }

    Ok(())
}

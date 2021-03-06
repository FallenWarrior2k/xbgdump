use anyhow::{bail, Context};
use getopts::Options;
use image::{
    buffer::ConvertBuffer, pnm::PNMSubtype, Bgra, DynamicImage, GenericImage, GenericImageView,
    ImageBuffer, ImageOutputFormat, Rgba,
};
use std::{
    borrow::Cow,
    convert::{TryFrom, TryInto},
    env::args_os,
    io::stdout,
};
use x11rb::{
    connection::Connection,
    cookie::Cookie,
    protocol::{
        randr::{
            ConnectionExt as RRConnectionExt, GetCrtcInfoReply, GetScreenResourcesCurrentReply,
        },
        xproto::{AtomEnum, ConnectionExt, ImageFormat, Pixmap, Window},
    },
};

const RGBA_DEPTH: u8 = 32;
const RGB_DEPTH: u8 = 24;

type BgraImage = ImageBuffer<Bgra<u8>, Vec<u8>>;

fn print_usage(program: &str, opts: Options) {
    print!(
        "{}",
        opts.usage(&format!(
            "USAGE: {} [options] [<outfile>.png|<outfile>.pam|-]\n\
    xbgdump saves the current X11 background to the specified file (or stdout for -).",
            program
        ))
    )
}

// Image grabbing logic based on https://github.com/neXromancers/shotgun and
// https://www.apriorit.com/dev-blog/672-lin-how-to-take-multi-monitor-screenshots-on-linux
// Pixmap grabbing based on https://github.com/polybar/polybar

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = args_os().map(Cow::from).collect();
    let (program, args) = args
        .split_first()
        .map(|(p, a)| (p.to_string_lossy(), a))
        .unwrap_or(("xbgdump".into(), &[]));

    let mut opts = Options::new();
    opts.optflag("m", "mask", "Mask off-screen areas with full transparency.");
    opts.optflag("h", "help", "Show this help.");

    let parsed = opts.parse(args)?;

    if parsed.opt_present("h") || parsed.free.len() > 1 {
        print_usage(program.as_ref(), opts);
        // TODO: Exit code
        // Rust makes this very complicated
        // The only stable way to set the exit code is to use std::process::exit(), which has the
        // issue of not running destructors.
        // Setting the exit code via regular control flow requires the unstable Termination trait,
        // which is provided for Result, but always calls Debug::fmt in the error case, so there's
        // no way to silently exit with an error status without custom types.
        return Ok(());
    }

    let out_file: Cow<_> = parsed
        .free
        .first()
        .map(Into::into)
        .unwrap_or("bg.png".into());

    let mask = parsed.opt_present("m");

    let (c, screen_num) = x11rb::connect(None)?;
    let root = c.setup().roots[screen_num].root;

    let raw_bg = get_background(&c, root).context("Failed to get background image.")?;

    let processed_image = if mask {
        mask_offscreen(&c, root, raw_bg).context("Failed to mask off-screen areas.")?
    } else {
        raw_bg
    };

    if out_file == "-" {
        processed_image
            .write_to(
                &mut stdout(),
                ImageOutputFormat::Pnm(PNMSubtype::ArbitraryMap),
            )
            .context("Failed to write image.")?;
    } else {
        processed_image
            .save(out_file.as_ref())
            .context("Failed to save image.")?;
    }

    Ok(())
}

fn get_background(c: &impl Connection, root: Window) -> anyhow::Result<DynamicImage> {
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

    match image_x.depth {
        // I haven't actually tested this; it's just conjecture from 24-bit being BGR0
        RGBA_DEPTH => Ok(DynamicImage::ImageRgba8(bgra.convert())),
        RGB_DEPTH => Ok(DynamicImage::ImageRgb8(bgra.convert())),
        depth => bail!("Unsupported pixel depth {}.", depth),
    }
}

fn mask_offscreen(
    c: &impl Connection,
    root: Window,
    // Needs to be mutable for .sub_image(), even though it's never modified
    mut raw_bg: DynamicImage,
) -> anyhow::Result<DynamicImage> {
    // Largely inspired by the similar code in shotgun
    let GetScreenResourcesCurrentReply {
        config_timestamp,
        crtcs,
        ..
    } = c
        .randr_get_screen_resources_current(root)
        .context("Failed to create cookie to retrieve RandR resources.")?
        .reply()
        .context("Failed to retrieve RandR resources. Is RandR supported?")?;

    let crtc_info_cookies = crtcs
        .into_iter()
        .map(|crtc| c.randr_get_crtc_info(crtc, config_timestamp))
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to create cookies to retrieve screen layout.")?;
    let crtc_infos = crtc_info_cookies
        .into_iter()
        .map(Cookie::reply)
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to retrieve screen layout.")?;

    match crtc_infos.len() {
        0 => bail!("RandR reports zero screens."),
        1 => return Ok(raw_bg),
        _ => {}
    };

    let (total_width, total_height) = raw_bg.dimensions();
    let mut masked = ImageBuffer::from_pixel(total_width, total_height, Rgba([0, 0, 0, 0]));
    for GetCrtcInfoReply {
        x,
        y,
        width,
        height,
        ..
    } in crtc_infos
    {
        if i32::from(x) + i32::from(width) < 0 || i32::from(y) + i32::from(height) < 0 {
            // No on-screen portions, nothing to do
            continue;
        }

        // Do some clamping in case we're not entirely on-screen
        // I don't know if that's even possible for the root window,
        // but having the code is better than randomly tripping an assertion.
        let (x, width): (u32, u32) = if x < 0 {
            // Unwrap safe because width + x >= 0
            (0, u32::try_from(i32::from(width) + i32::from(x)).unwrap())
        } else {
            // Unwrap safe because x >= 0 at this point
            (x.try_into().unwrap(), width.into())
        };
        let (y, height): (u32, u32) = if y < 0 {
            // Unwrap safe because height + y >= 0
            (0, u32::try_from(i32::from(height) + i32::from(y)).unwrap())
        } else {
            // Unwrap safe because y >= 0 at this point
            (y.try_into().unwrap(), height.into())
        };

        let area = raw_bg.sub_image(x, y, width, height);
        masked.copy_from(&area, x, y).expect(
            "Failed to copy on-screen areas into final result. \
                        This is a bug in the sizing calculations.",
        );
    }

    Ok(DynamicImage::ImageRgba8(masked))
}

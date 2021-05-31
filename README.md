# xbgdump

`xbgdump` is a simple tool to dump the current X11 background to an image file.

You can use it like `xbgdump file.png` or `xbgdump -` to send data to stdout. By default, it writes to the file `bg.png` in the current directory.

For now, only PNG is supported, but in theory, it should be easy to expand support to all formats supported by [image-rs](https://github.com/image-rs/image). Especially PAM looks interesting for piping to e.g. ImageMagick, as the PNG encoding is by far the most expensive step at the moment, yet wasted work if the image immediately gets decoded again for further processing.

## Motivation

I made this because I use [nitrogen](https://github.com/l3ib/nitrogen) and [i3lock](https://github.com/i3/i3lock) as my screen locker. I wanted a blurred version of my background for my lock screen, but i3lock only takes a single image, which I didn't have, as nitrogen generates it on the fly.

I knew [polybar](https://github.com/polybar/polybar) inspects the background to implement pseudo-transparency, which is where I took the initial idea from. I then tried using [xprop](https://gitlab.freedesktop.org/xorg/app/xprop), but to the best of my knowledge, it appears to only let me retrieve the ID of the pixmap used, not its contents. Which then led to me making this.

## Internals

`xbgdump` works by retrieving the pixmap attached to the X root window under the property `_XROOTPMAP_ID`. This property is set by [feh](https://github.com/derf/feh) and nitrogen; I have not tested this with other wallpaper-setting tools or desktop environments yet.

For 8-bit RGB, the contents of this pixmap are returned by [xcb](https://github.com/rtbo/rust-xcb) as BGR0, which is then converted to RGB before being encoded as PNG and output to the given file or stdout.

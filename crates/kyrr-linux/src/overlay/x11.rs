//! The wave as an X window the window manager leaves alone (override
//! redirect, like a tooltip): GNOME on Wayland shows it through XWayland
//! above all windows, X11 desktops too. It needs a compositor for its round
//! corners; without one it is not shown.

use kyrr_common::wave::{self, Surface, MARGIN};
use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::shape::{ConnectionExt as _, SK, SO};
use x11rb::protocol::xproto::{
    AtomEnum, ClipOrdering, ColormapAlloc, ConnectionExt as _, CreateGCAux, CreateWindowAux, ImageFormat, ImageOrder,
    PropMode, VisualClass, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

pub struct Popup {
    connection: RustConnection,
    window: u32,
    gc: u32,
    colormap: u32,
    scale: f32,
    size: (u32, u32),
    /// Middle of the pill's bottom edge.
    anchor: (i32, i32),
    mapped: bool,
}

impl Popup {
    pub fn open() -> Option<Popup> {
        let (connection, number) = x11rb::connect(None).ok()?;
        let screen = connection.setup().roots.get(number)?.clone();
        let root = screen.root;
        let atom = |name: &str| atom(&connection, name);
        // Without a compositor the corners would be black. XWayland always
        // has one, the Wayland compositor (GNOME).
        if std::env::var_os("WAYLAND_DISPLAY").is_none() {
            let compositor = atom(&format!("_NET_WM_CM_S{number}"))?;
            if connection.get_selection_owner(compositor).ok()?.reply().ok()?.owner == 0 {
                return None;
            }
        }
        // Without SHAPE the pill would catch clicks.
        connection.shape_query_version().ok()?.reply().ok()?;
        let visual = screen
            .allowed_depths
            .iter()
            .filter(|d| d.depth == 32)
            .flat_map(|d| &d.visuals)
            .find(|v| v.class == VisualClass::TRUE_COLOR)?
            .visual_id;

        let scale = dpi(&connection, root).map_or(1.0, |dpi| (dpi / 96.0).clamp(1.0, 4.0));
        let (x, y, width, height) =
            monitor(&connection, root).unwrap_or((0, 0, screen.width_in_pixels as i32, screen.height_in_pixels as i32));
        let bottom = workarea_bottom(&connection, root, atom("_NET_WORKAREA")?)
            .filter(|&b| b > y && b < y + height)
            .unwrap_or(y + height);
        let anchor = (x + width / 2, bottom - (MARGIN as f32 * scale) as i32);
        let size = wave::size(scale);

        let colormap = connection.generate_id().ok()?;
        connection
            .create_colormap(ColormapAlloc::NONE, colormap, root, visual)
            .ok()?;
        let window = connection.generate_id().ok()?;
        connection
            .create_window(
                32,
                window,
                root,
                (anchor.0 - size.0 as i32 / 2) as i16,
                (anchor.1 - size.1 as i32) as i16,
                size.0 as u16,
                size.1 as u16,
                0,
                WindowClass::INPUT_OUTPUT,
                visual,
                &CreateWindowAux::new()
                    .background_pixel(0)
                    .border_pixel(0)
                    .override_redirect(1)
                    .colormap(colormap),
            )
            .ok()?;
        let gc = connection.generate_id().ok()?;
        let kind = atom("_NET_WM_WINDOW_TYPE").zip(atom("_NET_WM_WINDOW_TYPE_NOTIFICATION"));
        let popup = Popup {
            connection,
            window,
            gc,
            colormap,
            scale,
            size,
            anchor,
            mapped: false,
        };
        let c = &popup.connection;
        c.create_gc(gc, window, &CreateGCAux::new()).ok()?;
        // Clicks go through to the window below.
        let _ = c.shape_rectangles(SO::SET, SK::INPUT, ClipOrdering::UNSORTED, window, 0, 0, &[]);
        let _ = c.change_property8(PropMode::REPLACE, window, AtomEnum::WM_NAME, AtomEnum::STRING, b"kyrr");
        let _ = c.change_property8(
            PropMode::REPLACE,
            window,
            AtomEnum::WM_CLASS,
            AtomEnum::STRING,
            b"kyrr\0kyrr\0",
        );
        if let Some((kind, notification)) = kind {
            let _ = c.change_property32(PropMode::REPLACE, window, kind, AtomEnum::ATOM, &[notification]);
        }
        c.flush().ok()?;
        Some(popup)
    }
}

fn atom(connection: &RustConnection, name: &str) -> Option<u32> {
    Some(connection.intern_atom(false, name.as_bytes()).ok()?.reply().ok()?.atom)
}

/// `Xft.dpi` from the X resources, which desktops set for HiDPI screens.
fn dpi(connection: &RustConnection, root: u32) -> Option<f32> {
    let reply = connection
        .get_property(false, root, AtomEnum::RESOURCE_MANAGER, AtomEnum::STRING, 0, 1 << 16)
        .ok()?
        .reply()
        .ok()?;
    let text = String::from_utf8_lossy(&reply.value);
    text.lines()
        .find_map(|line| line.strip_prefix("Xft.dpi:"))
        .and_then(|value| value.trim().parse().ok())
}

/// The primary monitor (else the first): x, y, width, height.
fn monitor(connection: &RustConnection, root: u32) -> Option<(i32, i32, i32, i32)> {
    let monitors = connection.randr_get_monitors(root, true).ok()?.reply().ok()?.monitors;
    let m = monitors.iter().find(|m| m.primary).or(monitors.first())?;
    Some((m.x as i32, m.y as i32, m.width as i32, m.height as i32))
}

/// Where the desktop's work area ends (above a bottom panel).
fn workarea_bottom(connection: &RustConnection, root: u32, workarea: u32) -> Option<i32> {
    let reply = connection
        .get_property(false, root, workarea, AtomEnum::CARDINAL, 0, 4)
        .ok()?
        .reply()
        .ok()?;
    let values: Vec<u32> = reply.value32()?.collect();
    let [_, y, _, height] = values[..] else {
        return None;
    };
    Some((y + height) as i32)
}

impl Surface for Popup {
    fn scale(&mut self) -> f32 {
        self.scale
    }

    fn dark(&mut self) -> bool {
        super::dark()
    }

    fn show(&mut self, pixels: &[u32], width: u32, height: u32) -> bool {
        let Some(pixels) = pixels.get(..(width * height) as usize) else {
            return false;
        };
        let c = &self.connection;
        if (width, height) != self.size {
            self.size = (width, height);
            let aux = x11rb::protocol::xproto::ConfigureWindowAux::new()
                .x(self.anchor.0 - width as i32 / 2)
                .y(self.anchor.1 - height as i32)
                .width(width)
                .height(height);
            if c.configure_window(self.window, &aux).is_err() {
                return false;
            }
        }
        let big_endian = c.setup().image_byte_order == ImageOrder::MSB_FIRST;
        let bytes: Vec<u8> = pixels
            .iter()
            .flat_map(|p| if big_endian { p.to_be_bytes() } else { p.to_le_bytes() })
            .collect();
        if !self.mapped {
            self.mapped = true;
            if c.map_window(self.window).is_err() {
                return false;
            }
        }
        let drawn = c.put_image(
            ImageFormat::Z_PIXMAP,
            self.window,
            self.gc,
            width as u16,
            height as u16,
            0,
            0,
            0,
            32,
            &bytes,
        );
        if drawn.is_err() {
            return false;
        }
        c.flush().is_ok()
    }
}

impl Drop for Popup {
    fn drop(&mut self) {
        let c = &self.connection;
        let _ = c.free_gc(self.gc);
        let _ = c.destroy_window(self.window);
        let _ = c.free_colormap(self.colormap);
        let _ = c.flush();
    }
}

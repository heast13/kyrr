//! The wave as a layer surface: above all windows, at the bottom of the
//! output the compositor picks (the one in use), without keyboard focus and
//! with an empty input region.

use std::fs::File;
use std::os::fd::AsFd;
use std::os::unix::fs::FileExt;

use kyrr_common::wave::{Surface, HEIGHT, MARGIN, WIDTH};
use rustix::fs::{memfd_create, MemfdFlags};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::wl_buffer::{self, WlBuffer};
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_region::WlRegion;
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_shm::{Format, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::protocol::wl_surface::{self, WlSurface};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::{Layer as Level, ZwlrLayerShellV1};
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::{self, Anchor, ZwlrLayerSurfaceV1};

pub struct Layer {
    queue: EventQueue<State>,
    state: State,
    shm: WlShm,
    surface: WlSurface,
    layer: ZwlrLayerSurfaceV1,
    pool: Option<Pool>,
}

/// Two buffers of one size in shared memory: one the compositor shows, one
/// to draw the next picture into.
struct Pool {
    file: File,
    pool: WlShmPool,
    buffers: [WlBuffer; 2],
    size: (u32, u32),
}

struct State {
    configured: bool,
    closed: bool,
    scale: i32,
    /// Buffers the compositor still reads, of the current pool.
    busy: [bool; 2],
    /// Counts pools, so a late release of an old buffer frees nothing new.
    pool: u32,
}

impl Layer {
    pub fn open() -> Option<Layer> {
        let connection = Connection::connect_to_env().ok()?;
        let (globals, mut queue) = registry_queue_init::<State>(&connection).ok()?;
        let handle = queue.handle();
        // Version 6 tells the surface's scale.
        let compositor: WlCompositor = globals.bind(&handle, 1..=6, ()).ok()?;
        let shm: WlShm = globals.bind(&handle, 1..=1, ()).ok()?;
        let shell: ZwlrLayerShellV1 = globals.bind(&handle, 1..=4, ()).ok()?;
        let surface = compositor.create_surface(&handle, ());
        let nothing = compositor.create_region(&handle, ());
        surface.set_input_region(Some(&nothing));
        nothing.destroy();
        let layer = shell.get_layer_surface(&surface, None, Level::Overlay, "kyrr".into(), &handle, ());
        layer.set_size(WIDTH, HEIGHT);
        layer.set_anchor(Anchor::Bottom);
        layer.set_margin(0, 0, MARGIN as i32, 0);
        surface.commit();
        let mut state = State {
            configured: false,
            closed: false,
            scale: 1,
            busy: [false; 2],
            pool: 0,
        };
        for _ in 0..3 {
            if state.configured || state.closed {
                break;
            }
            queue.roundtrip(&mut state).ok()?;
        }
        (state.configured && !state.closed).then_some(Layer {
            queue,
            state,
            shm,
            surface,
            layer,
            pool: None,
        })
    }

    /// Makes the buffers for pictures of this size, unless they are there.
    fn make_pool(&mut self, width: u32, height: u32) -> Option<()> {
        if self.pool.as_ref().is_none_or(|p| p.size != (width, height)) {
            if let Some(old) = self.pool.take() {
                old.buffers.iter().for_each(WlBuffer::destroy);
                old.pool.destroy();
            }
            let stride = width * 4;
            let bytes = stride * height;
            let file = File::from(memfd_create("kyrr-wave", MemfdFlags::CLOEXEC).ok()?);
            file.set_len(2 * bytes as u64).ok()?;
            let handle = self.queue.handle();
            let pool = self.shm.create_pool(file.as_fd(), 2 * bytes as i32, &handle, ());
            self.state.pool += 1;
            self.state.busy = [false; 2];
            let id = self.state.pool;
            let buffer = |index: usize| {
                pool.create_buffer(
                    (index as u32 * bytes) as i32,
                    width as i32,
                    height as i32,
                    stride as i32,
                    Format::Argb8888,
                    &handle,
                    (id, index),
                )
            };
            let buffers = [buffer(0), buffer(1)];
            self.pool = Some(Pool {
                file,
                pool,
                buffers,
                size: (width, height),
            });
        }
        Some(())
    }
}

impl Surface for Layer {
    fn scale(&mut self) -> f32 {
        self.state.scale as f32
    }

    fn show(&mut self, pixels: &[u32], width: u32, height: u32) -> bool {
        // Picks up released buffers, a new scale, and a closed surface.
        if self.queue.roundtrip(&mut self.state).is_err() || self.state.closed {
            return false;
        }
        // The buffer must be a whole multiple of the surface.
        let scale = (width / WIDTH).max(1);
        if width != WIDTH * scale || height != HEIGHT * scale {
            return true;
        }
        let Some(pool) = self.make_pool(width, height).and(self.pool.as_ref()) else {
            return false;
        };
        let Some(index) = self.state.busy.iter().position(|busy| !busy) else {
            return true;
        };
        let bytes: Vec<u8> = pixels.iter().flat_map(|p| p.to_ne_bytes()).collect();
        if pool.file.write_all_at(&bytes, (index * bytes.len()) as u64).is_err() {
            return false;
        }
        if self.surface.version() >= 3 {
            self.surface.set_buffer_scale(scale as i32);
        }
        self.surface.attach(Some(&pool.buffers[index]), 0, 0);
        self.surface.damage(0, 0, WIDTH as i32, HEIGHT as i32);
        self.surface.commit();
        self.state.busy[index] = true;
        self.queue.flush().is_ok()
    }
}

impl Drop for Layer {
    fn drop(&mut self) {
        self.layer.destroy();
        self.surface.destroy();
        if let Some(pool) = self.pool.take() {
            pool.buffers.iter().for_each(WlBuffer::destroy);
            pool.pool.destroy();
        }
        let _ = self.queue.flush();
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, ()> for State {
    fn event(
        state: &mut Self,
        layer: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure { serial, .. } => {
                layer.ack_configure(serial);
                state.configured = true;
            }
            zwlr_layer_surface_v1::Event::Closed => state.closed = true,
            _ => {}
        }
    }
}

impl Dispatch<WlSurface, ()> for State {
    fn event(state: &mut Self, _: &WlSurface, event: wl_surface::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_surface::Event::PreferredBufferScale { factor } = event {
            state.scale = factor.clamp(1, 4);
        }
    }
}

impl Dispatch<WlBuffer, (u32, usize)> for State {
    fn event(
        state: &mut Self,
        _: &WlBuffer,
        event: wl_buffer::Event,
        &(pool, index): &(u32, usize),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(event, wl_buffer::Event::Release) && pool == state.pool {
            state.busy[index] = false;
        }
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

macro_rules! ignore_events {
    ($($interface:ty),*) => {$(
        impl Dispatch<$interface, ()> for State {
            fn event(
                _: &mut Self,
                _: &$interface,
                _: <$interface as Proxy>::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }
    )*};
}

ignore_events!(WlCompositor, WlShm, WlShmPool, WlRegion, ZwlrLayerShellV1);

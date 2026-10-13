//! The virtual microphone: kyrr records the real microphone, cleans it and
//! offers the result as a PipeWire source named "kyrr", which becomes the
//! default microphone. Every app that records from the default gets the
//! clean voice; no app has to be set up.
//!
//! Two streams, wired like PipeWire's own loopback module: a capture stream
//! on the real microphone and a source stream that apps record from. Both
//! share a node group, so one clock drives them. When an app reads from
//! "kyrr", the capture stream triggers the source stream, which takes the
//! newest microphone buffer, runs it through the filter and hands it on in
//! the same cycle. The capture side is passive: while no app records from
//! "kyrr", the real microphone is not opened.
//!
//! If kyrr ends without restoring the default (a crash), PipeWire falls
//! back to the real microphone by itself, and "kyrr" is the default again
//! as soon as it comes back.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use pipewire as pw;
use pw::properties::properties;
use pw::spa;
use pw::spa::pod::Pod;
use pw::stream::{StreamFlags, StreamRc};
use pw::sys as pw_sys;

/// The source apps see. Also its node name, so it can be found again.
pub const NODE_NAME: &str = "kyrr";
/// The filter's rate; PipeWire converts from and to whatever the
/// microphone and the apps use.
pub const RATE: u32 = 48_000;

const DEFAULT_SOURCE_KEY: &str = "default.configured.audio.source";

/// A microphone PipeWire knows: its node name (stable across restarts) and
/// what the system calls it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Microphone {
    pub name: String,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Starting,
    /// No microphone is connected; "kyrr" waits for one.
    NoMicrophone,
    /// Records from this microphone (node name) whenever an app reads "kyrr".
    Running(String),
    Failed(String),
}

/// Runs on PipeWire's real-time thread with mono samples at [`RATE`]: no
/// allocation, no locks that could wait.
pub type Process = Box<dyn FnMut(&mut [f32]) + Send>;

enum Command {
    UseMicrophone(String),
    Quit,
}

#[derive(Default)]
struct Shared {
    microphones: Vec<(u32, Microphone)>,
    status: Option<Status>,
}

/// The running virtual microphone. Dropping it removes "kyrr" and gives the
/// default back to the microphone that had it.
pub struct VirtualMic {
    commands: pw::channel::Sender<Command>,
    shared: Arc<Mutex<Shared>>,
    thread: Option<JoinHandle<()>>,
}

impl VirtualMic {
    /// Starts "kyrr" on `microphone` (a node name), or on the system's
    /// default microphone when it is `None` or not connected.
    pub fn start(microphone: Option<String>, process: Process) -> Result<VirtualMic, String> {
        let shared = Arc::new(Mutex::new(Shared {
            status: Some(Status::Starting),
            ..Shared::default()
        }));
        let (commands, receiver) = pw::channel::channel();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let thread_shared = shared.clone();
        let thread = std::thread::Builder::new()
            .name("kyrr-pipewire".into())
            .spawn(move || {
                if let Err(e) = run(microphone, process, receiver, thread_shared, &ready_tx) {
                    let _ = ready_tx.send(Err(e));
                }
            })
            .map_err(|e| e.to_string())?;
        ready_rx.recv().map_err(|e| e.to_string())??;
        Ok(VirtualMic {
            commands,
            shared,
            thread: Some(thread),
        })
    }

    /// The real microphones, without "kyrr" itself.
    pub fn microphones(&self) -> Vec<Microphone> {
        let shared = lock(&self.shared);
        let mut mics: Vec<Microphone> = shared.microphones.iter().map(|(_, m)| m.clone()).collect();
        mics.sort_by(|a, b| a.description.cmp(&b.description));
        mics
    }

    pub fn status(&self) -> Status {
        lock(&self.shared).status.clone().unwrap_or(Status::Starting)
    }

    /// Records from another microphone (node name) from now on.
    pub fn use_microphone(&self, name: &str) {
        let _ = self.commands.send(Command::UseMicrophone(name.to_string()));
    }
}

impl Drop for VirtualMic {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Quit);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The real microphones, read once without starting "kyrr" (for the first
/// setup, before there is a [`VirtualMic`]).
pub fn list() -> Result<Vec<Microphone>, String> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None).map_err(|e| e.to_string())?;
    let context = pw::context::ContextRc::new(&mainloop, None).map_err(|e| e.to_string())?;
    let core = context
        .connect_rc(None)
        .map_err(|_| "PipeWire is not running".to_string())?;
    let registry = core.get_registry_rc().map_err(|e| e.to_string())?;
    let found = Rc::new(RefCell::new(Vec::new()));
    let sink = found.clone();
    let _registry_listener = registry
        .add_listener_local()
        .global(move |global| {
            if global.type_ != pw::types::ObjectType::Node {
                return;
            }
            if let Some(mic) = global.props.and_then(as_microphone) {
                sink.borrow_mut().push(mic);
            }
        })
        .register();
    let pending = core.sync(0).map_err(|e| e.to_string())?;
    let mainloop_weak = mainloop.downgrade();
    let _core_listener = core
        .add_listener_local()
        .done(move |id, seq| {
            if id == pw::core::PW_ID_CORE && seq == pending {
                if let Some(mainloop) = mainloop_weak.upgrade() {
                    mainloop.quit();
                }
            }
        })
        .register();
    mainloop.run();
    let mut mics = found.take();
    mics.sort_by(|a, b| a.description.cmp(&b.description));
    Ok(mics)
}

/// A node that is a real microphone (not "kyrr" itself).
fn as_microphone(props: &spa::utils::dict::DictRef) -> Option<Microphone> {
    let name = props.get(*pw::keys::NODE_NAME)?;
    if props.get(*pw::keys::MEDIA_CLASS) != Some("Audio/Source") || name == NODE_NAME {
        return None;
    }
    let description = props
        .get(*pw::keys::NODE_DESCRIPTION)
        .or_else(|| props.get(*pw::keys::NODE_NICK))
        .unwrap_or(name);
    Some(Microphone {
        name: name.to_string(),
        description: description.to_string(),
    })
}

fn lock(shared: &Mutex<Shared>) -> std::sync::MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(|p| p.into_inner())
}

/// Everything that lives on the PipeWire thread.
struct State {
    core: pw::core::CoreRc,
    shared: Arc<Mutex<Shared>>,
    /// The microphone asked for; `None` follows the system default.
    wanted: Option<String>,
    process: Arc<Mutex<Process>>,
    /// Becomes true once the registry has listed what exists at start.
    synced: bool,
    metadata: Option<(pw::metadata::Metadata, pw::metadata::MetadataListener)>,
    /// The default microphone before "kyrr" took over, as metadata JSON.
    previous_default: Option<String>,
    streams: Option<Streams>,
    /// Quitting once PipeWire confirms it got the restored default.
    quit_after: Option<pw::spa::utils::result::AsyncSeq>,
}

/// Fields drop in order: the listeners before the streams they belong to
/// (the source listener also points at the capture stream).
struct Streams {
    _source_listener: pw::stream::StreamListener<SourceData>,
    _capture_listener: pw::stream::StreamListener<()>,
    source: StreamRc,
    capture: StreamRc,
    microphone: String,
}

impl Drop for Streams {
    /// The callbacks run on PipeWire's real-time thread. Disconnecting takes
    /// both nodes out of it and waits until it has, so no callback is still
    /// running when the listeners and their data go.
    fn drop(&mut self) {
        let _ = self.source.disconnect();
        let _ = self.capture.disconnect();
    }
}

struct SourceData {
    /// The capture stream; it outlives this listener (both live in `Streams`,
    /// and the listeners go first).
    capture: *mut pw_sys::pw_stream,
    /// Only ever locked by the real-time thread; shared so the streams can
    /// be replaced without taking the filter apart.
    process: Arc<Mutex<Process>>,
}

fn run(
    microphone: Option<String>,
    process: Process,
    receiver: pw::channel::Receiver<Command>,
    shared: Arc<Mutex<Shared>>,
    ready: &std::sync::mpsc::Sender<Result<(), String>>,
) -> Result<(), String> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None).map_err(|e| e.to_string())?;
    let context = pw::context::ContextRc::new(&mainloop, None).map_err(|e| e.to_string())?;
    // WirePlumber lets a sandboxed (Flatpak) client change the default
    // microphone only when it says it manages audio, as EasyEffects does.
    let core = context
        .connect_rc(Some(properties! { *pw::keys::MEDIA_CATEGORY => "Manager" }))
        .map_err(|_| "PipeWire is not running".to_string())?;
    let registry = core.get_registry_rc().map_err(|e| e.to_string())?;

    let state = Rc::new(RefCell::new(State {
        core: core.clone(),
        shared,
        wanted: microphone,
        process: Arc::new(Mutex::new(process)),
        synced: false,
        metadata: None,
        previous_default: None,
        streams: None,
        quit_after: None,
    }));

    let registry_weak = registry.downgrade();
    let on_global = state.clone();
    let on_remove = state.clone();
    let _registry_listener = registry
        .add_listener_local()
        .global(move |global| {
            let Some(props) = global.props else { return };
            match global.type_ {
                pw::types::ObjectType::Node => {
                    let Some(mic) = as_microphone(props) else { return };
                    lock(&on_global.borrow().shared).microphones.push((global.id, mic));
                    if on_global.borrow().synced {
                        connect_if_needed(&on_global);
                    }
                }
                pw::types::ObjectType::Metadata => {
                    if props.get("metadata.name") != Some("default") {
                        return;
                    }
                    let Some(registry) = registry_weak.upgrade() else {
                        return;
                    };
                    let Ok(metadata) = registry.bind::<pw::metadata::Metadata, _>(global) else {
                        return;
                    };
                    let weak = Rc::downgrade(&on_global);
                    let listener = metadata
                        .add_listener_local()
                        .property(move |subject, key, _, value| {
                            if subject == 0 && key == Some(DEFAULT_SOURCE_KEY) {
                                if let Some(state) = weak.upgrade() {
                                    let value = value.map(str::to_string);
                                    if value.as_deref().and_then(json_name) != Some(NODE_NAME) {
                                        state.borrow_mut().previous_default = value;
                                    }
                                }
                            }
                            0
                        })
                        .register();
                    on_global.borrow_mut().metadata = Some((metadata, listener));
                }
                _ => {}
            }
        })
        .global_remove(move |id| {
            let state = on_remove.borrow();
            lock(&state.shared).microphones.retain(|(i, _)| *i != id);
        })
        .register();

    // Once the registry has listed everything that exists (first sync) and
    // the default metadata has told its values (second sync), pick the
    // microphone and start.
    let on_done = state.clone();
    let mainloop_weak = mainloop.downgrade();
    let listed = core.sync(0).map_err(|e| e.to_string())?;
    let told = Cell::new(None);
    let _core_listener = core
        .add_listener_local()
        .done(move |id, seq| {
            if id != pw::core::PW_ID_CORE {
                return;
            }
            if seq == listed {
                told.set(on_done.borrow().core.sync(0).ok());
            }
            if told.get() == Some(seq) && !on_done.borrow().synced {
                on_done.borrow_mut().synced = true;
                connect_if_needed(&on_done);
            }
            if on_done.borrow().quit_after == Some(seq) {
                if let Some(mainloop) = mainloop_weak.upgrade() {
                    mainloop.quit();
                }
            }
        })
        .register();

    let mainloop_weak = mainloop.downgrade();
    let on_command = state.clone();
    let _receiver = receiver.attach(mainloop.loop_(), move |command| match command {
        Command::UseMicrophone(name) => {
            on_command.borrow_mut().wanted = Some(name);
            reconnect(&on_command);
        }
        Command::Quit => {
            let mut state = on_command.borrow_mut();
            restore_default(&state);
            state.streams = None;
            match state.core.sync(0) {
                Ok(seq) => state.quit_after = Some(seq),
                Err(_) => {
                    if let Some(mainloop) = mainloop_weak.upgrade() {
                        mainloop.quit();
                    }
                }
            }
        }
    });

    let _ = ready.send(Ok(()));
    mainloop.run();
    state.borrow_mut().streams = None;
    Ok(())
}

/// The microphone to record from: the one asked for if it is there, else
/// the default from before "kyrr", else any.
fn choose_microphone(state: &State) -> Option<String> {
    let shared = lock(&state.shared);
    let present = |name: &str| shared.microphones.iter().any(|(_, m)| m.name == name);
    let previous = state.previous_default.as_deref().and_then(json_name);
    state
        .wanted
        .as_deref()
        .filter(|n| present(n))
        .or(previous.filter(|n| present(n)))
        .or(shared.microphones.first().map(|(_, m)| m.name.as_str()))
        .map(str::to_string)
}

fn connect_if_needed(state: &Rc<RefCell<State>>) {
    let wanted_missing = {
        let s = state.borrow();
        match &s.streams {
            None => false,
            // A microphone asked for came back: move to it.
            Some(streams) => s.wanted.as_ref().is_some_and(|w| *w != streams.microphone),
        }
    };
    if state.borrow().streams.is_some() && !wanted_missing {
        return;
    }
    reconnect(state);
}

/// (Re)creates both streams on the chosen microphone. "kyrr" goes away for
/// a moment; apps recording from it pick it up again as the default.
fn reconnect(state: &Rc<RefCell<State>>) {
    let Some(microphone) = choose_microphone(&state.borrow()) else {
        if state.borrow().streams.is_none() {
            set_status(&state.borrow(), Status::NoMicrophone);
        }
        return;
    };
    if state
        .borrow()
        .streams
        .as_ref()
        .is_some_and(|s| s.microphone == microphone)
    {
        return;
    }
    state.borrow_mut().streams = None;
    let (core, process) = {
        let s = state.borrow();
        (s.core.clone(), s.process.clone())
    };
    match create_streams(&core, &microphone, process) {
        Ok(streams) => {
            set_status(&state.borrow(), Status::Running(microphone));
            state.borrow_mut().streams = Some(streams);
            make_default(&state.borrow());
        }
        Err(e) => set_status(&state.borrow(), Status::Failed(e)),
    }
}

fn set_status(state: &State, status: Status) {
    lock(&state.shared).status = Some(status);
}

fn make_default(state: &State) {
    if let Some((metadata, _)) = &state.metadata {
        let value = format!("{{\"name\":\"{NODE_NAME}\"}}");
        metadata.set_property(0, DEFAULT_SOURCE_KEY, Some("Spa:String:JSON"), Some(&value));
    }
}

fn restore_default(state: &State) {
    if let Some((metadata, _)) = &state.metadata {
        match &state.previous_default {
            Some(v) => metadata.set_property(0, DEFAULT_SOURCE_KEY, Some("Spa:String:JSON"), Some(v)),
            None => metadata.set_property(0, DEFAULT_SOURCE_KEY, None, None),
        }
    }
}

/// `name` from metadata JSON like `{"name":"alsa_input.usb-..."}`.
fn json_name(json: &str) -> Option<&str> {
    let rest = &json[json.find("\"name\"")? + 6..];
    let rest = rest.trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
    Some(&rest[..rest.find('"')?])
}

fn audio_format() -> Vec<u8> {
    let mut info = spa::param::audio::AudioInfoRaw::new();
    info.set_format(spa::param::audio::AudioFormat::F32LE);
    info.set_rate(RATE);
    info.set_channels(1);
    let mut position = [0; spa::param::audio::MAX_CHANNELS];
    position[0] = spa::sys::SPA_AUDIO_CHANNEL_MONO;
    info.set_position(position);
    let object = spa::pod::Value::Object(spa::pod::Object {
        type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
        id: spa::param::ParamType::EnumFormat.as_raw(),
        properties: info.into(),
    });
    spa::pod::serialize::PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &object)
        .map(|(cursor, _)| cursor.into_inner())
        .unwrap_or_default()
}

fn create_streams(core: &pw::core::CoreRc, microphone: &str, process: Arc<Mutex<Process>>) -> Result<Streams, String> {
    let group = format!("kyrr-{}", std::process::id());
    let source = StreamRc::new(
        core.clone(),
        "kyrr",
        properties! {
            *pw::keys::MEDIA_CLASS => "Audio/Source",
            *pw::keys::NODE_NAME => NODE_NAME,
            *pw::keys::NODE_DESCRIPTION => "kyrr",
            *pw::keys::NODE_GROUP => group.as_str(),
            *pw::keys::NODE_LINK_GROUP => group.as_str(),
            *pw::keys::NODE_VIRTUAL => "true",
            *pw::keys::MEDIA_ICON_NAME => "audio-input-microphone",
            "resample.prefill" => "true",
        },
    )
    .map_err(|e| e.to_string())?;
    let capture = StreamRc::new(
        core.clone(),
        "kyrr microphone",
        properties! {
            *pw::keys::MEDIA_TYPE => "Audio",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::NODE_NAME => "kyrr.input",
            *pw::keys::NODE_DESCRIPTION => "kyrr",
            *pw::keys::NODE_GROUP => group.as_str(),
            *pw::keys::NODE_LINK_GROUP => group.as_str(),
            *pw::keys::NODE_VIRTUAL => "true",
            *pw::keys::NODE_PASSIVE => "true",
            *pw::keys::TARGET_OBJECT => microphone,
            // Never fall back to the default microphone: that is "kyrr".
            "node.dont-fallback" => "true",
            "resample.prefill" => "true",
        },
    )
    .map_err(|e| e.to_string())?;

    let capture_ptr = capture.as_raw_ptr();
    // A raw pointer, not the Rc: this runs on the real-time thread. The
    // source outlives the listener (see `Streams`).
    let source_ptr = source.as_raw_ptr();
    let capture_listener = capture
        .add_local_listener::<()>()
        .process(move |_, _| unsafe {
            pw_sys::pw_stream_trigger_process(source_ptr);
        })
        .register()
        .map_err(|e| e.to_string())?;
    let source_listener = source
        .add_local_listener_with_user_data(SourceData {
            capture: capture_ptr,
            process,
        })
        .process(|source, data| unsafe { fill(source, data) })
        .register()
        .map_err(|e| e.to_string())?;

    let format = audio_format();
    let connect = |stream: &StreamRc, direction, flags| {
        let Some(pod) = Pod::from_bytes(&format) else {
            return Err("invalid audio format".to_string());
        };
        stream
            .connect(direction, None, flags, &mut [pod])
            .map_err(|e| e.to_string())
    };
    let common = StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS | StreamFlags::RT_PROCESS;
    // The source first, so it is ready before the capture side triggers it.
    let connected = connect(&source, spa::utils::Direction::Output, common | StreamFlags::TRIGGER).and_then(|()| {
        let async_ = StreamFlags::from_bits_retain(pw_sys::pw_stream_flags_PW_STREAM_FLAG_ASYNC);
        connect(&capture, spa::utils::Direction::Input, common | async_)
    });
    connected?;
    Ok(Streams {
        _source_listener: source_listener,
        _capture_listener: capture_listener,
        source,
        capture,
        microphone: microphone.to_string(),
    })
}

/// The source's process callback: the newest microphone buffer, filtered,
/// into the source's buffer. Silence when the microphone has nothing.
unsafe fn fill(source: &pw::stream::Stream, data: &mut SourceData) {
    let input = newest_buffer(data.capture);
    let Some(mut out) = source.dequeue_buffer() else {
        if !input.is_null() {
            unsafe { pw_sys::pw_stream_queue_buffer(data.capture, input) };
        }
        return;
    };
    let requested = out.requested() as usize;
    let Some(out_data) = out.datas_mut().first_mut() else {
        if !input.is_null() {
            unsafe { pw_sys::pw_stream_queue_buffer(data.capture, input) };
        }
        return;
    };
    let mut frames = 0;
    if let Some(bytes) = out_data.data() {
        let samples = as_f32(bytes);
        let input_samples = unsafe { buffer_samples(input) };
        frames = match input_samples {
            Some(src) => {
                let n = src.len().min(samples.len());
                samples[..n].copy_from_slice(&src[..n]);
                n
            }
            None => {
                let n = if requested > 0 { requested } else { samples.len() }.min(samples.len());
                samples[..n].fill(0.0);
                n
            }
        };
        if let Ok(mut process) = data.process.try_lock() {
            process(&mut samples[..frames]);
        }
    }
    let chunk = out_data.chunk_mut();
    *chunk.offset_mut() = 0;
    *chunk.stride_mut() = 4;
    *chunk.size_mut() = (frames * 4) as u32;
    if !input.is_null() {
        unsafe { pw_sys::pw_stream_queue_buffer(data.capture, input) };
    }
}

/// Dequeues all waiting capture buffers and keeps the newest; null if none.
fn newest_buffer(capture: *mut pw_sys::pw_stream) -> *mut pw_sys::pw_buffer {
    let mut newest: *mut pw_sys::pw_buffer = std::ptr::null_mut();
    loop {
        let next = unsafe { pw_sys::pw_stream_dequeue_buffer(capture) };
        if next.is_null() {
            return newest;
        }
        if !newest.is_null() {
            unsafe { pw_sys::pw_stream_queue_buffer(capture, newest) };
        }
        newest = next;
    }
}

/// The samples a capture buffer holds.
unsafe fn buffer_samples<'a>(buffer: *mut pw_sys::pw_buffer) -> Option<&'a [f32]> {
    let spa_buffer = unsafe { buffer.as_ref()?.buffer.as_ref()? };
    if spa_buffer.n_datas == 0 {
        return None;
    }
    let data = unsafe { spa_buffer.datas.as_ref()? };
    let chunk = unsafe { data.chunk.as_ref()? };
    if data.data.is_null() {
        return None;
    }
    let offset = (chunk.offset % data.maxsize.max(1)) as usize;
    let size = (chunk.size as usize).min(data.maxsize as usize - offset);
    let start = unsafe { (data.data as *const u8).add(offset) };
    if start.align_offset(std::mem::align_of::<f32>()) != 0 {
        return None;
    }
    Some(unsafe { std::slice::from_raw_parts(start as *const f32, size / 4) })
}

fn as_f32(bytes: &mut [u8]) -> &mut [f32] {
    // PipeWire's buffers are aligned; a misaligned one would yield nothing.
    let (head, samples, _) = unsafe { bytes.align_to_mut::<f32>() };
    if head.is_empty() {
        samples
    } else {
        &mut []
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_name_from_metadata_json() {
        assert_eq!(
            json_name(r#"{"name":"alsa_input.usb-fifine"}"#),
            Some("alsa_input.usb-fifine")
        );
        assert_eq!(json_name(r#"{ "name" : "kyrr" }"#), Some("kyrr"));
        assert_eq!(json_name("{}"), None);
    }
}

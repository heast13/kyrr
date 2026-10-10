//! The audio processing object: a stream effect (SFX) on a capture endpoint
//! that runs the voice filter in place, after the driver's own effect.
//!
//! Every entry point catches panics: a panic unwinding into the audio engine
//! would take down `audiodg.exe` and with it all sound on the machine. When
//! anything is wrong the microphone passes through unchanged.

use std::os::windows::ffi::OsStrExt;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, TryLockError};
use std::time::{SystemTime, UNIX_EPOCH};

use kyrr_common::{endpoint, status, APO_CLSID, APO_NAME, REG_ENDPOINTS, REG_STATUS};
use kyrr_denoise::{Mode, Model, VoiceFilter};
use kyrr_voice::Voice;
use windows::core::{Ref, Result, GUID, HRESULT};
use windows::Win32::Foundation::{E_FAIL, E_INVALIDARG, E_POINTER};
use windows::Win32::Media::Audio::Apo::{
    APOInitSystemEffects, IAudioMediaType, IAudioProcessingObject, IAudioProcessingObjectConfiguration_Impl,
    IAudioProcessingObjectRT_Impl, IAudioProcessingObject_Impl, IAudioSystemEffects_Impl, APOERR_ALREADY_INITIALIZED,
    APOERR_FORMAT_NOT_SUPPORTED, APOERR_NOT_INITIALIZED, APO_CONNECTION_DESCRIPTOR, APO_CONNECTION_PROPERTY, APO_FLAG,
    APO_FLAG_DEFAULT, APO_FLAG_INPLACE, APO_REG_PROPERTIES, BUFFER_SILENT, BUFFER_VALID, UNCOMPRESSEDAUDIOFORMAT,
};
use windows::Win32::Media::Audio::PKEY_AudioEndpoint_GUID;
use windows::Win32::Media::Multimedia::KSDATAFORMAT_SUBTYPE_IEEE_FLOAT;
use windows::Win32::System::Com::CoTaskMemAlloc;
use windows::Win32::System::Com::StructuredStorage::PropVariantToBSTR;

use crate::child::Child;
use crate::log::log;
use crate::monitor::{monitor, peak, Probe};
use crate::object::Object;
use crate::registry::{read_string, write_values, Value};
use crate::settings::live;
use crate::swap::{model_for, Swap};

/// Live APO objects, for `DllCanUnloadNow`.
pub static OBJECTS: AtomicUsize = AtomicUsize::new(0);

/// Latency reported before the format is known: 70 ms, kyrr's value at the
/// common rates other than 48 kHz.
const DEFAULT_LATENCY_100NS: i64 = 70 * 10_000;

#[derive(Default)]
struct State {
    initialized: bool,
    /// Endpoint GUID, for finding the driver's effect.
    endpoint: Option<String>,
    child: Option<Child>,
    /// Set while locked for processing.
    filter: Option<VoiceFilter>,
    /// Set while locked for processing; runs even when the filter failed.
    voice: Option<Voice>,
    /// The before/after probe this instance is recording.
    probe: Option<Probe>,
    locked: bool,
    channels: usize,
    rate: u32,
}

/// The APO's state and logic; `object.rs` wraps it into the COM object the
/// audio engine sees, whose interface methods are implemented below.
pub struct KyrrApo {
    state: Mutex<State>,
    /// Hand-over of a filter with another model, see `swap.rs`.
    swap: Arc<Swap>,
    /// Channel count while locked, readable when the state is busy.
    channels: AtomicUsize,
    /// Declared last so it counts down after everything else is dropped.
    _count: ObjectCount,
}

/// Counts live objects for `DllCanUnloadNow`.
struct ObjectCount;

impl ObjectCount {
    fn new() -> ObjectCount {
        OBJECTS.fetch_add(1, Ordering::SeqCst);
        ObjectCount
    }
}

impl Drop for ObjectCount {
    fn drop(&mut self) {
        OBJECTS.fetch_sub(1, Ordering::SeqCst);
    }
}

impl KyrrApo {
    pub fn new() -> KyrrApo {
        // The settings watcher and model loaders run DLL code for the life
        // of the process; the DLL must never be unloaded under them.
        crate::pin_module();
        // Read the settings and start their watcher outside the audio thread.
        let _ = live();
        KyrrApo {
            state: Mutex::new(State::default()),
            swap: Swap::new(),
            channels: AtomicUsize::new(0),
            _count: ObjectCount::new(),
        }
    }

    /// The state, also after a panic poisoned the lock: every entry point
    /// catches panics, and the state stays usable.
    fn state(&self) -> Result<std::sync::MutexGuard<'_, State>> {
        Ok(self.state.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

/// Runs `f`, turning a panic into `E_FAIL` and logging it.
fn guarded<T>(what: &str, f: impl FnOnce() -> Result<T>) -> Result<T> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(payload) => {
            let msg = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_default();
            log(&format!("panic in {what}: {msg}"));
            Err(E_FAIL.into())
        }
    }
}

/// Sample rate and channel count of an uncompressed float32 format.
fn float_format(t: &IAudioMediaType) -> Option<(u32, usize)> {
    let mut f = UNCOMPRESSEDAUDIOFORMAT::default();
    unsafe { t.GetUncompressedAudioFormat(&mut f) }.ok()?;
    let float = f.guidFormatType == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT
        && f.dwBytesPerSampleContainer == 4
        && f.dwValidBitsPerSample == 32;
    (float && f.dwSamplesPerFrame > 0 && f.fFramesPerSecond >= 1.0)
        .then_some((f.fFramesPerSecond.round() as u32, f.dwSamplesPerFrame as usize))
}

/// kyrr processes float32 in place, so input and output formats must be the
/// same float format.
fn check_format(opposite: Option<&IAudioMediaType>, requested: Option<&IAudioMediaType>) -> Result<IAudioMediaType> {
    let requested = requested.ok_or(E_POINTER)?;
    let wanted = float_format(requested).ok_or(APOERR_FORMAT_NOT_SUPPORTED)?;
    if let Some(opposite) = opposite {
        if float_format(opposite).is_some_and(|o| o != wanted) {
            return Err(APOERR_FORMAT_NOT_SUPPORTED.into());
        }
    }
    Ok(requested.clone())
}

/// Folder of this DLL, where the models live.
fn dll_dir() -> Option<PathBuf> {
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::System::LibraryLoader::{
        GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
        GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    };
    let mut module = HMODULE::default();
    let anchor = dll_dir as *const () as *const u16;
    let flags = GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT;
    unsafe { GetModuleHandleExW(flags, windows::core::PCWSTR(anchor), &mut module) }.ok()?;
    let mut buf = vec![0u16; 32768];
    let len = unsafe { GetModuleFileNameW(Some(module), &mut buf) } as usize;
    if len == 0 || len >= buf.len() {
        return None;
    }
    PathBuf::from(String::from_utf16_lossy(&buf[..len]))
        .parent()
        .map(PathBuf::from)
}

/// Endpoint GUID string (as in the MMDevices registry) from the init data.
fn endpoint_guid(init: &[u8]) -> Option<String> {
    if init.len() < std::mem::size_of::<APOInitSystemEffects>() {
        return None;
    }
    let fx = unsafe { &*(init.as_ptr() as *const APOInitSystemEffects) };
    let store = fx.pAPOEndpointProperties.as_ref()?;
    let value = unsafe { store.GetValue(&PKEY_AudioEndpoint_GUID) }.ok()?;
    let text = unsafe { PropVariantToBSTR(&value) }.ok()?.to_string();
    (!text.is_empty()).then_some(text)
}

pub fn format_label(rate: u32, channels: usize) -> String {
    format!("{rate} Hz, {channels} ch")
}

pub fn report(state: &str, engine: &str, format: &str) {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    write_values(
        REG_STATUS,
        &[
            (status::STATE, Value::Str(state)),
            (status::ENGINE, Value::Str(engine)),
            (status::FORMAT, Value::Str(format)),
            (status::UPDATED, Value::Qword(now)),
        ],
    );
}

/// onnxruntime.dll needs the Visual C++ runtime, which audiodg.exe would
/// look for in System32 only. The installer ships it next to the DLL;
/// loading it from there first satisfies onnxruntime.dll's imports.
fn preload_vc_runtime(dir: &std::path::Path) {
    use windows::Win32::System::LibraryLoader::LoadLibraryW;
    for name in [
        "vcruntime140.dll",
        "vcruntime140_1.dll",
        "msvcp140.dll",
        "msvcp140_1.dll",
    ] {
        let path = dir.join(name);
        if path.is_file() {
            let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            // Kept loaded for the life of the process, like onnxruntime.dll.
            let _ = unsafe { LoadLibraryW(windows::core::PCWSTR(wide.as_ptr())) };
        }
    }
}

/// Loads `model` and builds the filter (slow; never on the audio thread).
pub fn build_filter(
    rate: u32,
    channels: usize,
    max_frames: usize,
    model: Model,
) -> std::result::Result<(VoiceFilter, String), String> {
    let dir = dll_dir().ok_or("install folder not found")?;
    let ort = dir.join("onnxruntime.dll");
    if !kyrr_denoise::native_possible(&dir.join(model.file_name())) {
        preload_vc_runtime(&dir);
    }
    let denoiser = kyrr_denoise::load(&dir, model, Some(&ort))?;
    #[cfg(feature = "native")]
    let engine = format!("{}, {}", denoiser.engine(), model.label());
    #[cfg(not(feature = "native"))]
    let engine = format!("onnx, {}", model.label());
    let filter = VoiceFilter::new(denoiser, rate, channels, max_frames.max(1))?;
    Ok((filter, engine))
}

impl IAudioProcessingObject_Impl for Object {
    fn Reset(&self) -> Result<()> {
        guarded("Reset", || {
            let mut st = self.state()?;
            if let Some(child) = &st.child {
                child.reset();
            }
            if let Some(filter) = st.filter.as_mut() {
                filter.reset();
            }
            if let Some(voice) = st.voice.as_mut() {
                voice.reset();
            }
            Ok(())
        })
    }

    fn GetLatency(&self) -> Result<i64> {
        guarded("GetLatency", || {
            let st = self.state()?;
            let own = match &st.filter {
                Some(f) => f.latency_frames() as i64 * 10_000_000 / f.rate().max(1) as i64,
                None => DEFAULT_LATENCY_100NS,
            };
            Ok(own + st.child.as_ref().map_or(0, Child::latency))
        })
    }

    fn GetRegistrationProperties(&self) -> Result<*mut APO_REG_PROPERTIES> {
        guarded("GetRegistrationProperties", || {
            let props = unsafe { CoTaskMemAlloc(std::mem::size_of::<APO_REG_PROPERTIES>()) } as *mut APO_REG_PROPERTIES;
            if props.is_null() {
                return Err(windows::Win32::Foundation::E_OUTOFMEMORY.into());
            }
            let mut p = APO_REG_PROPERTIES {
                clsid: GUID::from_u128(APO_CLSID),
                Flags: APO_FLAG(APO_FLAG_DEFAULT.0 | APO_FLAG_INPLACE.0),
                u32MajorVersion: 1,
                u32MinorVersion: 0,
                u32MinInputConnections: 1,
                u32MaxInputConnections: 1,
                u32MinOutputConnections: 1,
                u32MaxOutputConnections: 1,
                u32MaxInstances: u32::MAX,
                u32NumAPOInterfaces: 1,
                iidAPOInterfaceList: [<IAudioProcessingObject as windows::core::Interface>::IID],
                ..unsafe { std::mem::zeroed() }
            };
            copy_wide(&mut p.szFriendlyName, APO_NAME);
            copy_wide(&mut p.szCopyrightInfo, "Copyright (c) 2026 heast13, MIT OR Apache-2.0");
            unsafe { props.write(p) };
            Ok(props)
        })
    }

    fn Initialize(&self, cbdatasize: u32, pbydata: *const u8) -> Result<()> {
        guarded("Initialize", || {
            let mut st = self.state()?;
            if st.initialized {
                return Err(APOERR_ALREADY_INITIALIZED.into());
            }
            let init: &[u8] = if pbydata.is_null() || cbdatasize == 0 {
                &[]
            } else {
                unsafe { std::slice::from_raw_parts(pbydata, cbdatasize as usize) }
            };
            st.endpoint = endpoint_guid(init);
            if let Some(ep) = &st.endpoint {
                let key = format!(r"{REG_ENDPOINTS}\{ep}");
                if let Some(clsid) = read_string(&key, endpoint::CHILD_SFX) {
                    match parse_guid(&clsid).map(|g| Child::create(g, init)) {
                        Some(Ok(child)) => st.child = Some(child),
                        Some(Err(e)) => log(&format!("driver effect {clsid} failed, running without it: {e}")),
                        None => log(&format!("driver effect {clsid}: not a CLSID")),
                    }
                }
            }
            log(&format!(
                "initialized for endpoint {}, driver effect: {}",
                st.endpoint.as_deref().unwrap_or("?"),
                if st.child.is_some() { "yes" } else { "no" }
            ));
            st.initialized = true;
            Ok(())
        })
    }

    fn IsInputFormatSupported(
        &self,
        poppositeformat: Ref<IAudioMediaType>,
        prequestedinputformat: Ref<IAudioMediaType>,
    ) -> Result<IAudioMediaType> {
        guarded("IsInputFormatSupported", || {
            let st = self.state()?;
            match &st.child {
                Some(child) => {
                    let agreed = child.is_input_supported(poppositeformat.as_ref(), prequestedinputformat.as_ref())?;
                    check_format(None, Some(&agreed))
                }
                None => check_format(poppositeformat.as_ref(), prequestedinputformat.as_ref()),
            }
        })
    }

    fn IsOutputFormatSupported(
        &self,
        poppositeformat: Ref<IAudioMediaType>,
        prequestedoutputformat: Ref<IAudioMediaType>,
    ) -> Result<IAudioMediaType> {
        guarded("IsOutputFormatSupported", || {
            let st = self.state()?;
            match &st.child {
                Some(child) => {
                    let agreed =
                        child.is_output_supported(poppositeformat.as_ref(), prequestedoutputformat.as_ref())?;
                    check_format(None, Some(&agreed))
                }
                None => check_format(poppositeformat.as_ref(), prequestedoutputformat.as_ref()),
            }
        })
    }

    fn GetInputChannelCount(&self) -> Result<u32> {
        guarded("GetInputChannelCount", || {
            let st = self.state()?;
            if let Some(child) = &st.child {
                return child.input_channels();
            }
            if !st.locked {
                return Err(APOERR_NOT_INITIALIZED.into());
            }
            Ok(st.channels as u32)
        })
    }
}

impl IAudioProcessingObjectConfiguration_Impl for Object {
    fn LockForProcess(
        &self,
        u32numinputconnections: u32,
        ppinputconnections: *const *const APO_CONNECTION_DESCRIPTOR,
        u32numoutputconnections: u32,
        ppoutputconnections: *const *const APO_CONNECTION_DESCRIPTOR,
    ) -> Result<()> {
        guarded("LockForProcess", || {
            if u32numinputconnections != 1 || u32numoutputconnections != 1 {
                return Err(E_INVALIDARG.into());
            }
            if ppinputconnections.is_null() || ppoutputconnections.is_null() {
                return Err(E_POINTER.into());
            }
            let inputs = unsafe { std::slice::from_raw_parts(ppinputconnections, 1) };
            let outputs = unsafe { std::slice::from_raw_parts(ppoutputconnections, 1) };
            let input = unsafe { inputs[0].as_ref() }.ok_or(E_POINTER)?;
            let output = unsafe { outputs[0].as_ref() }.ok_or(E_POINTER)?;
            let in_fmt = input
                .pFormat
                .as_ref()
                .and_then(float_format)
                .ok_or(APOERR_FORMAT_NOT_SUPPORTED)?;
            let out_fmt = output
                .pFormat
                .as_ref()
                .and_then(float_format)
                .ok_or(APOERR_FORMAT_NOT_SUPPORTED)?;
            if in_fmt != out_fmt {
                return Err(APOERR_FORMAT_NOT_SUPPORTED.into());
            }

            let mut st = self.state()?;
            if let Some(child) = &st.child {
                child.lock(inputs, outputs)?;
            }
            let (rate, channels) = in_fmt;
            let max_frames = input.u32MaxFrameCount.max(output.u32MaxFrameCount) as usize;
            let format = format_label(rate, channels);
            let model = model_for(live().get().light_model);
            match build_filter(rate, channels, max_frames, model) {
                Ok((filter, engine)) => {
                    log(&format!("locked: {format}, up to {max_frames} frames, engine {engine}"));
                    report("running", &engine, &format);
                    st.filter = Some(filter);
                    self.swap.locked(rate, channels, max_frames, Some(model));
                }
                Err(e) => {
                    log(&format!("locked: {format}, passing audio through: {e}"));
                    report(&format!("passthrough: {e}"), "", &format);
                    st.filter = None;
                    self.swap.locked(rate, channels, max_frames, None);
                }
            }
            st.voice = Some(Voice::new(rate, channels));
            st.probe = None;
            // Create the shared monitor here, not on the audio thread.
            let _ = monitor();
            st.rate = rate;
            st.channels = channels;
            self.channels.store(channels, Ordering::Relaxed);
            st.locked = true;
            Ok(())
        })
    }

    fn UnlockForProcess(&self) -> Result<()> {
        guarded("UnlockForProcess", || {
            let mut st = self.state()?;
            if let Some(child) = &st.child {
                child.unlock();
            }
            self.swap.unlocked();
            st.filter = None;
            st.voice = None;
            st.probe = None;
            st.locked = false;
            self.channels.store(0, Ordering::Relaxed);
            Ok(())
        })
    }
}

impl IAudioProcessingObjectRT_Impl for Object {
    fn APOProcess(
        &self,
        u32numinputconnections: u32,
        ppinputconnections: *const *const APO_CONNECTION_PROPERTY,
        u32numoutputconnections: u32,
        ppoutputconnections: *mut *mut APO_CONNECTION_PROPERTY,
    ) {
        let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
            self.process(
                u32numinputconnections,
                ppinputconnections,
                u32numoutputconnections,
                ppoutputconnections,
            )
        }));
    }

    fn CalcInputFrames(&self, u32outputframecount: u32) -> u32 {
        u32outputframecount
    }

    fn CalcOutputFrames(&self, u32inputframecount: u32) -> u32 {
        u32inputframecount
    }
}

impl IAudioSystemEffects_Impl for Object {}

impl KyrrApo {
    /// Runs on the audio engine's real-time thread: no allocation, no
    /// blocking. If the state is busy (never expected), audio passes through.
    unsafe fn process(
        &self,
        n_in: u32,
        pp_in: *const *const APO_CONNECTION_PROPERTY,
        n_out: u32,
        pp_out: *mut *mut APO_CONNECTION_PROPERTY,
    ) {
        if n_in < 1 || n_out < 1 || pp_in.is_null() || pp_out.is_null() {
            return;
        }
        let (Some(input), Some(output)) = (unsafe { (*pp_in).as_ref() }, unsafe { (*pp_out).as_mut() }) else {
            return;
        };
        let mut st = match self.state.try_lock() {
            Ok(st) => st,
            Err(TryLockError::Poisoned(p)) => p.into_inner(),
            Err(TryLockError::WouldBlock) => {
                unsafe { pass_through(input, output, self.channels.load(Ordering::Relaxed)) };
                return;
            }
        };
        let channels = st.channels;
        if !st.locked || channels == 0 {
            return;
        }

        // The driver's effect runs first and writes its result to the output.
        let frames = if let Some(child) = &st.child {
            unsafe { child.process(n_in, pp_in, n_out, pp_out) };
            if output.u32BufferFlags == BUFFER_SILENT {
                unsafe { zero(output, channels) };
            }
            output.u32ValidFrameCount as usize
        } else {
            unsafe { pass_through(input, output, channels) };
            input.u32ValidFrameCount as usize
        };

        if output.pBuffer != 0 && frames > 0 {
            let buf = unsafe { std::slice::from_raw_parts_mut(output.pBuffer as *mut f32, frames * channels) };
            let s = live().get();
            let st = &mut *st;
            self.swap.take(&mut st.filter);
            let monitor = monitor();
            if let Some(m) = monitor {
                if st.probe.is_none() {
                    st.probe = m.claim_probe(st.rate);
                }
                if let Some(p) = &st.probe {
                    m.record(p, 0, buf, channels);
                }
            }
            let before = peak(buf);

            if let Some(voice) = st.voice.as_mut() {
                voice.set(&s.voice);
                voice.pre(buf);
            }
            if let Some(filter) = st.filter.as_mut() {
                filter.set_mode(if s.enabled { Mode::Process } else { Mode::Bypass });
                filter.set_limit_db(s.limit_db as f32);
                filter.process_in_place(buf);
            }
            if let Some(voice) = st.voice.as_mut() {
                voice.post(buf);
            }

            if let Some(m) = monitor {
                m.levels(before, peak(buf));
                if let Some(p) = st.probe.as_mut() {
                    m.record(p, 1, buf, channels);
                    if !m.advance(p, frames) {
                        st.probe = None;
                    }
                }
            }
        }
        output.u32ValidFrameCount = frames as u32;
        output.u32BufferFlags = BUFFER_VALID;
    }
}

/// Copies the input to the output (unless they share a buffer) and turns a
/// silent input into real zeros. `channels == 0` means unknown: copy nothing.
unsafe fn pass_through(input: &APO_CONNECTION_PROPERTY, output: &mut APO_CONNECTION_PROPERTY, channels: usize) {
    let frames = input.u32ValidFrameCount as usize;
    if channels > 0 && input.pBuffer != 0 && output.pBuffer != 0 {
        let n = frames * channels;
        if input.u32BufferFlags == BUFFER_SILENT {
            unsafe { std::ptr::write_bytes(output.pBuffer as *mut f32, 0, n) };
        } else if input.pBuffer != output.pBuffer {
            unsafe { std::ptr::copy_nonoverlapping(input.pBuffer as *const f32, output.pBuffer as *mut f32, n) };
        }
    }
    output.u32ValidFrameCount = input.u32ValidFrameCount;
    output.u32BufferFlags = input.u32BufferFlags;
}

unsafe fn zero(output: &mut APO_CONNECTION_PROPERTY, channels: usize) {
    if output.pBuffer != 0 {
        let n = output.u32ValidFrameCount as usize * channels;
        unsafe { std::ptr::write_bytes(output.pBuffer as *mut f32, 0, n) };
    }
}

fn copy_wide(dst: &mut [u16], s: &str) {
    for (d, c) in dst.iter_mut().zip(s.encode_utf16().chain(Some(0))) {
        *d = c;
    }
    if let Some(last) = dst.last_mut() {
        *last = 0;
    }
}

/// Parses `{XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}`.
pub fn parse_guid(s: &str) -> Option<GUID> {
    let hex: String = s
        .trim()
        .trim_matches(['{', '}'])
        .chars()
        .filter(|c| *c != '-')
        .collect();
    if hex.len() != 32 {
        return None;
    }
    u128::from_str_radix(&hex, 16).ok().map(GUID::from_u128)
}

/// `HRESULT` for `DllCanUnloadNow`.
pub fn can_unload() -> HRESULT {
    if OBJECTS.load(Ordering::SeqCst) == 0 {
        windows::Win32::Foundation::S_OK
    } else {
        windows::Win32::Foundation::S_FALSE
    }
}

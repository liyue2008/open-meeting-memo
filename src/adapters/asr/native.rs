//! Single-threaded safe ownership layer over the pinned C++/ONNX C ABI.
use super::{audio::RATE, ffi::*};
use crate::core::contracts::*;
use libloading::Library;
use std::{
    ffi::{c_char, c_void, CStr, CString},
    path::Path,
    ptr::NonNull,
};

type Handle = *const c_void;
macro_rules! api {
    ($($name:ident: $ty:ty => $symbol:literal),+ $(,)?) => {
        struct Api { $($name: $ty,)+ _library: Library }
        impl Api {
            unsafe fn load(path: &Path) -> Result<Self> {
                let library = Library::new(path).map_err(|_| OmmError::unavailable("configured sherpa-onnx library could not load"))?;
                $(let $name = *library.get::<$ty>(concat!($symbol, "\0").as_bytes()).map_err(|_| OmmError::unavailable("sherpa-onnx 1.13.2 C API symbol missing"))?;)+
                let api = Self { $($name,)+ _library: library };
                let version = (api.version)();
                if version.is_null() || CStr::from_ptr(version).to_bytes() != b"1.13.2" { return Err(OmmError::unavailable("sherpa-onnx library version must be exactly 1.13.2")); }
                Ok(api)
            }
        }
    }
}
api! {
    version: unsafe extern "C" fn() -> *const c_char => "SherpaOnnxGetVersionStr",
    create: unsafe extern "C" fn(*const SherpaOnnxOfflineRecognizerConfig) -> Handle => "SherpaOnnxCreateOfflineRecognizer",
    destroy: unsafe extern "C" fn(Handle) -> () => "SherpaOnnxDestroyOfflineRecognizer",
    stream: unsafe extern "C" fn(Handle) -> Handle => "SherpaOnnxCreateOfflineStream",
    destroy_stream: unsafe extern "C" fn(Handle) -> () => "SherpaOnnxDestroyOfflineStream",
    accept: unsafe extern "C" fn(Handle, i32, *const f32, i32) -> () => "SherpaOnnxAcceptWaveformOffline",
    decode: unsafe extern "C" fn(Handle, *const Handle, i32) -> () => "SherpaOnnxDecodeMultipleOfflineStreams",
    result: unsafe extern "C" fn(Handle) -> *const SherpaOnnxOfflineRecognizerResult => "SherpaOnnxGetOfflineStreamResult",
    destroy_result: unsafe extern "C" fn(Handle) -> () => "SherpaOnnxDestroyOfflineRecognizerResult",
    vad_create: unsafe extern "C" fn(*const SherpaOnnxVadModelConfig, f32) -> Handle => "SherpaOnnxCreateVoiceActivityDetector",
    vad_destroy: unsafe extern "C" fn(Handle) -> () => "SherpaOnnxDestroyVoiceActivityDetector",
    vad_accept: unsafe extern "C" fn(Handle, *const f32, i32) -> () => "SherpaOnnxVoiceActivityDetectorAcceptWaveform",
    vad_empty: unsafe extern "C" fn(Handle) -> i32 => "SherpaOnnxVoiceActivityDetectorEmpty",
    vad_front: unsafe extern "C" fn(Handle) -> *const SherpaOnnxSpeechSegment => "SherpaOnnxVoiceActivityDetectorFront",
    vad_pop: unsafe extern "C" fn(Handle) -> () => "SherpaOnnxVoiceActivityDetectorPop",
    vad_flush: unsafe extern "C" fn(Handle) -> () => "SherpaOnnxVoiceActivityDetectorFlush",
    speech_destroy: unsafe extern "C" fn(Handle) -> () => "SherpaOnnxDestroySpeechSegment",
}
// Each guard borrows the library owner: unload cannot occur before destruction.
struct Owned<'a> {
    pointer: NonNull<c_void>,
    destroy: unsafe extern "C" fn(Handle),
    _api: &'a Api,
}
impl<'a> Owned<'a> {
    fn new(pointer: Handle, destroy: unsafe extern "C" fn(Handle), api: &'a Api) -> Result<Self> {
        Ok(Self {
            pointer: NonNull::new(pointer.cast_mut()).ok_or_else(|| {
                OmmError::new(
                    ErrorCode::Backend,
                    false,
                    "native ASR returned a null handle",
                )
            })?,
            destroy,
            _api: api,
        })
    }
    fn get(&self) -> Handle {
        self.pointer.as_ptr()
    }
}
impl Drop for Owned<'_> {
    fn drop(&mut self) {
        unsafe {
            (self.destroy)(self.get());
        }
    }
}

fn backend(message: &str) -> OmmError {
    OmmError::new(ErrorCode::Backend, false, message)
}
struct DecodeState {
    reader: std::fs::File,
    samples: usize,
    source: AudioSource,
    offset_ms: u64,
    pending: Option<(usize, usize)>,
    previous_end: usize,
    batch: Vec<(usize, usize)>,
    result: Vec<TranscriptSegment>,
    text_bytes: usize,
    progress: Box<dyn FnMut(usize, usize, u64, u64) -> Result<()>>,
    cache: Option<std::path::PathBuf>,
    decoded: usize,
    vad_spans: usize,
    speech_samples: u64,
    started: std::time::Instant,
}
pub(super) struct NativeEngine {
    api: Api,
    recognizer: NonNull<c_void>,
    vad_config: SherpaOnnxVadModelConfig,
    // Keep config strings alive until both engines and streams have been destroyed.
    _strings: Vec<CString>,
}
impl NativeEngine {
    pub(super) fn new(cfg: &AsrConfig) -> Result<Self> {
        if !cfg.library_path.is_absolute() || !cfg.library_path.is_file() {
            return Err(OmmError::unavailable(
                "explicit absolute sherpa-onnx library file is required",
            ));
        }
        if !(1..=64).contains(&cfg.threads) {
            return Err(OmmError::invalid("ASR threads must be 1–64"));
        }
        let paths = [
            &cfg.encoder_path,
            &cfg.decoder_path,
            &cfg.tokens_path,
            &cfg.vad_path,
        ];
        let mut strings = vec![];
        for path in paths {
            let s = path
                .to_str()
                .ok_or_else(|| OmmError::invalid("native model path must be UTF-8"))?;
            strings.push(
                CString::new(s).map_err(|_| OmmError::invalid("native model path contains NUL"))?,
            );
        }
        strings.push(CString::new("cpu").unwrap());
        strings.push(CString::new("greedy_search").unwrap());
        // Header explicitly requires zero-initialization; all fields are C scalars/pointers.
        let mut config: SherpaOnnxOfflineRecognizerConfig = unsafe { std::mem::zeroed() };
        config.feat_config = SherpaOnnxFeatureConfig {
            sample_rate: RATE as i32,
            feature_dim: 80,
        };
        config.model_config.fire_red_asr = SherpaOnnxOfflineFireRedAsrModelConfig {
            encoder: strings[0].as_ptr(),
            decoder: strings[1].as_ptr(),
        };
        config.model_config.tokens = strings[2].as_ptr();
        config.model_config.num_threads = cfg.threads as i32;
        config.model_config.provider = strings[4].as_ptr();
        config.decoding_method = strings[5].as_ptr();
        let mut vad_config: SherpaOnnxVadModelConfig = unsafe { std::mem::zeroed() };
        vad_config.silero_vad = SherpaOnnxSileroVadModelConfig {
            model: strings[3].as_ptr(),
            threshold: 0.5,
            min_silence_duration: 0.7,
            min_speech_duration: 0.25,
            window_size: 512,
            max_speech_duration: 25.0,
        };
        vad_config.sample_rate = RATE as i32;
        vad_config.num_threads = 1;
        vad_config.provider = strings[4].as_ptr();
        let api = unsafe { Api::load(&cfg.library_path)? };
        let recognizer =
            NonNull::new(unsafe { (api.create)(&config) }.cast_mut()).ok_or_else(|| {
                OmmError::new(
                    ErrorCode::Backend,
                    false,
                    "FireRedASR recognizer creation failed",
                )
            })?;
        Ok(Self {
            api,
            recognizer,
            vad_config,
            _strings: strings,
        })
    }
    fn drain(&self, vad: Handle, state: &mut DecodeState) -> Result<()> {
        while unsafe { (self.api.vad_empty)(vad) } == 0 {
            let pointer = unsafe { (self.api.vad_front)(vad) };
            let speech = Owned::new(pointer.cast(), self.api.speech_destroy, &self.api)?;
            let s = unsafe { &*pointer };
            if s.start < 0 || s.n <= 0 || s.n as usize > RATE * 60 || s.samples.is_null() {
                return Err(backend(
                    "native VAD returned invalid or oversized speech bounds",
                ));
            }
            let start = s.start as usize;
            let end = (start + s.n as usize).min(state.samples);
            if start >= end || start < state.previous_end {
                return Err(backend(
                    "native VAD speech bounds are outside input or unordered",
                ));
            }
            state.previous_end = end;
            state.vad_spans += 1;
            drop(speech);
            unsafe {
                (self.api.vad_pop)(vad);
            }
            if let Some((last_start, last_end)) = state.pending {
                if start.saturating_sub(last_end) < RATE / 5 && end - last_start <= RATE * 25 {
                    state.pending = Some((last_start, end));
                    continue;
                }
                state.batch.push((last_start, last_end));
            }
            state.pending = Some((start, end));
            if state.batch.len() == 1 {
                self.decode_batch(state)?;
            }
        }
        Ok(())
    }
    fn decode_batch(&self, state: &mut DecodeState) -> Result<()> {
        use std::io::{Read, Seek, SeekFrom};
        let mut spans = vec![];
        for (start, end) in std::mem::take(&mut state.batch) {
            let cached = state
                .cache
                .as_ref()
                .map(|root| super::chunks::read(root, start, end))
                .transpose()?
                .flatten();
            if let Some(text) = cached {
                Self::append_text(state, start, end, text)?;
            } else {
                spans.push((start, end));
            }
        }
        if spans.is_empty() {
            (state.progress)(
                state.decoded,
                state.vad_spans,
                state.speech_samples,
                state.started.elapsed().as_millis() as u64,
            )?;
            return Ok(());
        }
        let mut order: Vec<usize> = (0..spans.len()).collect();
        order.sort_by_key(|i| std::cmp::Reverse(spans[*i].1 - spans[*i].0));
        let mut waveforms = vec![];
        let mut streams = vec![];
        for &index in &order {
            let (start, end) = spans[index];
            let lo = start.saturating_sub(RATE / 5);
            let hi = end.saturating_add(RATE / 5).min(state.samples);
            let mut bytes = vec![0u8; (hi - lo) * 4];
            state.reader.seek(SeekFrom::Start(lo as u64 * 4))?;
            state.reader.read_exact(&mut bytes)?;
            let samples: Vec<f32> = bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b))
                .collect();
            let stream = Owned::new(
                unsafe { (self.api.stream)(self.recognizer.as_ptr()) },
                self.api.destroy_stream,
                &self.api,
            )?;
            unsafe {
                (self.api.accept)(
                    stream.get(),
                    RATE as i32,
                    samples.as_ptr(),
                    samples.len() as i32,
                );
            }
            waveforms.push(samples);
            streams.push(stream);
        }
        let handles: Vec<Handle> = streams.iter().map(Owned::get).collect();
        (state.progress)(
            state.decoded,
            state.vad_spans,
            state.speech_samples,
            state.started.elapsed().as_millis() as u64,
        )?;
        unsafe {
            (self.api.decode)(
                self.recognizer.as_ptr(),
                handles.as_ptr(),
                handles.len() as i32,
            );
        }
        let mut texts = vec![String::new(); spans.len()];
        for (&index, stream) in order.iter().zip(&streams) {
            let pointer = unsafe { (self.api.result)(stream.get()) };
            let _result = Owned::new(pointer.cast(), self.api.destroy_result, &self.api)?;
            let text = unsafe { (*pointer).text };
            if text.is_null() {
                return Err(backend("native ASR returned null text"));
            }
            let text = unsafe { CStr::from_ptr(text) }
                .to_str()
                .map_err(|_| backend("native ASR returned non-UTF8 text"))?;
            if text.len() > 1024 * 1024 {
                return Err(backend("native ASR segment text exceeds resource limit"));
            }
            texts[index] = text.trim().to_owned();
        }
        for ((start, end), text) in spans.into_iter().zip(texts) {
            if let Some(root) = &state.cache {
                super::chunks::write(root, start, end, &text)?;
            }
            Self::append_text(state, start, end, text)?;
        }
        (state.progress)(
            state.decoded,
            state.vad_spans,
            state.speech_samples,
            state.started.elapsed().as_millis() as u64,
        )?;
        Ok(())
    }
    fn append_text(state: &mut DecodeState, start: usize, end: usize, text: String) -> Result<()> {
        state.text_bytes += text.len() + 128;
        if state.text_bytes > 16 * 1024 * 1024 {
            return Err(backend(
                "native ASR transcript exceeds protocol resource limit",
            ));
        }
        state.decoded += 1;
        state.speech_samples += (end - start) as u64;
        if !text.is_empty() {
            state.result.push(TranscriptSegment {
                source: state.source,
                start_ms: state.offset_ms + (start as u64 * 1000 + 8000) / RATE as u64,
                end_ms: state.offset_ms + (end as u64 * 1000 + 8000) / RATE as u64,
                text,
            });
        }
        Ok(())
    }
    #[cfg(test)]
    pub(super) fn transcribe_file(
        &self,
        path: &Path,
        samples: usize,
        source: AudioSource,
        offset_ms: u64,
    ) -> Result<Vec<TranscriptSegment>> {
        self.transcribe_file_progress(
            path,
            samples,
            source,
            offset_ms,
            None,
            Box::new(|_, _, _, _| Ok(())),
        )
    }
    pub(super) fn transcribe_file_progress(
        &self,
        path: &Path,
        samples: usize,
        source: AudioSource,
        offset_ms: u64,
        cache: Option<std::path::PathBuf>,
        progress: Box<dyn FnMut(usize, usize, u64, u64) -> Result<()>>,
    ) -> Result<Vec<TranscriptSegment>> {
        use std::io::Read;
        let vad = Owned::new(
            unsafe { (self.api.vad_create)(&self.vad_config, 60.0) },
            self.api.vad_destroy,
            &self.api,
        )?;
        let mut input = std::io::BufReader::new(std::fs::File::open(path)?);
        let mut state = DecodeState {
            reader: std::fs::File::open(path)?,
            samples,
            source,
            offset_ms,
            pending: None,
            previous_end: 0,
            batch: vec![],
            result: vec![],
            text_bytes: 0,
            progress,
            cache,
            decoded: 0,
            vad_spans: 0,
            speech_samples: 0,
            started: std::time::Instant::now(),
        };
        for position in (0..samples).step_by(512) {
            // This ABI uses signed 32-bit VAD sample timestamps, not an input-duration policy.
            if position > i32::MAX as usize - 512 {
                return Err(backend("sherpa-onnx 1.13.2 VAD sample index exhausted"));
            }
            let count = (samples - position).min(512);
            let mut bytes = [0u8; 512 * 4];
            input.read_exact(&mut bytes[..count * 4])?;
            let mut window = [0.0f32; 512];
            for (dest, raw) in window
                .iter_mut()
                .zip(bytes[..count * 4].as_chunks::<4>().0.iter())
            {
                *dest = f32::from_le_bytes(*raw);
            }
            unsafe {
                (self.api.vad_accept)(vad.get(), window.as_ptr(), 512);
            }
            self.drain(vad.get(), &mut state)?;
        }
        unsafe {
            (self.api.vad_flush)(vad.get());
        }
        self.drain(vad.get(), &mut state)?;
        if let Some(span) = state.pending.take() {
            state.batch.push(span);
        }
        if !state.batch.is_empty() {
            self.decode_batch(&mut state)?;
        }
        (state.progress)(
            state.decoded,
            state.vad_spans,
            state.speech_samples,
            state.started.elapsed().as_millis() as u64,
        )?;
        state.result.sort_by_key(|s| s.start_ms);
        Ok(state.result)
    }
}
impl Drop for NativeEngine {
    fn drop(&mut self) {
        unsafe {
            (self.api.destroy)(self.recognizer.as_ptr());
        }
    }
}
#[cfg(test)]
mod tests;

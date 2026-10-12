// Layouts transcribed from the pinned v1.13.2 C header. See workers/local-asr/README.md.
#![allow(non_snake_case)]
use std::ffi::c_char;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxFeatureConfig {
    pub sample_rate: i32,
    pub feature_dim: i32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineTransducerModelConfig {
    pub encoder: *const c_char,
    pub decoder: *const c_char,
    pub joiner: *const c_char,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineParaformerModelConfig {
    pub model: *const c_char,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineNemoEncDecCtcModelConfig {
    pub model: *const c_char,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineWhisperModelConfig {
    pub encoder: *const c_char,
    pub decoder: *const c_char,
    pub language: *const c_char,
    pub task: *const c_char,
    pub tail_paddings: i32,
    pub enable_token_timestamps: i32,
    pub enable_segment_timestamps: i32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineTdnnModelConfig {
    pub model: *const c_char,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineSenseVoiceModelConfig {
    pub model: *const c_char,
    pub language: *const c_char,
    pub use_itn: i32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineMoonshineModelConfig {
    pub preprocessor: *const c_char,
    pub encoder: *const c_char,
    pub uncached_decoder: *const c_char,
    pub cached_decoder: *const c_char,
    pub merged_decoder: *const c_char,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineFireRedAsrModelConfig {
    pub encoder: *const c_char,
    pub decoder: *const c_char,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineDolphinModelConfig {
    pub model: *const c_char,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineZipformerCtcModelConfig {
    pub model: *const c_char,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineCanaryModelConfig {
    pub encoder: *const c_char,
    pub decoder: *const c_char,
    pub src_lang: *const c_char,
    pub tgt_lang: *const c_char,
    pub use_pnc: i32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineWenetCtcModelConfig {
    pub model: *const c_char,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineOmnilingualAsrCtcModelConfig {
    pub model: *const c_char,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineMedAsrCtcModelConfig {
    pub model: *const c_char,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineFunASRNanoModelConfig {
    pub encoder_adaptor: *const c_char,
    pub llm: *const c_char,
    pub embedding: *const c_char,
    pub tokenizer: *const c_char,
    pub system_prompt: *const c_char,
    pub user_prompt: *const c_char,
    pub max_new_tokens: i32,
    pub temperature: f32,
    pub top_p: f32,
    pub seed: i32,
    pub language: *const c_char,
    pub itn: i32,
    pub hotwords: *const c_char,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineFireRedAsrCtcModelConfig {
    pub model: *const c_char,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineQwen3ASRModelConfig {
    pub conv_frontend: *const c_char,
    pub encoder: *const c_char,
    pub decoder: *const c_char,
    pub tokenizer: *const c_char,
    pub max_total_len: i32,
    pub max_new_tokens: i32,
    pub temperature: f32,
    pub top_p: f32,
    pub seed: i32,
    pub hotwords: *const c_char,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineCohereTranscribeModelConfig {
    pub encoder: *const c_char,
    pub decoder: *const c_char,
    pub language: *const c_char,
    pub use_punct: i32,
    pub use_itn: i32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineModelConfig {
    pub transducer: SherpaOnnxOfflineTransducerModelConfig,
    pub paraformer: SherpaOnnxOfflineParaformerModelConfig,
    pub nemo_ctc: SherpaOnnxOfflineNemoEncDecCtcModelConfig,
    pub whisper: SherpaOnnxOfflineWhisperModelConfig,
    pub tdnn: SherpaOnnxOfflineTdnnModelConfig,
    pub tokens: *const c_char,
    pub num_threads: i32,
    pub debug: i32,
    pub provider: *const c_char,
    pub model_type: *const c_char,
    pub modeling_unit: *const c_char,
    pub bpe_vocab: *const c_char,
    pub telespeech_ctc: *const c_char,
    pub sense_voice: SherpaOnnxOfflineSenseVoiceModelConfig,
    pub moonshine: SherpaOnnxOfflineMoonshineModelConfig,
    pub fire_red_asr: SherpaOnnxOfflineFireRedAsrModelConfig,
    pub dolphin: SherpaOnnxOfflineDolphinModelConfig,
    pub zipformer_ctc: SherpaOnnxOfflineZipformerCtcModelConfig,
    pub canary: SherpaOnnxOfflineCanaryModelConfig,
    pub wenet_ctc: SherpaOnnxOfflineWenetCtcModelConfig,
    pub omnilingual: SherpaOnnxOfflineOmnilingualAsrCtcModelConfig,
    pub medasr: SherpaOnnxOfflineMedAsrCtcModelConfig,
    pub funasr_nano: SherpaOnnxOfflineFunASRNanoModelConfig,
    pub fire_red_asr_ctc: SherpaOnnxOfflineFireRedAsrCtcModelConfig,
    pub qwen3_asr: SherpaOnnxOfflineQwen3ASRModelConfig,
    pub cohere_transcribe: SherpaOnnxOfflineCohereTranscribeModelConfig,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineLMConfig {
    pub model: *const c_char,
    pub scale: f32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxHomophoneReplacerConfig {
    pub dict_dir: *const c_char,
    pub lexicon: *const c_char,
    pub rule_fsts: *const c_char,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineRecognizerConfig {
    pub feat_config: SherpaOnnxFeatureConfig,
    pub model_config: SherpaOnnxOfflineModelConfig,
    pub lm_config: SherpaOnnxOfflineLMConfig,
    pub decoding_method: *const c_char,
    pub max_active_paths: i32,
    pub hotwords_file: *const c_char,
    pub hotwords_score: f32,
    pub rule_fsts: *const c_char,
    pub rule_fars: *const c_char,
    pub blank_penalty: f32,
    pub hr: SherpaOnnxHomophoneReplacerConfig,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxOfflineRecognizerResult {
    pub text: *const c_char,
    pub timestamps: *mut f32,
    pub count: i32,
    pub tokens: *const c_char,
    pub tokens_arr: *const *const c_char,
    pub json: *const c_char,
    pub lang: *const c_char,
    pub emotion: *const c_char,
    pub event: *const c_char,
    pub durations: *mut f32,
    pub ys_log_probs: *mut f32,
    pub segment_timestamps: *const f32,
    pub segment_durations: *const f32,
    pub segment_texts: *const c_char,
    pub segment_texts_arr: *const *const c_char,
    pub segment_count: i32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxSileroVadModelConfig {
    pub model: *const c_char,
    pub threshold: f32,
    pub min_silence_duration: f32,
    pub min_speech_duration: f32,
    pub window_size: i32,
    pub max_speech_duration: f32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxTenVadModelConfig {
    pub model: *const c_char,
    pub threshold: f32,
    pub min_silence_duration: f32,
    pub min_speech_duration: f32,
    pub window_size: i32,
    pub max_speech_duration: f32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxVadModelConfig {
    pub silero_vad: SherpaOnnxSileroVadModelConfig,
    pub sample_rate: i32,
    pub num_threads: i32,
    pub provider: *const c_char,
    pub debug: i32,
    pub ten_vad: SherpaOnnxTenVadModelConfig,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SherpaOnnxSpeechSegment {
    pub start: i32,
    pub samples: *mut f32,
    pub n: i32,
}

#![cfg(feature = "asr")]
#[path = "../src/adapters/asr/ffi.rs"]
mod ffi;
use ffi::*;
use std::{
    mem::{align_of, offset_of, size_of},
    process::Command,
};
#[test]
fn pinned_header_all_consumed_layouts_match_rust() {
    let temp = tempfile::tempdir().unwrap();
    let exe = temp.path().join("layout");
    assert!(Command::new("cc")
        .args([
            "-std=c11",
            "-Wall",
            "-Werror",
            "tests/asr_fixtures/layout.c",
            "-o"
        ])
        .arg(&exe)
        .status()
        .unwrap()
        .success());
    let out = Command::new(exe).output().unwrap();
    assert!(out.status.success());
    let expected = vec![
        vec![
            size_of::<SherpaOnnxFeatureConfig>(),
            align_of::<SherpaOnnxFeatureConfig>(),
            offset_of!(SherpaOnnxFeatureConfig, sample_rate),
            offset_of!(SherpaOnnxFeatureConfig, feature_dim),
        ],
        vec![
            size_of::<SherpaOnnxOfflineTransducerModelConfig>(),
            align_of::<SherpaOnnxOfflineTransducerModelConfig>(),
            offset_of!(SherpaOnnxOfflineTransducerModelConfig, encoder),
            offset_of!(SherpaOnnxOfflineTransducerModelConfig, decoder),
            offset_of!(SherpaOnnxOfflineTransducerModelConfig, joiner),
        ],
        vec![
            size_of::<SherpaOnnxOfflineParaformerModelConfig>(),
            align_of::<SherpaOnnxOfflineParaformerModelConfig>(),
            offset_of!(SherpaOnnxOfflineParaformerModelConfig, model),
        ],
        vec![
            size_of::<SherpaOnnxOfflineNemoEncDecCtcModelConfig>(),
            align_of::<SherpaOnnxOfflineNemoEncDecCtcModelConfig>(),
            offset_of!(SherpaOnnxOfflineNemoEncDecCtcModelConfig, model),
        ],
        vec![
            size_of::<SherpaOnnxOfflineWhisperModelConfig>(),
            align_of::<SherpaOnnxOfflineWhisperModelConfig>(),
            offset_of!(SherpaOnnxOfflineWhisperModelConfig, encoder),
            offset_of!(SherpaOnnxOfflineWhisperModelConfig, decoder),
            offset_of!(SherpaOnnxOfflineWhisperModelConfig, language),
            offset_of!(SherpaOnnxOfflineWhisperModelConfig, task),
            offset_of!(SherpaOnnxOfflineWhisperModelConfig, tail_paddings),
            offset_of!(SherpaOnnxOfflineWhisperModelConfig, enable_token_timestamps),
            offset_of!(
                SherpaOnnxOfflineWhisperModelConfig,
                enable_segment_timestamps
            ),
        ],
        vec![
            size_of::<SherpaOnnxOfflineTdnnModelConfig>(),
            align_of::<SherpaOnnxOfflineTdnnModelConfig>(),
            offset_of!(SherpaOnnxOfflineTdnnModelConfig, model),
        ],
        vec![
            size_of::<SherpaOnnxOfflineSenseVoiceModelConfig>(),
            align_of::<SherpaOnnxOfflineSenseVoiceModelConfig>(),
            offset_of!(SherpaOnnxOfflineSenseVoiceModelConfig, model),
            offset_of!(SherpaOnnxOfflineSenseVoiceModelConfig, language),
            offset_of!(SherpaOnnxOfflineSenseVoiceModelConfig, use_itn),
        ],
        vec![
            size_of::<SherpaOnnxOfflineMoonshineModelConfig>(),
            align_of::<SherpaOnnxOfflineMoonshineModelConfig>(),
            offset_of!(SherpaOnnxOfflineMoonshineModelConfig, preprocessor),
            offset_of!(SherpaOnnxOfflineMoonshineModelConfig, encoder),
            offset_of!(SherpaOnnxOfflineMoonshineModelConfig, uncached_decoder),
            offset_of!(SherpaOnnxOfflineMoonshineModelConfig, cached_decoder),
            offset_of!(SherpaOnnxOfflineMoonshineModelConfig, merged_decoder),
        ],
        vec![
            size_of::<SherpaOnnxOfflineFireRedAsrModelConfig>(),
            align_of::<SherpaOnnxOfflineFireRedAsrModelConfig>(),
            offset_of!(SherpaOnnxOfflineFireRedAsrModelConfig, encoder),
            offset_of!(SherpaOnnxOfflineFireRedAsrModelConfig, decoder),
        ],
        vec![
            size_of::<SherpaOnnxOfflineDolphinModelConfig>(),
            align_of::<SherpaOnnxOfflineDolphinModelConfig>(),
            offset_of!(SherpaOnnxOfflineDolphinModelConfig, model),
        ],
        vec![
            size_of::<SherpaOnnxOfflineZipformerCtcModelConfig>(),
            align_of::<SherpaOnnxOfflineZipformerCtcModelConfig>(),
            offset_of!(SherpaOnnxOfflineZipformerCtcModelConfig, model),
        ],
        vec![
            size_of::<SherpaOnnxOfflineCanaryModelConfig>(),
            align_of::<SherpaOnnxOfflineCanaryModelConfig>(),
            offset_of!(SherpaOnnxOfflineCanaryModelConfig, encoder),
            offset_of!(SherpaOnnxOfflineCanaryModelConfig, decoder),
            offset_of!(SherpaOnnxOfflineCanaryModelConfig, src_lang),
            offset_of!(SherpaOnnxOfflineCanaryModelConfig, tgt_lang),
            offset_of!(SherpaOnnxOfflineCanaryModelConfig, use_pnc),
        ],
        vec![
            size_of::<SherpaOnnxOfflineWenetCtcModelConfig>(),
            align_of::<SherpaOnnxOfflineWenetCtcModelConfig>(),
            offset_of!(SherpaOnnxOfflineWenetCtcModelConfig, model),
        ],
        vec![
            size_of::<SherpaOnnxOfflineOmnilingualAsrCtcModelConfig>(),
            align_of::<SherpaOnnxOfflineOmnilingualAsrCtcModelConfig>(),
            offset_of!(SherpaOnnxOfflineOmnilingualAsrCtcModelConfig, model),
        ],
        vec![
            size_of::<SherpaOnnxOfflineMedAsrCtcModelConfig>(),
            align_of::<SherpaOnnxOfflineMedAsrCtcModelConfig>(),
            offset_of!(SherpaOnnxOfflineMedAsrCtcModelConfig, model),
        ],
        vec![
            size_of::<SherpaOnnxOfflineFunASRNanoModelConfig>(),
            align_of::<SherpaOnnxOfflineFunASRNanoModelConfig>(),
            offset_of!(SherpaOnnxOfflineFunASRNanoModelConfig, encoder_adaptor),
            offset_of!(SherpaOnnxOfflineFunASRNanoModelConfig, llm),
            offset_of!(SherpaOnnxOfflineFunASRNanoModelConfig, embedding),
            offset_of!(SherpaOnnxOfflineFunASRNanoModelConfig, tokenizer),
            offset_of!(SherpaOnnxOfflineFunASRNanoModelConfig, system_prompt),
            offset_of!(SherpaOnnxOfflineFunASRNanoModelConfig, user_prompt),
            offset_of!(SherpaOnnxOfflineFunASRNanoModelConfig, max_new_tokens),
            offset_of!(SherpaOnnxOfflineFunASRNanoModelConfig, temperature),
            offset_of!(SherpaOnnxOfflineFunASRNanoModelConfig, top_p),
            offset_of!(SherpaOnnxOfflineFunASRNanoModelConfig, seed),
            offset_of!(SherpaOnnxOfflineFunASRNanoModelConfig, language),
            offset_of!(SherpaOnnxOfflineFunASRNanoModelConfig, itn),
            offset_of!(SherpaOnnxOfflineFunASRNanoModelConfig, hotwords),
        ],
        vec![
            size_of::<SherpaOnnxOfflineFireRedAsrCtcModelConfig>(),
            align_of::<SherpaOnnxOfflineFireRedAsrCtcModelConfig>(),
            offset_of!(SherpaOnnxOfflineFireRedAsrCtcModelConfig, model),
        ],
        vec![
            size_of::<SherpaOnnxOfflineQwen3ASRModelConfig>(),
            align_of::<SherpaOnnxOfflineQwen3ASRModelConfig>(),
            offset_of!(SherpaOnnxOfflineQwen3ASRModelConfig, conv_frontend),
            offset_of!(SherpaOnnxOfflineQwen3ASRModelConfig, encoder),
            offset_of!(SherpaOnnxOfflineQwen3ASRModelConfig, decoder),
            offset_of!(SherpaOnnxOfflineQwen3ASRModelConfig, tokenizer),
            offset_of!(SherpaOnnxOfflineQwen3ASRModelConfig, max_total_len),
            offset_of!(SherpaOnnxOfflineQwen3ASRModelConfig, max_new_tokens),
            offset_of!(SherpaOnnxOfflineQwen3ASRModelConfig, temperature),
            offset_of!(SherpaOnnxOfflineQwen3ASRModelConfig, top_p),
            offset_of!(SherpaOnnxOfflineQwen3ASRModelConfig, seed),
            offset_of!(SherpaOnnxOfflineQwen3ASRModelConfig, hotwords),
        ],
        vec![
            size_of::<SherpaOnnxOfflineCohereTranscribeModelConfig>(),
            align_of::<SherpaOnnxOfflineCohereTranscribeModelConfig>(),
            offset_of!(SherpaOnnxOfflineCohereTranscribeModelConfig, encoder),
            offset_of!(SherpaOnnxOfflineCohereTranscribeModelConfig, decoder),
            offset_of!(SherpaOnnxOfflineCohereTranscribeModelConfig, language),
            offset_of!(SherpaOnnxOfflineCohereTranscribeModelConfig, use_punct),
            offset_of!(SherpaOnnxOfflineCohereTranscribeModelConfig, use_itn),
        ],
        vec![
            size_of::<SherpaOnnxOfflineModelConfig>(),
            align_of::<SherpaOnnxOfflineModelConfig>(),
            offset_of!(SherpaOnnxOfflineModelConfig, transducer),
            offset_of!(SherpaOnnxOfflineModelConfig, paraformer),
            offset_of!(SherpaOnnxOfflineModelConfig, nemo_ctc),
            offset_of!(SherpaOnnxOfflineModelConfig, whisper),
            offset_of!(SherpaOnnxOfflineModelConfig, tdnn),
            offset_of!(SherpaOnnxOfflineModelConfig, tokens),
            offset_of!(SherpaOnnxOfflineModelConfig, num_threads),
            offset_of!(SherpaOnnxOfflineModelConfig, debug),
            offset_of!(SherpaOnnxOfflineModelConfig, provider),
            offset_of!(SherpaOnnxOfflineModelConfig, model_type),
            offset_of!(SherpaOnnxOfflineModelConfig, modeling_unit),
            offset_of!(SherpaOnnxOfflineModelConfig, bpe_vocab),
            offset_of!(SherpaOnnxOfflineModelConfig, telespeech_ctc),
            offset_of!(SherpaOnnxOfflineModelConfig, sense_voice),
            offset_of!(SherpaOnnxOfflineModelConfig, moonshine),
            offset_of!(SherpaOnnxOfflineModelConfig, fire_red_asr),
            offset_of!(SherpaOnnxOfflineModelConfig, dolphin),
            offset_of!(SherpaOnnxOfflineModelConfig, zipformer_ctc),
            offset_of!(SherpaOnnxOfflineModelConfig, canary),
            offset_of!(SherpaOnnxOfflineModelConfig, wenet_ctc),
            offset_of!(SherpaOnnxOfflineModelConfig, omnilingual),
            offset_of!(SherpaOnnxOfflineModelConfig, medasr),
            offset_of!(SherpaOnnxOfflineModelConfig, funasr_nano),
            offset_of!(SherpaOnnxOfflineModelConfig, fire_red_asr_ctc),
            offset_of!(SherpaOnnxOfflineModelConfig, qwen3_asr),
            offset_of!(SherpaOnnxOfflineModelConfig, cohere_transcribe),
        ],
        vec![
            size_of::<SherpaOnnxOfflineLMConfig>(),
            align_of::<SherpaOnnxOfflineLMConfig>(),
            offset_of!(SherpaOnnxOfflineLMConfig, model),
            offset_of!(SherpaOnnxOfflineLMConfig, scale),
        ],
        vec![
            size_of::<SherpaOnnxHomophoneReplacerConfig>(),
            align_of::<SherpaOnnxHomophoneReplacerConfig>(),
            offset_of!(SherpaOnnxHomophoneReplacerConfig, dict_dir),
            offset_of!(SherpaOnnxHomophoneReplacerConfig, lexicon),
            offset_of!(SherpaOnnxHomophoneReplacerConfig, rule_fsts),
        ],
        vec![
            size_of::<SherpaOnnxOfflineRecognizerConfig>(),
            align_of::<SherpaOnnxOfflineRecognizerConfig>(),
            offset_of!(SherpaOnnxOfflineRecognizerConfig, feat_config),
            offset_of!(SherpaOnnxOfflineRecognizerConfig, model_config),
            offset_of!(SherpaOnnxOfflineRecognizerConfig, lm_config),
            offset_of!(SherpaOnnxOfflineRecognizerConfig, decoding_method),
            offset_of!(SherpaOnnxOfflineRecognizerConfig, max_active_paths),
            offset_of!(SherpaOnnxOfflineRecognizerConfig, hotwords_file),
            offset_of!(SherpaOnnxOfflineRecognizerConfig, hotwords_score),
            offset_of!(SherpaOnnxOfflineRecognizerConfig, rule_fsts),
            offset_of!(SherpaOnnxOfflineRecognizerConfig, rule_fars),
            offset_of!(SherpaOnnxOfflineRecognizerConfig, blank_penalty),
            offset_of!(SherpaOnnxOfflineRecognizerConfig, hr),
        ],
        vec![
            size_of::<SherpaOnnxOfflineRecognizerResult>(),
            align_of::<SherpaOnnxOfflineRecognizerResult>(),
            offset_of!(SherpaOnnxOfflineRecognizerResult, text),
            offset_of!(SherpaOnnxOfflineRecognizerResult, timestamps),
            offset_of!(SherpaOnnxOfflineRecognizerResult, count),
            offset_of!(SherpaOnnxOfflineRecognizerResult, tokens),
            offset_of!(SherpaOnnxOfflineRecognizerResult, tokens_arr),
            offset_of!(SherpaOnnxOfflineRecognizerResult, json),
            offset_of!(SherpaOnnxOfflineRecognizerResult, lang),
            offset_of!(SherpaOnnxOfflineRecognizerResult, emotion),
            offset_of!(SherpaOnnxOfflineRecognizerResult, event),
            offset_of!(SherpaOnnxOfflineRecognizerResult, durations),
            offset_of!(SherpaOnnxOfflineRecognizerResult, ys_log_probs),
            offset_of!(SherpaOnnxOfflineRecognizerResult, segment_timestamps),
            offset_of!(SherpaOnnxOfflineRecognizerResult, segment_durations),
            offset_of!(SherpaOnnxOfflineRecognizerResult, segment_texts),
            offset_of!(SherpaOnnxOfflineRecognizerResult, segment_texts_arr),
            offset_of!(SherpaOnnxOfflineRecognizerResult, segment_count),
        ],
        vec![
            size_of::<SherpaOnnxSileroVadModelConfig>(),
            align_of::<SherpaOnnxSileroVadModelConfig>(),
            offset_of!(SherpaOnnxSileroVadModelConfig, model),
            offset_of!(SherpaOnnxSileroVadModelConfig, threshold),
            offset_of!(SherpaOnnxSileroVadModelConfig, min_silence_duration),
            offset_of!(SherpaOnnxSileroVadModelConfig, min_speech_duration),
            offset_of!(SherpaOnnxSileroVadModelConfig, window_size),
            offset_of!(SherpaOnnxSileroVadModelConfig, max_speech_duration),
        ],
        vec![
            size_of::<SherpaOnnxTenVadModelConfig>(),
            align_of::<SherpaOnnxTenVadModelConfig>(),
            offset_of!(SherpaOnnxTenVadModelConfig, model),
            offset_of!(SherpaOnnxTenVadModelConfig, threshold),
            offset_of!(SherpaOnnxTenVadModelConfig, min_silence_duration),
            offset_of!(SherpaOnnxTenVadModelConfig, min_speech_duration),
            offset_of!(SherpaOnnxTenVadModelConfig, window_size),
            offset_of!(SherpaOnnxTenVadModelConfig, max_speech_duration),
        ],
        vec![
            size_of::<SherpaOnnxVadModelConfig>(),
            align_of::<SherpaOnnxVadModelConfig>(),
            offset_of!(SherpaOnnxVadModelConfig, silero_vad),
            offset_of!(SherpaOnnxVadModelConfig, sample_rate),
            offset_of!(SherpaOnnxVadModelConfig, num_threads),
            offset_of!(SherpaOnnxVadModelConfig, provider),
            offset_of!(SherpaOnnxVadModelConfig, debug),
            offset_of!(SherpaOnnxVadModelConfig, ten_vad),
        ],
        vec![
            size_of::<SherpaOnnxSpeechSegment>(),
            align_of::<SherpaOnnxSpeechSegment>(),
            offset_of!(SherpaOnnxSpeechSegment, start),
            offset_of!(SherpaOnnxSpeechSegment, samples),
            offset_of!(SherpaOnnxSpeechSegment, n),
        ],
    ];
    let actual: Vec<Vec<usize>> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|l| l.split_whitespace().map(|n| n.parse().unwrap()).collect())
        .collect();
    assert_eq!(actual, expected);
}

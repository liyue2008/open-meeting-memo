/* Controlled C ABI fixture, NOT an inference engine. Compiles against official 1.13.2 header. */
#include "../../workers/local-asr/include/c-api.h"
#include <assert.h>
#include <stdlib.h>
#include <string.h>
static int mode, active[5], accepts, decodes, last_input;
void FixtureMode(int m) { mode = m; }
int FixtureActive(int i) { return active[i]; }
int FixtureLastInput(void) { return last_input; }
int FixtureAccepts(void) { return accepts; }
int FixtureDecodes(void) { return decodes; }
const char *SherpaOnnxGetVersionStr(void) { return mode == 5 ? "1.13.1" : "1.13.2"; }
struct SherpaOnnxOfflineRecognizer { int marker; };
struct SherpaOnnxOfflineStream { int n; };
struct SherpaOnnxVoiceActivityDetector { int total, queued, popped; };
const SherpaOnnxOfflineRecognizer *SherpaOnnxCreateOfflineRecognizer(const SherpaOnnxOfflineRecognizerConfig *c) {
 assert(c->feat_config.sample_rate == 16000 && c->feat_config.feature_dim == 80);
 assert(c->model_config.fire_red_asr.encoder && c->model_config.fire_red_asr.decoder && c->model_config.tokens);
 assert(!c->model_config.whisper.encoder && !c->model_config.transducer.encoder);
 assert(c->model_config.num_threads == 2 && !strcmp(c->model_config.provider,"cpu"));
 assert(!strcmp(c->decoding_method,"greedy_search"));
 if (mode == 6) return NULL;
 ++active[0]; return calloc(1,sizeof(SherpaOnnxOfflineRecognizer));
}
void SherpaOnnxDestroyOfflineRecognizer(const SherpaOnnxOfflineRecognizer *r) { --active[0]; free((void*)r); }
const SherpaOnnxOfflineStream *SherpaOnnxCreateOfflineStream(const SherpaOnnxOfflineRecognizer *r) {
 assert(r); if (mode == 3) return NULL; ++active[1]; return calloc(1,sizeof(SherpaOnnxOfflineStream));
}
void SherpaOnnxDestroyOfflineStream(const SherpaOnnxOfflineStream *s) { --active[1]; free((void*)s); }
void SherpaOnnxAcceptWaveformOffline(const SherpaOnnxOfflineStream *s, int32_t rate, const float *p, int32_t n) {
 assert(s && p && rate == 16000 && n > 0); ((SherpaOnnxOfflineStream*)s)->n=n; last_input=n; ++accepts;
}
void SherpaOnnxDecodeMultipleOfflineStreams(const SherpaOnnxOfflineRecognizer *r, const SherpaOnnxOfflineStream **s, int32_t n) {
 assert(r && n > 0 && n <= 8); for (int i=0; i<n; ++i) assert(s[i]->n>0); ++decodes;
}
const SherpaOnnxOfflineRecognizerResult *SherpaOnnxGetOfflineStreamResult(const SherpaOnnxOfflineStream *s) {
 assert(s); if (mode == 1) return NULL;
 SherpaOnnxOfflineRecognizerResult *r=calloc(1,sizeof(*r)); ++active[3];
 r->text = mode == 4 ? NULL : "测试 meeting"; return r;
}
void SherpaOnnxDestroyOfflineRecognizerResult(const SherpaOnnxOfflineRecognizerResult *r) { --active[3]; free((void*)r); }
const SherpaOnnxVoiceActivityDetector *SherpaOnnxCreateVoiceActivityDetector(const SherpaOnnxVadModelConfig *c, float buffer) {
 assert(c->sample_rate==16000 && c->silero_vad.window_size==512 && c->silero_vad.model);
 assert(c->silero_vad.threshold==0.5f && c->silero_vad.max_speech_duration==25.0f);
 assert(c->silero_vad.min_silence_duration==0.7f && c->silero_vad.min_speech_duration==0.25f);
 assert(!c->ten_vad.model && !strcmp(c->provider,"cpu") && buffer==60.0f);
 if(mode==2) return NULL; ++active[2]; return calloc(1,sizeof(SherpaOnnxVoiceActivityDetector));
}
void SherpaOnnxDestroyVoiceActivityDetector(const SherpaOnnxVoiceActivityDetector *v) { --active[2]; free((void*)v); }
void SherpaOnnxVoiceActivityDetectorAcceptWaveform(const SherpaOnnxVoiceActivityDetector *v, const float *p, int32_t n) {
 assert(p && n==512); ((SherpaOnnxVoiceActivityDetector*)v)->total += n;
 if(mode==9 && v->total>=36000) ((SherpaOnnxVoiceActivityDetector*)v)->queued = v->total>=40000 ? 2 : 1;
}
int32_t SherpaOnnxVoiceActivityDetectorEmpty(const SherpaOnnxVoiceActivityDetector *v) { return v->queued==v->popped; }
void SherpaOnnxVoiceActivityDetectorFlush(const SherpaOnnxVoiceActivityDetector *v) { if(mode!=9) ((SherpaOnnxVoiceActivityDetector*)v)->queued=mode==7 ? 10 : 1; }
const SherpaOnnxSpeechSegment *SherpaOnnxVoiceActivityDetectorFront(const SherpaOnnxVoiceActivityDetector *v) {
 assert(v->queued>v->popped); SherpaOnnxSpeechSegment *s=calloc(1,sizeof(*s));
 s->start=mode==9 ? (v->popped ? 36000 : 32700) : (mode==7 ? 1600+6400*v->popped : 1600); s->n=mode==7 ? 1600 : 3200; s->samples=calloc(s->n,sizeof(float)); ++active[4]; return s;
}
void SherpaOnnxDestroySpeechSegment(const SherpaOnnxSpeechSegment *s) { --active[4]; free(s->samples); free((void*)s); }
void SherpaOnnxVoiceActivityDetectorPop(const SherpaOnnxVoiceActivityDetector *v) { ++((SherpaOnnxVoiceActivityDetector*)v)->popped; }

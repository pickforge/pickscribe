// A small C surface over whisper.cpp for Dart FFI. It mirrors src/transcribe.rs on the
// desktop: pick the most likely configured language, then decode with the glossary prompt.

#include <atomic>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <string>
#include <vector>

#include "whisper.h"

#define PS_API extern "C" __attribute__((visibility("default")))

// A loaded model with the state that every dictation reuses, so the compute buffers are
// allocated once. `abort` stops a transcription in progress.
struct Model {
    whisper_context *ctx;
    whisper_state *state;
    std::atomic<bool> abort{false};
};

namespace {

std::vector<std::string> split_codes(const char *list) {
    std::vector<std::string> codes;
    std::string current;
    for (const char *c = list; c != nullptr && *c != '\0'; ++c) {
        if (*c == ',') {
            if (!current.empty()) codes.push_back(current);
            current.clear();
        } else {
            current.push_back(*c);
        }
    }
    if (!current.empty()) codes.push_back(current);
    return codes;
}

// Free detection over all languages mislabels short Portuguese clips as Spanish or Galician
// too often, so only the configured languages compete.
const char *pick_language(whisper_context *ctx, whisper_state *state, const float *pcm,
                          int32_t n_samples, const std::vector<std::string> &codes,
                          int32_t threads) {
    if (codes.empty()) return "auto";
    if (codes.size() == 1) return whisper_lang_str(whisper_lang_id(codes[0].c_str()));
    if (whisper_pcm_to_mel_with_state(ctx, state, pcm, n_samples, threads) != 0) return nullptr;
    std::vector<float> probabilities(whisper_lang_max_id() + 1, 0.0f);
    if (whisper_lang_auto_detect_with_state(ctx, state, 0, threads, probabilities.data()) < 0) {
        return nullptr;
    }
    int best = -1;
    for (const auto &code : codes) {
        const int id = whisper_lang_id(code.c_str());
        if (id >= 0 && (best < 0 || probabilities[id] > probabilities[best])) best = id;
    }
    return best < 0 ? "auto" : whisper_lang_str(best);
}

bool should_abort(void *model) {
    return static_cast<Model *>(model)->abort.load(std::memory_order_relaxed);
}

char *copy_string(const std::string &text) {
    char *out = static_cast<char *>(std::malloc(text.size() + 1));
    if (out != nullptr) std::memcpy(out, text.c_str(), text.size() + 1);
    return out;
}

}  // namespace

PS_API Model *ps_load(const char *model_path) {
    whisper_context_params params = whisper_context_default_params();
    params.use_gpu = false;
    whisper_context *ctx = whisper_init_from_file_with_params_no_state(model_path, params);
    if (ctx == nullptr) return nullptr;
    whisper_state *state = whisper_init_state(ctx);
    if (state == nullptr) {
        whisper_free(ctx);
        return nullptr;
    }
    return new Model{ctx, state};
}

PS_API void ps_unload(Model *model) {
    whisper_free_state(model->state);
    whisper_free(model->ctx);
    delete model;
}

// Any thread may call this. Set it before a transcription starts and clear it before the next
// one; a transcription that sees it returns 5.
PS_API void ps_set_abort(Model *model, int32_t abort) {
    model->abort.store(abort != 0, std::memory_order_relaxed);
}

// Transcribes 16 kHz mono audio. `languages` is a comma-separated list of codes, and
// `prompt` may be empty. A beam size of 1 or less decodes greedily. On success, returns 0,
// stores the text in `*out_text`, to free with ps_free_string, and the language code in
// `*out_language`, which needs no freeing.
PS_API int32_t ps_transcribe(Model *model, const float *pcm, int32_t n_samples,
                             const char *languages, const char *prompt, int32_t threads,
                             int32_t beam_size, char **out_text, const char **out_language) {
    whisper_context *ctx = model->ctx;
    whisper_state *state = model->state;
    const char *language =
        pick_language(ctx, state, pcm, n_samples, split_codes(languages), threads);
    if (language == nullptr) return 2;
    if (should_abort(model)) return 5;
    whisper_full_params params = whisper_full_default_params(
        beam_size > 1 ? WHISPER_SAMPLING_BEAM_SEARCH : WHISPER_SAMPLING_GREEDY);
    if (beam_size > 1) {
        params.beam_search.beam_size = beam_size;
        params.beam_search.patience = -1.0f;
    }
    params.language = language;
    params.initial_prompt = (prompt != nullptr && *prompt != '\0') ? prompt : nullptr;
    params.n_threads = threads;
    params.no_context = true;
    params.no_timestamps = true;
    params.suppress_blank = true;
    params.suppress_nst = true;
    params.print_special = false;
    params.print_progress = false;
    params.print_realtime = false;
    params.print_timestamps = false;
    params.abort_callback = should_abort;
    params.abort_callback_user_data = model;
    if (whisper_full_with_state(ctx, state, params, pcm, n_samples) != 0) {
        return should_abort(model) ? 5 : 3;
    }
    std::string text;
    const int segments = whisper_full_n_segments_from_state(state);
    for (int i = 0; i < segments; ++i) {
        text += whisper_full_get_segment_text_from_state(state, i);
    }
    *out_text = copy_string(text);
    *out_language = language;
    return *out_text == nullptr ? 4 : 0;
}

PS_API void ps_free_string(char *text) { std::free(text); }

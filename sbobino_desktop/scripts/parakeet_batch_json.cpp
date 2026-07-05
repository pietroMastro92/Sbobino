#include "parakeet_capi.h"

#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <iostream>
#include <sstream>
#include <string>
#include <vector>

struct Chunk {
    int index = 0;
    double decode_start = 0.0;
    double decode_end = 0.0;
    double commit_start = 0.0;
    double commit_end = 0.0;
    std::string path;
};

static void usage() {
    std::fprintf(stderr,
                 "usage: parakeet-batch-json --model <model.gguf> --manifest <chunks.tsv> [--lang <locale>]\n");
}

static std::uint16_t read_u16(std::istream& in) {
    unsigned char bytes[2]{};
    in.read(reinterpret_cast<char*>(bytes), 2);
    return static_cast<std::uint16_t>(bytes[0]) |
           (static_cast<std::uint16_t>(bytes[1]) << 8);
}

static std::uint32_t read_u32(std::istream& in) {
    unsigned char bytes[4]{};
    in.read(reinterpret_cast<char*>(bytes), 4);
    return static_cast<std::uint32_t>(bytes[0]) |
           (static_cast<std::uint32_t>(bytes[1]) << 8) |
           (static_cast<std::uint32_t>(bytes[2]) << 16) |
           (static_cast<std::uint32_t>(bytes[3]) << 24);
}

static bool read_pcm16_mono_wav(const std::string& path, std::vector<float>& samples,
                                int& sample_rate) {
    std::ifstream in(path, std::ios::binary);
    char riff[4]{}, wave[4]{};
    in.read(riff, 4);
    (void) read_u32(in);
    in.read(wave, 4);
    if (!in || std::memcmp(riff, "RIFF", 4) != 0 || std::memcmp(wave, "WAVE", 4) != 0) {
        return false;
    }

    std::uint16_t format = 0, channels = 0, bits = 0;
    std::vector<char> pcm;
    while (in && (format == 0 || pcm.empty())) {
        char id[4]{};
        in.read(id, 4);
        if (!in) {
            break;
        }
        const std::uint32_t size = read_u32(in);
        if (std::memcmp(id, "fmt ", 4) == 0 && size >= 16) {
            format = read_u16(in);
            channels = read_u16(in);
            sample_rate = static_cast<int>(read_u32(in));
            (void) read_u32(in);
            (void) read_u16(in);
            bits = read_u16(in);
            in.seekg(static_cast<std::streamoff>(size - 16), std::ios::cur);
        } else if (std::memcmp(id, "data", 4) == 0) {
            pcm.resize(size);
            in.read(pcm.data(), static_cast<std::streamsize>(size));
        } else {
            in.seekg(static_cast<std::streamoff>(size), std::ios::cur);
        }
        if (size & 1U) {
            in.seekg(1, std::ios::cur);
        }
    }
    if (format != 1 || channels != 1 || bits != 16 || sample_rate <= 0 || pcm.empty()) {
        return false;
    }

    samples.reserve(pcm.size() / 2);
    for (std::size_t i = 0; i + 1 < pcm.size(); i += 2) {
        const auto lo = static_cast<unsigned char>(pcm[i]);
        const auto hi = static_cast<unsigned char>(pcm[i + 1]);
        const auto value = static_cast<std::int16_t>(
            static_cast<std::uint16_t>(lo) | (static_cast<std::uint16_t>(hi) << 8));
        samples.push_back(static_cast<float>(value) / 32768.0f);
    }
    return true;
}

static bool read_manifest(const std::string& path, std::vector<Chunk>& chunks) {
    std::ifstream in(path);
    if (!in) {
        return false;
    }

    std::string line;
    while (std::getline(in, line)) {
        if (line.empty()) {
            continue;
        }
        std::stringstream ss(line);
        std::string index_s, decode_start_s, decode_end_s, commit_start_s, commit_end_s, chunk_path;
        if (!std::getline(ss, index_s, '\t')) return false;
        if (!std::getline(ss, decode_start_s, '\t')) return false;
        if (!std::getline(ss, decode_end_s, '\t')) return false;
        if (!std::getline(ss, commit_start_s, '\t')) return false;
        if (!std::getline(ss, commit_end_s, '\t')) return false;
        if (!std::getline(ss, chunk_path)) return false;

        Chunk chunk;
        chunk.index = std::atoi(index_s.c_str());
        chunk.decode_start = std::atof(decode_start_s.c_str());
        chunk.decode_end = std::atof(decode_end_s.c_str());
        chunk.commit_start = std::atof(commit_start_s.c_str());
        chunk.commit_end = std::atof(commit_end_s.c_str());
        chunk.path = chunk_path;
        chunks.push_back(chunk);
    }
    return true;
}

static std::string unwrap_singleton_json_array(const std::string& batch_json) {
    if (batch_json.size() < 2 || batch_json.front() != '[' || batch_json.back() != ']') {
        return "";
    }
    return batch_json.substr(1, batch_json.size() - 2);
}

static void emit_event(const char* phase, int index, const std::string& message) {
    std::fprintf(stderr,
                 "SBOBINO_PARAKEET_WORKER {\"phase\":\"%s\",\"index\":%d,\"message\":\"%s\"}\n",
                 phase,
                 index,
                 message.c_str());
    std::fflush(stderr);
}

int main(int argc, char** argv) {
    std::string model;
    std::string manifest;
    std::string target_lang = "auto";

    for (int i = 1; i < argc; ++i) {
        if (std::strcmp(argv[i], "--model") == 0 && i + 1 < argc) {
            model = argv[++i];
        } else if (std::strcmp(argv[i], "--manifest") == 0 && i + 1 < argc) {
            manifest = argv[++i];
        } else if (std::strcmp(argv[i], "--lang") == 0 && i + 1 < argc) {
            target_lang = argv[++i];
        } else {
            usage();
            return 2;
        }
    }
    if (model.empty() || manifest.empty()) {
        usage();
        return 2;
    }

    std::vector<Chunk> chunks;
    if (!read_manifest(manifest, chunks) || chunks.empty()) {
        std::fprintf(stderr, "parakeet-batch-json: failed to read manifest %s\n", manifest.c_str());
        return 1;
    }

    emit_event("loading_model", -1, "Loading Parakeet model");
    std::fprintf(stderr, "parakeet-batch-json: loading model %s\n", model.c_str());
    parakeet_ctx* ctx = parakeet_capi_load(model.c_str());
    if (!ctx) {
        emit_event("failed", -1, "Failed to load Parakeet model");
        std::fprintf(stderr, "parakeet-batch-json: failed to load model %s\n", model.c_str());
        return 1;
    }
    emit_event("model_ready", -1, "Parakeet model ready");

    int expected_sample_rate = 0;
    for (const auto& chunk : chunks) {
        emit_event("chunk_started", chunk.index, "Processing Parakeet chunk");
        std::vector<float> samples;
        int sample_rate = 0;
        if (!read_pcm16_mono_wav(chunk.path, samples, sample_rate)) {
            emit_event("failed", chunk.index, "Chunk is not mono PCM16 WAV");
            std::fprintf(stderr, "parakeet-batch-json: chunk is not mono PCM16 WAV: %s\n",
                         chunk.path.c_str());
            parakeet_capi_free(ctx);
            return 1;
        }
        if (expected_sample_rate == 0) {
            expected_sample_rate = sample_rate;
        } else if (sample_rate != expected_sample_rate) {
            emit_event("failed", chunk.index, "Chunk sample rate is inconsistent");
            std::fprintf(stderr,
                         "parakeet-batch-json: inconsistent sample rate in chunk %d: %d != %d\n",
                         chunk.index, sample_rate, expected_sample_rate);
            parakeet_capi_free(ctx);
            return 1;
        }

        int sample_count = static_cast<int>(samples.size());
        char* json = parakeet_capi_transcribe_pcm_batch_json_lang(
            ctx, samples.data(), &sample_count, 1, sample_rate, 0, target_lang.c_str());
        if (!json) {
            emit_event("failed", chunk.index, "Parakeet chunk failed");
            std::fprintf(stderr, "parakeet-batch-json: chunk %d failed: %s\n",
                         chunk.index, parakeet_capi_last_error(ctx));
            parakeet_capi_free(ctx);
            return 1;
        }

        const std::string batch_json(json);
        const std::string result_json = unwrap_singleton_json_array(batch_json);
        parakeet_capi_free_string(json);
        if (result_json.empty()) {
            emit_event("failed", chunk.index, "Chunk returned malformed JSON");
            std::fprintf(stderr, "parakeet-batch-json: chunk %d returned malformed batch JSON\n",
                         chunk.index);
            parakeet_capi_free(ctx);
            return 1;
        }

        std::printf(
            "{\"index\":%d,\"decode_start\":%.3f,\"decode_end\":%.3f,\"commit_start\":%.3f,\"commit_end\":%.3f,\"result\":%s}\n",
            chunk.index,
            chunk.decode_start,
            chunk.decode_end,
            chunk.commit_start,
            chunk.commit_end,
            result_json.c_str());
        std::fflush(stdout);
        emit_event("chunk_completed", chunk.index, "Parakeet chunk completed");
    }

    parakeet_capi_free(ctx);
    return 0;
}

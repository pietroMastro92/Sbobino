#include "parakeet_capi.h"

#include <array>
#include <cerrno>
#include <charconv>
#include <cctype>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <iostream>
#include <string>
#include <system_error>
#include <vector>

static constexpr double kMaxSerializedDecodeSeconds = 45.0;
static constexpr double kManifestTimestampToleranceSeconds = 0.01;

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
                 "usage: parakeet-batch-json --model <model.gguf> --manifest <chunks.tsv> [--lang <locale>] [--threads N]\n");
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

static bool has_whitespace(const std::string& value) {
    for (unsigned char character : value) {
        if (std::isspace(character)) {
            return true;
        }
    }
    return false;
}

static bool parse_strict_index(const std::string& value, int& index) {
    if (value.empty() || has_whitespace(value)) {
        return false;
    }
    int parsed = 0;
    const char* begin = value.data();
    const char* end = begin + value.size();
    const auto result = std::from_chars(begin, end, parsed);
    if (result.ec != std::errc{} || result.ptr != end || parsed < 0) {
        return false;
    }
    index = parsed;
    return true;
}

static bool parse_strict_finite_double(const std::string& value, double& output) {
    if (value.empty() || has_whitespace(value)) {
        return false;
    }
    errno = 0;
    char* end = nullptr;
    const double parsed = std::strtod(value.c_str(), &end);
    if (errno == ERANGE || end != value.c_str() + value.size() || !std::isfinite(parsed)) {
        return false;
    }
    output = parsed;
    return true;
}

static bool split_manifest_row(const std::string& line, std::array<std::string, 6>& fields) {
    std::size_t start = 0;
    for (std::size_t index = 0; index < fields.size() - 1; ++index) {
        const std::size_t tab = line.find('\t', start);
        if (tab == std::string::npos) {
            return false;
        }
        fields[index] = line.substr(start, tab - start);
        start = tab + 1;
    }
    fields.back() = line.substr(start);
    return fields.back().find('\t') == std::string::npos;
}

static bool read_manifest(const std::string& path, std::vector<Chunk>& chunks,
                          std::string& failure_reason) {
    std::ifstream in(path);
    if (!in) {
        failure_reason = "manifest cannot be opened";
        return false;
    }

    std::string line;
    std::size_t line_number = 0;
    while (std::getline(in, line)) {
        ++line_number;
        if (line.empty()) {
            failure_reason = "manifest contains an empty row at line " + std::to_string(line_number);
            return false;
        }

        std::array<std::string, 6> fields;
        if (!split_manifest_row(line, fields) || fields.back().empty()) {
            failure_reason = "manifest row " + std::to_string(line_number) +
                             " must contain exactly six non-empty TSV fields";
            return false;
        }

        Chunk chunk;
        if (!parse_strict_index(fields[0], chunk.index) ||
            !parse_strict_finite_double(fields[1], chunk.decode_start) ||
            !parse_strict_finite_double(fields[2], chunk.decode_end) ||
            !parse_strict_finite_double(fields[3], chunk.commit_start) ||
            !parse_strict_finite_double(fields[4], chunk.commit_end)) {
            failure_reason = "manifest row " + std::to_string(line_number) +
                             " has an invalid finite index or timestamp";
            return false;
        }
        chunk.path = fields[5];

        if (chunk.decode_start < 0.0 ||
            chunk.commit_start < 0.0 ||
            chunk.decode_end < chunk.decode_start ||
            chunk.commit_end <= chunk.commit_start ||
            chunk.commit_start < chunk.decode_start ||
            chunk.commit_end > chunk.decode_end ||
            chunk.decode_end - chunk.decode_start > kMaxSerializedDecodeSeconds) {
            failure_reason = "manifest row " + std::to_string(line_number) +
                             " violates bounded decode/commit ordering";
            return false;
        }
        if (chunk.index != static_cast<int>(chunks.size())) {
            failure_reason = "manifest row " + std::to_string(line_number) +
                             " has a duplicate, skipped, or non-monotonic index";
            return false;
        }
        // A retry manifest may start at the first uncovered global commit
        // edge rather than at t=0. The parent has already persisted the
        // preceding rows, so require contiguity from this row onward instead
        // of rejecting every resumed attempt. The non-negative bounds above
        // still prevent malformed offsets; subsequent rows must remain
        // contiguous and monotonic within the timestamp tolerance.
        if (!chunks.empty()) {
            const Chunk& previous = chunks.back();
            if (std::fabs(chunk.commit_start - previous.commit_end) >
                    kManifestTimestampToleranceSeconds ||
                chunk.decode_start + kManifestTimestampToleranceSeconds < previous.decode_start ||
                chunk.decode_end + kManifestTimestampToleranceSeconds < previous.decode_end) {
                failure_reason = "manifest row " + std::to_string(line_number) +
                                 " is not contiguous and monotonic";
                return false;
            }
        }
        chunks.push_back(chunk);
    }
    if (chunks.empty()) {
        failure_reason = "manifest has no chunk rows";
        return false;
    }
    return true;
}

static std::string unwrap_singleton_json_array(const std::string& batch_json) {
    if (batch_json.size() < 2 || batch_json.front() != '[' || batch_json.back() != ']') {
        return "";
    }
    return batch_json.substr(1, batch_json.size() - 2);
}

static bool is_auto_target_lang(const std::string& target_lang) {
    return target_lang.empty() || target_lang == "auto";
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
    int threads = 4;

    for (int i = 1; i < argc; ++i) {
        if (std::strcmp(argv[i], "--model") == 0 && i + 1 < argc) {
            model = argv[++i];
        } else if (std::strcmp(argv[i], "--manifest") == 0 && i + 1 < argc) {
            manifest = argv[++i];
        } else if (std::strcmp(argv[i], "--lang") == 0 && i + 1 < argc) {
            target_lang = argv[++i];
        } else if (std::strcmp(argv[i], "--threads") == 0 && i + 1 < argc) {
            if (!parse_strict_index(argv[++i], threads) || threads < 1 || threads > 8) {
                usage();
                return 2;
            }
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
    std::string manifest_failure;
    if (!read_manifest(manifest, chunks, manifest_failure)) {
        std::fprintf(stderr, "parakeet-batch-json: rejected manifest %s before model load: %s\n",
                     manifest.c_str(), manifest_failure.c_str());
        return 1;
    }

    emit_event("loading_model", -1, "Loading Parakeet model");
    std::fprintf(stderr, "parakeet-batch-json: loading model %s\n", model.c_str());
    parakeet_capi_set_num_threads(threads);
    parakeet_ctx* ctx = parakeet_capi_load(model.c_str());
    if (!ctx) {
        emit_event("failed", -1, "Failed to load Parakeet model");
        std::fprintf(stderr, "parakeet-batch-json: failed to load model %s\n", model.c_str());
        return 1;
    }
    emit_event("model_ready", -1, "Parakeet model ready");

    const bool auto_target_lang = is_auto_target_lang(target_lang);
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

        char* json = nullptr;
        bool result_is_batch = false;
        if (auto_target_lang) {
            // Nemotron's language-aware PCM batch path can return an empty
            // result for an otherwise voiced chunk. The path JSON API uses
            // the same loaded model and the same WAV bytes as parakeet-cli,
            // while retaining timestamps and confidence in the JSON result.
            json = parakeet_capi_transcribe_path_json(ctx, chunk.path.c_str(), 0);
        } else {
            int sample_count = static_cast<int>(samples.size());
            json = parakeet_capi_transcribe_pcm_batch_json_lang(
                ctx, samples.data(), &sample_count, 1, sample_rate, 0, target_lang.c_str());
            result_is_batch = true;
        }
        if (!json) {
            emit_event("failed", chunk.index, "Parakeet chunk failed");
            const char* last_error = parakeet_capi_last_error(ctx);
            std::fprintf(stderr, "parakeet-batch-json: chunk %d failed: %s\n", chunk.index,
                         last_error ? last_error : "unknown Parakeet error");
            parakeet_capi_free(ctx);
            return 1;
        }

        const std::string serialized_json(json);
        parakeet_capi_free_string(json);
        const std::string result_json = result_is_batch
            ? unwrap_singleton_json_array(serialized_json)
            : serialized_json;
        if (result_json.empty()) {
            emit_event("failed", chunk.index, "Chunk returned malformed JSON");
            std::fprintf(stderr, "parakeet-batch-json: chunk %d returned malformed JSON\n",
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

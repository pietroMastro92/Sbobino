#include "parakeet_capi.h"

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
    double start = 0.0;
    double end = 0.0;
    std::string path;
};

static void usage() {
    std::fprintf(stderr, "usage: parakeet-batch-json --model <model.gguf> --manifest <chunks.tsv>\n");
}

static bool read_manifest(const std::string& path, std::vector<Chunk>& chunks) {
    std::ifstream in(path);
    if (!in) return false;
    std::string line;
    while (std::getline(in, line)) {
        if (line.empty()) continue;
        std::stringstream ss(line);
        std::string index_s, start_s, end_s, chunk_path;
        if (!std::getline(ss, index_s, '\t')) return false;
        if (!std::getline(ss, start_s, '\t')) return false;
        if (!std::getline(ss, end_s, '\t')) return false;
        if (!std::getline(ss, chunk_path)) return false;
        Chunk c;
        c.index = std::atoi(index_s.c_str());
        c.start = std::atof(start_s.c_str());
        c.end = std::atof(end_s.c_str());
        c.path = chunk_path;
        chunks.push_back(c);
    }
    return true;
}

int main(int argc, char** argv) {
    std::string model;
    std::string manifest;
    for (int i = 1; i < argc; ++i) {
        if (std::strcmp(argv[i], "--model") == 0 && i + 1 < argc) {
            model = argv[++i];
        } else if (std::strcmp(argv[i], "--manifest") == 0 && i + 1 < argc) {
            manifest = argv[++i];
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

    parakeet_ctx* ctx = parakeet_capi_load(model.c_str());
    if (!ctx) {
        std::fprintf(stderr, "parakeet-batch-json: failed to load model %s\n", model.c_str());
        return 1;
    }

    for (const Chunk& chunk : chunks) {
        char* json = parakeet_capi_transcribe_path_json(ctx, chunk.path.c_str(), 0);
        if (!json) {
            std::fprintf(stderr, "parakeet-batch-json: chunk %d failed: %s\n", chunk.index, parakeet_capi_last_error(ctx));
            parakeet_capi_free(ctx);
            return 1;
        }
        std::printf("{\"index\":%d,\"start\":%.3f,\"end\":%.3f,\"result\":%s}\n", chunk.index, chunk.start, chunk.end, json);
        std::fflush(stdout);
        parakeet_capi_free_string(json);
    }

    parakeet_capi_free(ctx);
    return 0;
}

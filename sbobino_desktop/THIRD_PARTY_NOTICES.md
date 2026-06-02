# Third-party software and data

This document lists major open-source components and data that **Sbobino Desktop** may **ship with** or **download and install on first launch** (for example FFmpeg, whisper.cpp binaries, the pyannote Python runtime, and diarization model files). It is provided for **attribution and transparency**. It is **not legal advice**. If you redistribute binaries or models, you are responsible for complying with each upstream license (and for any patent or export rules that may apply in your jurisdiction).

- **Application source code** for this workspace is under the MIT License; see [`LICENSE`](LICENSE).
- **Authoritative license texts** for third parties are always those published by each upstream project.

## Build inventory (maintainers)

Versions below match the **default** values in the packaging scripts at the time this file was last reviewed. Override environment variables are noted where they exist.

| Artifact | Default version / pin | Source in this repo |
|----------|------------------------|---------------------|
| **whisper.cpp** (`whisper-cli`, `whisper-stream`) | `1.8.4` (`SBOBINO_RUNTIME_WHISPER_CPP_VERSION`) | [`scripts/package_macos_runtime_asset.sh`](scripts/package_macos_runtime_asset.sh) — tarball `https://github.com/ggml-org/whisper.cpp/archive/refs/tags/v1.8.4.tar.gz` |
| **parakeet.cpp** (`parakeet-cli`, experimental) | `0.1.1` (`SBOBINO_RUNTIME_PARAKEET_CPP_VERSION`) | Same script — git tag `https://github.com/mudler/parakeet.cpp/tree/v0.1.1`, cloned with submodules |
| **SDL2** (built static for whisper examples) | `2.32.10` (`SBOBINO_RUNTIME_SDL2_VERSION`) | Same script — `https://github.com/libsdl-org/SDL/releases/download/release-2.32.10/...` |
| **FFmpeg** (standalone `ffmpeg` in speech runtime zip) | `8.1` (`SBOBINO_RUNTIME_FFMPEG_VERSION`) | Same script — `https://ffmpeg.org/releases/ffmpeg-8.1.tar.xz`, minimal `./configure` (no external GPL libraries enabled in that script) |
| **FFmpeg** (shared libs bundled for TorchCodec inside pyannote runtime) | `8.0` (archive dated `2025-03-14`, SHA256 pinned in script) | [`scripts/setup_bundled_pyannote.sh`](scripts/setup_bundled_pyannote.sh) — `https://pytorch.s3.amazonaws.com/torchcodec/ffmpeg/2025-03-14/macos_arm64/8.0.tar.gz` |
| **Python** (embedded stdlib + venv layout for pyannote) | `3.11.x` from build host (`PYTHON_VERSION=3.11`) | [`scripts/setup_bundled_pyannote.sh`](scripts/setup_bundled_pyannote.sh) |
| **pyannote.audio** (pip pin in bundled runtime) | `4.0.4` | Same script — `pip install "pyannote.audio==4.0.4"` (pulls PyTorch and other dependencies under their respective licenses) |
| **Speaker diarization pipeline (weights + config)** | Latest `main` at clone time (no commit pin in script) | Same script — default `https://huggingface.co/pyannote-community/speaker-diarization-community-1` |

When you cut a release, **update this file** if you change any of the above pins, URLs, or checksums.

## End-user summary table

| Component / data | Upstream | License (summary) | Where to read more |
|------------------|----------|-------------------|---------------------|
| Sbobino Desktop (this repository’s code) | This project | MIT | [`LICENSE`](LICENSE) |
| whisper.cpp | [ggml-org/whisper.cpp](https://github.com/ggml-org/whisper.cpp) | MIT | [LICENSE](https://github.com/ggml-org/whisper.cpp/blob/master/LICENSE) |
| parakeet.cpp | [mudler/parakeet.cpp](https://github.com/mudler/parakeet.cpp) | MIT | [LICENSE](https://github.com/mudler/parakeet.cpp/blob/master/LICENSE) |
| SDL2 | [libsdl-org/SDL](https://github.com/libsdl-org/SDL) | zlib License | [LICENSE.txt](https://github.com/libsdl-org/SDL/blob/main/LICENSE.txt) |
| FFmpeg | [FFmpeg](https://ffmpeg.org/) | **LGPL 2.1+** for most of the core library; **GPL 2+** when optional GPL components are enabled. Your obligations depend on **how** FFmpeg is built and linked. | [https://ffmpeg.org/legal.html](https://ffmpeg.org/legal.html) |
| Python | [python.org](https://www.python.org/) | PSF License Agreement | [https://docs.python.org/3/license.html](https://docs.python.org/3/license.html) |
| pyannote.audio | [pyannote/pyannote-audio](https://github.com/pyannote/pyannote-audio) | MIT (verify on the release you install) | Repository `LICENSE` |
| PyTorch, torchcodec, and other pip dependencies | PyTorch / respective authors | Mostly BSD-style and similar permissive licenses; **verify** with `pip show` / upstream repos for the exact wheels you ship | [PyTorch](https://github.com/pytorch/pytorch), [torchcodec](https://github.com/pytorch/torchcodec) |
| **Diarization model** `pyannote-community/speaker-diarization-community-1` | [Hugging Face model card](https://huggingface.co/pyannote-community/speaker-diarization-community-1) | **CC-BY-4.0** (stated in model card metadata) | Model card + [CC BY 4.0 legal text](https://creativecommons.org/licenses/by/4.0/legalcode) |
| **Parakeet GGUF ASR models** `mudler/parakeet-cpp-gguf` | [Hugging Face model card](https://huggingface.co/mudler/parakeet-cpp-gguf) | **CC-BY-4.0** (stated in model card metadata; weights are derived from NVIDIA Parakeet models) | Model card + [CC BY 4.0 legal text](https://creativecommons.org/licenses/by/4.0/legalcode) |
| Rust crates (e.g. Tauri, SQLite via `rusqlite`, `ring`, etc.) | crates.io / respective repos | Per-crate licenses in `Cargo.toml` and `Cargo.lock` | Regenerate or inspect with `cargo license` (third-party tool) if needed |

### CC-BY-4.0 model (pyannote community pipeline)

The Hugging Face repository **pyannote-community/speaker-diarization-community-1** declares **`license: cc-by-4.0`** in its model card. Typical CC BY 4.0 obligations include **appropriate credit**, a **link to the license**, and **indication of changes** if you modify the material. Read the full model card and the **Creative Commons** legal text for exact terms.

- Model card: [https://huggingface.co/pyannote-community/speaker-diarization-community-1](https://huggingface.co/pyannote-community/speaker-diarization-community-1)

Hugging Face may also show **gated access** or **additional terms** in the UI; those apply when you download through their platform.

### CC-BY-4.0 model (Parakeet GGUF ASR models)

The Hugging Face repository **mudler/parakeet-cpp-gguf** declares **`license: cc-by-4.0`** in its model card. Sbobino treats these as experimental ASR weights separate from Whisper model files. Typical CC BY 4.0 obligations include **appropriate credit**, a **link to the license**, and **indication of changes** if you modify the material.

- Model card: [https://huggingface.co/mudler/parakeet-cpp-gguf](https://huggingface.co/mudler/parakeet-cpp-gguf)

### FFmpeg source correspondence

Binaries or libraries derived from FFmpeg are subject to **FFmpeg’s license terms**. Upstream explains combinations of LGPL and GPL here: [https://ffmpeg.org/legal.html](https://ffmpeg.org/legal.html).

For **source correspondence** to the versions this project builds by default:

- **Standalone `ffmpeg` binary** (speech runtime): source archive `https://ffmpeg.org/releases/ffmpeg-8.1.tar.xz` (see [`scripts/package_macos_runtime_asset.sh`](scripts/package_macos_runtime_asset.sh)).
- **FFmpeg shared libraries** embedded for TorchCodec: archive URL and SHA256 in [`scripts/setup_bundled_pyannote.sh`](scripts/setup_bundled_pyannote.sh) (`TORCHCODEC_FFMPEG_*` variables).

If your legal counsel requires a **written offer** for LGPL/GPL source, add the exact wording they approve alongside these URLs in your distribution channel (for example the GitHub Release notes).

## Rust and JavaScript dependencies

The desktop app also depends on many smaller **Rust** crates and **npm** packages. Their licenses are declared in `Cargo.toml` / `Cargo.lock` and `package.json` / `package-lock.json`. For a full compliance matrix, use your organization’s preferred **SBOM** or license-audit tooling.

## Privacy note (separate from copyright)

Sbobino processes **audio and transcripts locally** by design, but any personal data you handle remains subject to **GDPR** and other privacy rules **independently** of software licenses. A separate privacy notice for end users may be required for your distribution.

## Revisione legale consigliata (Italia / UE)

Prima di una distribuzione pubblica ampia, valuta con un **avvocato** (competenze software / open source):

- Coerenza tra la licenza **MIT** del codice Sbobino e eventuali obblighi **copyleft** derivanti dai **binari** che redistribuisci (in particolare **FFmpeg** e librerie collegate).
- Obblighi di **attribuzione** e **condivisione** per **CC-BY-4.0** sul modello di diarizzazione.
- Eventuali **marchi** (nomi di prodotti terzi) e formulazioni nelle note di release.

Questo elenco non sostituisce quella revisione.

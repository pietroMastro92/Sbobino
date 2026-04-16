# GitHub Release notes template

Copy the block below into the **GitHub Release** description for each version. Replace placeholders in `ALL_CAPS`.

```markdown
## Sbobino `VERSION`

**Free desktop app** — local-first transcription and related features. This release is provided **as is**, without warranty of any kind (see the [MIT License](https://github.com/OWNER/REPO/blob/vVERSION/sbobino_desktop/LICENSE) for application source code).

### Third-party software and data

Installing or first-launch setup may download or install **open-source components**, including but not limited to:

- **FFmpeg** (audio processing)
- **whisper.cpp** (`whisper-cli`, `whisper-stream`) and **Whisper** model weights
- A **Python** runtime with **pyannote.audio** and related libraries
- The **pyannote-community** speaker diarization pipeline (model weights and config), licensed under **CC-BY-4.0** — see the [model card](https://huggingface.co/pyannote-community/speaker-diarization-community-1)

**Full attribution, version pins, and license references:** [THIRD_PARTY_NOTICES.md](https://github.com/OWNER/REPO/blob/vVERSION/sbobino_desktop/THIRD_PARTY_NOTICES.md)

You must comply with all applicable third-party licenses when you use, copy, or redistribute those components.

### Changelog

- …
```

### Placeholders

| Placeholder | Example |
|-------------|---------|
| `VERSION` | `0.1.16` |
| `OWNER/REPO` | Your GitHub `owner/repo` path |
| `vVERSION` | Tag name, e.g. `v0.1.16` |

Keep `THIRD_PARTY_NOTICES.md` in the repo **up to date** with the same tag you publish so the links above stay accurate.

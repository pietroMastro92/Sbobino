# SIU ASR benchmark for Sbobino v2.0.9

Date: 2026-07-05

Source audio: `/Users/pietromastro/Desktop/Automatic_transc/SIU_19062026.m4a`

Duration: 7813.995 seconds

## Results

| Engine | Model | Mode | Real seconds | RTFx |
| --- | --- | --- | ---: | ---: |
| parakeet.cpp | `tdt-0.6b-v3-q8_0.gguf` | `parakeet-batch-json`, 60s chunks, Metal | 298.15 | 26.21 |
| whisper.cpp | `ggml-large-v3-turbo-q8_0.bin` | `whisper-cli`, Italian, managed runtime | 961.18 | 8.13 |

Parakeet was 3.22x faster than Whisper on this local SIU run.

## Upstream context

- parakeet.cpp README: https://github.com/mudler/parakeet.cpp
- parakeet.cpp benchmark notes: https://github.com/mudler/parakeet.cpp/blob/master/benchmarks/BENCHMARK.md

## Evidence

- Parakeet logs: `/var/folders/8p/r_ncwk252n7gy1fc39fm55ch0000gn/T//sbobino-siu-parakeet-bench.aLJYhH`
- Whisper logs: `/var/folders/8p/r_ncwk252n7gy1fc39fm55ch0000gn/T//sbobino-siu-whisper-bench.3PcFX5`

The Parakeet batch run emitted all 131 chunk JSON records before a Metal cleanup assertion in `ggml-metal-device.m:618`. The timing is usable for throughput because output completed before process teardown.

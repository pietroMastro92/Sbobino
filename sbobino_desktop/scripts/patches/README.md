# Whisper stream replay patch

The packaged `whisper-stream` binary is built with the pinned source patch in
`whisper-stream-audio-file.patch` adds the explicit
`SBOBINO_WHISPER_REPLAY_WAV` hook for deterministic release smoke tests.
`whisper-stream-fifo.patch` prevents partial callbacks from being consumed
before a complete inference step is available. `whisper-stream-backlog.patch`
makes the stream fail closed at two seconds of backlog.

`whisper-stream-finalization.patch` prevents EOF or backlog stop from replaying
the previous inference window. `whisper-stream-lossless-drain.patch` grows the
FIFO instead of overwriting unread samples and permits the final drain only
after capture has stopped, so the recovery WAV contains the complete session.
The default microphone path remains unchanged.

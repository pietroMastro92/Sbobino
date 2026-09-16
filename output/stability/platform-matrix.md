# Native stability matrix

| Platform | Code checks | Package/install | Runtime/ASR | Interactive UI/accessibility | Status |
|---|---|---|---|---|---|
| macOS arm64 | Frontend 183/183, Rust 433 passed and packaging PASS | Technical `.app`/`.dmg` plus isolated executable launch PASS; Developer ID signing/notarization BLOCKED | Pending current native replay | Pending VoiceOver/microphone/GPU | IN PROGRESS |
| macOS Intel | Pending same-SHA runner | Pending | Pending | Pending VoiceOver | BLOCKED |
| Windows x86_64 | Pending same-SHA runner | Pending | Pending | Pending Narrator/microphone | BLOCKED |
| Linux x86_64 | Pending core/frontend CI | Not declared equivalent desktop support | Not supported as native parity proof | Not applicable | IN PROGRESS |

No row inherits PASS from historical, synthetic or another-platform evidence.

# Preparazione prodotto 2.0.37 e verifica codice (14 novembre 2026)

Stato: **preparazione di versione + gate sul codice**, non una release. Nessun
commit, stage, tag, push, dispatch, prerelease o promozione. Nessuna build di
distribuzione e nessuna rigenerazione del payload Pyannote. R6 è rimasto in
esecuzione e intatto durante tutto il lavoro.

Questo documento e `sbobino_desktop/docs/release-notes/v2.0.37.md` sono gli
unici file nuovi di questa slice; le modifiche ai metadati sono elencate in §3.
Le evidenze, i log e lo snapshot prima/dopo stanno in
`/Volumes/UltraDisk/sbobino-companion-product-20261114/official-release-product/`.

## 1. Esito in una riga

Il prodotto desktop è coerente a **2.0.37** su tutti i metadati effettivi
(8 file, incluso il lock locale) e ha note di rilascio verificate dal gate; i
gate di codice eseguiti passano (typecheck, 217 test frontend, 3 crate Rust,
17+29+5+1 test Python di packaging/CI). Restano fuori: build public della DMG,
zip Pyannote con audit a 0 violazioni, sblocco dei pin di workflow, prova
hosted e prova di pairing sul prodotto ufficiale. **Non è un PASS di release.**

## 2. Scope e limiti di questa slice

Ownership applicata:

- metadati di versione del prodotto: `package.json`, `package-lock.json`,
  `tauri.conf.json`, i quattro `Cargo.toml`, le quattro entry locali di
  `Cargo.lock`;
- nuovo `docs/release-notes/v2.0.37.md`;
- aggiornamento mirato di `docs/REPOSITORY_STATE.md`;
- questo report.

Fuori ownership, quindi **non toccati**: workflow `release.yml` /
`promote-release.yml` e relativi pin, test del freeze, codice Companion dei
gruppi A–E, script del gruppo F, `docs/companion-ai-v1.md`,
`docs/companion-diarization-readiness.md`, app iOS, `promo-video/`, `output/`,
`.agents/`, `.codex/`, `.playwright-cli/`, `AGENTS.md`,
`docs/piano-stabilizzazione-*`, `__pycache__`.

Non sono stati eseguiti: `prepare_local_release.sh`, `tauri build`,
`setup_bundled_pyannote.sh --force`, `release_readiness.sh` con un bundle,
dispatch di workflow, installazioni, pairing, modifiche a firewall/VPN/rete,
inferenze su R6/Ollama.

## 3. Snapshot e delta

Baseline: `main` a `ea429114c1ee6b15d534962cfd20593f1708b4fe`, unico worktree,
`37` voci dirty (20 `M`, 17 `??`) prima del lavoro.

Delta prodotto di questa slice, **9 voci** (8 modifiche + 1 nuovo file):

| File | Prima | Dopo |
| --- | --- | --- |
| `sbobino_desktop/apps/desktop/package.json` | 2.0.36 | 2.0.37 |
| `sbobino_desktop/apps/desktop/package-lock.json` | 2.0.36 (root + `packages[""]`) | 2.0.37 |
| `sbobino_desktop/apps/desktop/src-tauri/tauri.conf.json` | 2.0.36 | 2.0.37 |
| `sbobino_desktop/apps/desktop/src-tauri/Cargo.toml` | 2.0.36 | 2.0.37 |
| `sbobino_desktop/crates/domain/Cargo.toml` | 2.0.36 | 2.0.37 |
| `sbobino_desktop/crates/application/Cargo.toml` | 2.0.36 | 2.0.37 |
| `sbobino_desktop/crates/infrastructure/Cargo.toml` | 2.0.36 | 2.0.37 |
| `sbobino_desktop/Cargo.lock` | 4 package `sbobino-*` = 2.0.36 | 4 package `sbobino-*` = 2.0.37 |
| `sbobino_desktop/docs/release-notes/v2.0.37.md` | assente | nuovo |

Altre due modifiche, dentro file già presenti nel worktree:

- `docs/REPOSITORY_STATE.md` — era già `M`; ho aggiunto una sottosezione
  datata sulla preparazione 2.0.37 e una frase di disambiguazione nella
  baseline. Il resto del testo storico è invariato.
- `docs/reviews/companion-release-product-prep-20261114.md` — questo file,
  dentro la directory `docs/reviews/` già non tracciata: non cambia il conteggio
  delle voci `git status`.

Nessun altro file è stato modificato: dopo il lavoro `git status --porcelain`
ha 28 `M` + 18 `??` = 46 voci, cioè le 37 iniziali più le 9 del delta
(`snapshot/dirty-final.txt`). `git diff --check` è pulito su tutto il
repository. SHA-256 delle note nuove:
`f637900c9a3140dba4509c16517dc5bc0951524bbb69a971156f320b4d2f3dae`
(`evidence/new-files-sha256.txt` contiene anche l'hash di questo report).

Il `Cargo.lock` è stato allineato a mano, senza `cargo update`, senza upgrade
di dipendenze e senza rete: solo le quattro righe `version` dei package locali.
Il diff completo è in `evidence/product-version-diff.patch` (176 righe).

## 4. Punti di versione censiti e non modificati (per root)

Cercando `2.0.36` in file non ignorati restano riferimenti che **non** sono
metadati del prodotto e che vanno decisi da root:

| Punto | Natura | Azione |
| --- | --- | --- |
| `.github/workflows/release.yml:9,17,25,26` | lock del tag candidato a `v2.0.36` | aggiornare a `v2.0.37` solo dopo il commit revisionato |
| `.github/workflows/release.yml:21` | `REVIEWED_PRODUCT_SHA: 70a7e0a…` | ripuntare al commit prodotto revisionato |
| `.github/workflows/promote-release.yml:9,31,32` | promozione lockata a `v2.0.36` | sbloccare dopo la candidata |
| `.github/workflows/verification-packages.yml:95` | `check_release_versions.sh 2.0.36` | **fallirà** alla prossima esecuzione del workflow (push che tocca il file o dispatch): aggiornare a 2.0.37 |
| `.github/workflows/ci.yml:162` | smoke Intel che scarica gli asset runtime della release `2.0.36` | resta un input pinnato di proposito finché non esistono asset 2.0.37; decidere insieme ai pin |
| `.github/workflows/{arm,intel}-runtime-validation.yml:41,47`, `windows-port.yml:91,97` | validazioni storiche lockate a `v2.0.34` | storiche, dispatch manuale: non vanno "corrette" a 2.0.37 |
| `docs/companion-diarization-readiness.md:22` | esempio illustrativo del payload health (`desktop_version`) | documento di protocollo fuori ownership: valutare se aggiornare l'esempio quando il contratto viene rivisto |
| `docs/companion-ai-v1.md:3`, `README.md:3`, `docs/fluid-diarization-desktop.md`, `docs/reviews/*` | affermazioni storiche su ciò che è pubblicato o prove datate | **non toccare**: restano vere finché 2.0.37 non è pubblicata |

Il `ci.yml` che chiama `check_release_versions.sh` senza argomento (righe 77,
106) continua a funzionare: verifica solo la coerenza interna, che è 2.0.37.

## 5. Note di rilascio 2.0.37

`sbobino_desktop/docs/release-notes/v2.0.37.md` copre, con linguaggio
verificato sul codice:

- Companion LAN: server opt-in TLS con pinning, abbinamento cifrato persistente
  (`companion-pairing:v1`, sotto `factory.data_dir()`, revoca esplicita con
  `revoke_companion_pairing`, nessuna rotazione silenziosa);
- chat v2: `request_versions: [1, 2]`, `history` e citazioni versionate, ACK
  tramite la rotta di acknowledgement dei job AI;
- provider AI locali al Mac (Foundation Models o Qwen via Ollama locale), non
  sul telefono;
- turni speaker misurati (`speaker_turns_v1`) con attribuzione conservativa;
- readiness diarizzazione con insieme chiuso di 8 reason code
  (`ok`, `no_backend`, `fluid_missing`, `pyannote_missing`,
  `pyannote_repair_required`, `pyannote_receipt_required`,
  `pyannote_arch_mismatch`, `device_unsupported`) e rifiuto anticipato del job;
- diagnostica fresca dopo cambio modelli/runtime;
- fix di portabilità del packaging Pyannote (sanitize + audit condiviso prima
  di scrivere lo zip) e validazione degli asset allineata al layout spedito;
- limiti espliciti: niente build App Store, nessun relay remoto garantito,
  nessun LLM locale richiesto o incluso sull'iPhone, firma/notarizzazione
  condizionate ai secret, audit Intel/Windows non ancora provato;
- riabbinamento obbligatorio sul prodotto ufficiale: nessuna copia di token,
  certificati o chiavi dal profilo R6.

Gate delle note: `check_release_notes.sh 2.0.37 docs/release-notes/v2.0.37.md`
→ **PASS**, con range atteso `v2.0.36..v2.0.37` ricavato dal tag precedente.

## 6. Gate eseguiti (risultati reali)

| Gate | Comando | Esito | Log |
| --- | --- | --- | --- |
| Coerenza versioni | `./scripts/check_release_versions.sh 2.0.37` | **PASS** (7 sorgenti + lock + pubkey updater) | `evidence/version-coherence.txt` |
| Note di rilascio | `./scripts/check_release_notes.sh 2.0.37 docs/release-notes/v2.0.37.md` | **PASS** | `evidence/release-notes-gate.txt` |
| No test saltati | `./scripts/check_no_skipped_tests.sh` | **PASS** | `logs/check-no-skipped-tests.log` |
| Whitespace | `git diff --check` | **PASS** | `evidence/diff-check.txt` |
| Typecheck frontend | `./node_modules/.bin/tsc --noEmit` | **PASS** (exit 0) | `logs/typecheck.log` |
| Suite frontend completa | `./node_modules/.bin/vitest run` | **PASS**: 38 file, 217 test | `logs/vitest-full.log` |
| Rust domain+application | `cargo test --offline -p sbobino-domain -p sbobino-application` | **PASS**: 40 + 10 + 5 + 33 | `logs/cargo-ultradisk-target.log` |
| Rust infrastructure | `cargo test --offline -p sbobino-infrastructure` | **PASS**: 307 test (222 unit + 85 integrazione), 4 ignorati dichiarati | `logs/cargo-ultradisk-target.log` |
| Rust app crate | `cargo check --offline -p sbobino-desktop` | **PASS** (exit 0, 9,88 s a cache calda, 0 warning) | `logs/cargo-ultradisk-target.log` |
| Packaging portabile Pyannote | `python3 scripts/test_pyannote_portable_runtime.py` | **PASS**: 17 test | `logs/test_pyannote_portable_runtime.log` |
| Contratto workflow release | `python3 scripts/test_release_workflow_contract.py` | **PASS**: 29 test, 1 skip (`pwsh`) | `logs/test_release_workflow_contract.log` |
| Gate di qualità release | `python3 scripts/test_release_quality_gates.py` | **PASS**: 5 test | `logs/test_release_quality_gates.log` |
| Script bundle macOS | `python3 scripts/test_macos_bundle_scripts.py` | **PASS**: 1 test | `logs/test_macos_bundle_scripts.log` |

Ambiente dei gate: `TMPDIR`, `CARGO_HOME`, `CARGO_TARGET_DIR` e
`npm_config_cache` su `/Volumes/UltraDisk` (mount verificato, 107 GiB liberi
all'inizio), `CARGO_BUILD_JOBS=1`, `CARGO_INCREMENTAL=0`,
`CARGO_NET_OFFLINE=true`, riuso di
`/Volumes/UltraDisk/sbobino-companion-product-20261111/desktop/{cargo-home,target,npm-cache}`.
Nessuna dipendenza scaricata o aggiornata.

Nota di trasparenza: la prima esecuzione Rust ha usato la cache cargo di
UltraDisk ma **non** `CARGO_TARGET_DIR`, quindi ha creato
`sbobino_desktop/target` (3,6 GB) sul disco interno. Ho interrotto quel job,
registrato il percorso in `evidence/internal-disk-target-artifact.txt`, rimosso
quella directory (è un artefatto mio, gitignored) e rieseguito tutti i gate Rust
con `CARGO_TARGET_DIR` su UltraDisk: i risultati in tabella sono quelli della
seconda esecuzione. I numeri dei test sono identici nelle due esecuzioni.

## 7. Gate non eseguiti (e perché)

| Gate | Perché non ora |
| --- | --- |
| `prepare_local_release.sh 2.0.37 <out>` + `tauri build` | build di distribuzione: richiede chiavi updater dell'utente e produce DMG; esplicitamente fuori da questa slice |
| Rigenerazione asset Pyannote (`setup_bundled_pyannote.sh --force`) e zip con audit | richiede download/ricostruzione del runtime; il payload del repo è ancora RED (2 `host_rpath`) e la rigenerazione va fatta nella build public |
| `release_readiness.sh 2.0.37 <app>` | richiede un bundle già costruito |
| `cargo test --workspace` | include il crate app e i suoi test: parte del gate di build, non eseguito qui |
| `cargo clippy --workspace --all-targets -- -D warnings` | gate CI Linux/macOS; nessuna modifica Rust in questa slice |
| Portability smoke hosted, matrice `macos-15`, `macos-15-intel`, `windows-2025` | richiede commit pushato e workflow |
| Install DMG in `/Applications`, riabbinamento iPhone, prove LAN/AI | interazione umana sul prodotto ufficiale; R6 non va disturbato |
| Update `2.0.36 → 2.0.37` via `latest.json` | richiede release promossa |

Gate del freeze rimasti a 2.0.36, come atteso: `test_release_workflow_contract.py`
passa perché lavora su fixture; il lock reale è nei workflow elencati in §4 e
**non** è stato modificato, come richiesto.

## 8. R6 preservato

Verifica read-only prima del lavoro e dopo i gate:

- bundle `/Volumes/UltraDisk/sbobino-companion-product-20261114/desktop/chat-r6/bundle/Sbobino-Companion-R6-Chat-2.0.36-arm64.app`;
- processo PID `25290` vivo, comando = eseguibile del bundle R6;
- SHA-256 dell'eseguibile `e6c852522509e7587975bddfbf8c8c36b9889ed1af873f4e92e48dae67ffba5b` (coincide con il valore atteso);
- listener `TCP *:63101` del processo `25290`.

Nessun avvio/stop, nessuna modifica a settings, pairing, profilo o Ollama;
nessuna inferenza eseguita. Evidenza: `evidence/r6-preservation.txt`.

## 9. File da includere nel commit selettivo (A–G)

Legenda: *mio* = toccato in questa slice; *preesistente* = già nel worktree
prima, da includere così com'è.

| Gruppo | File | Origine |
| --- | --- | --- |
| A. Server/protocollo Companion | `crates/infrastructure/src/companion.rs`, `crates/infrastructure/src/companion_identity.rs`, `apps/desktop/src-tauri/src/commands/companion.rs`, `apps/desktop/src-tauri/src/lib.rs`, `crates/infrastructure/examples/companion_smoke.rs` | preesistente |
| B. AI v2 | `crates/infrastructure/src/companion_ai.rs`, `docs/companion-ai-v1.md` | preesistente |
| C. Turni misurati | `crates/application/src/services/transcription_service.rs`, `crates/application/tests/transcription_service_tests.rs`, `crates/domain/src/artifact.rs` | preesistente |
| D. Readiness/diarizzazione | `crates/infrastructure/src/fluid_diarization_runtime.rs` | preesistente |
| E. UI/i18n/a11y | `App.tsx`, `components/CompanionPanel.tsx`, `components/CompanionPanel.test.tsx`, `i18n.ts`, `test/settingsGeneralAccessibility.test.tsx`, `test/settingsRuntimeFreshness.test.tsx` | preesistente |
| F. Packaging Pyannote portabile | `scripts/package_pyannote_asset.sh`, `scripts/setup_bundled_pyannote.sh`, `scripts/lib/pyannote_portable_runtime.py`, `scripts/sanitize_pyannote_portable_runtime.sh`, `scripts/test_pyannote_portable_runtime.py` | preesistente |
| G. CI/doc | `.github/workflows/ci.yml`, `docs/REPOSITORY_STATE.md`, `docs/companion-diarization-readiness.md` | preesistente (in `REPOSITORY_STATE.md` ho aggiunto la sezione 2.0.37) |
| Versione prodotto | i 8 file di §3 | **mio** |
| Note di rilascio | `sbobino_desktop/docs/release-notes/v2.0.37.md` | **mio** (nuovo) |
| Report | `docs/reviews/companion-release-product-prep-20261114.md` | **mio** (nuovo) |
| Report di ricognizione | `docs/reviews/companion-official-release-readiness-20261114.md` | preesistente (untracked, di root) |

Da **escludere**: `.agents/`, `.codex/`, `.playwright-cli/`, `output/`,
`promo-video/`, `AGENTS.md`, `docs/piano-stabilizzazione-sol-luna-2026-09-14.md`,
`sbobino_desktop/scripts/__pycache__/`, `sbobino_desktop/scripts/lib/__pycache__/`.

Suggerimento di split per root: (1) gruppi A–G, (2) bump versione + note,
(3) documentazione/report. I pin di §4 vanno in un commit successivo, dopo la
revisione del commit prodotto.

## 10. Istruzioni successive (per root)

1. Revisione del diff di §3 e dei contenuti delle note; commit selettivo A–G
   senza gli extra elencati in §9.
2. Aggiornare i pin **dopo** il commit revisionato:
   `release.yml` (nome job, default, lock tag, `REVIEWED_PRODUCT_SHA`),
   `promote-release.yml` (default e lock), `verification-packages.yml:95`,
   e decidere per `ci.yml:162`.
3. Build public locale su Apple Silicon, con le chiavi dell'utente nel suo
   ambiente (non in argv, non in file del repo):
   ```bash
   cd /Users/pietromastro/Documents/sbobino_tauri/sbobino_desktop
   SBOBINO_RELEASE_PROFILE=public \
     ./scripts/prepare_local_release.sh 2.0.37 \
     /Volumes/UltraDisk/sbobino-companion-product-20261114/official-release-product/dist
   ```
   Attesi in `staging/aarch64/`: `Sbobino_2.0.37_aarch64.dmg`,
   `Sbobino_2.0.37_aarch64.app.tar.gz` + `.sig`,
   `pyannote-runtime-macos-aarch64.zip`, `pyannote-model-community-1.zip`,
   `speech-runtime-macos-aarch64.zip`. Il profilo public **non** incorpora
   `resources/pyannote`; `package_pyannote_asset.sh` deve fallire se l'audit
   trova violazioni. Criterio di accettazione dello zip Pyannote: **0
   violazioni** (`strict_closure` può restare `false`), nessun
   `/opt/homebrew`, `/usr/local`, `/Library/Frameworks` nei binari.
4. `./scripts/release_readiness.sh 2.0.37 <app>` sul bundle estratto, poi
   installazione reale in `/Applications` e riabbinamento iPhone: il pairing R6
   non vale per `com.sbobino.desktop` e non va copiato.
5. Tag `v2.0.37`, dispatch candidata (prerelease: nessun update utenti),
   attesa delle 7 proof + portability smoke `aarch64`/`x86_64` + Windows GUI,
   `promote_candidate_release.sh 2.0.37 pietroMastro92/Sbobino`. Rollback:
   restano `v2.0.36` e `v2.0.35`.

## 11. Rischi e limiti dichiarati

- Il bump di versione non è una certificazione: nessun artefatto pubblico
  2.0.37 esiste.
- Le note descrivono capacità del codice nel worktree; alcune prove (chat v2 su
  iPhone, turni misurati, diagnostica) sono prove di **candidato LAN**, non
  della build ufficiale.
- Il payload Pyannote del repo resta RED finché non viene rigenerato: nessun
  claim di portabilità finché l'audit della build reale non dà 0 violazioni.
- `verification-packages.yml` fallirà se eseguito con il pin 2.0.36 su un
  albero a 2.0.37: è un pin da aggiornare, non un difetto del prodotto.
- L'audit strict su macOS Intel e Windows non è stato eseguito: il primo run
  hosted può fallire, ed è il comportamento voluto.
- Non c'è stata verifica visiva/UI né interazione reale in questa slice.

## 12. File prodotto

Directory:
`/Volumes/UltraDisk/sbobino-companion-product-20261114/official-release-product/`

- `snapshot/dirty-before.txt`, `dirty-after.txt`, `dirty-final.txt`,
  `head-before.txt`, `diffstat-before.txt`, `version-files-before.sha256`;
- `evidence/version-coherence.txt`, `release-notes-gate.txt`,
  `diff-check.txt`, `product-version-diff.patch`,
  `tauri-conf-identity.json`, `internal-disk-target-artifact.txt`,
  `r6-preservation.txt`, `new-files-sha256.txt`;
- `logs/typecheck.log`, `vitest-full.log`, `cargo-ultradisk-target.log`,
  `cargo-test-domain-application.log`, `cargo-test-infrastructure.log`,
  `check-no-skipped-tests.log`, `test_pyannote_portable_runtime.log`,
  `test_release_workflow_contract.log`, `test_release_quality_gates.log`,
  `test_macos_bundle_scripts.log`.

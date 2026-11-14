# Companion nell'app ufficiale — ricognizione di release (14 novembre 2026)

Stato: **ricognizione mirata, non un PASS**. Nessuna modifica a sorgenti, versioni,
tag, workflow, installazioni, profili, chiavi, rete o payload. Nessun commit/push.
Le uniche scritture di questo lavoro sono questo file e il pacchetto evidenze in
`/Volumes/UltraDisk/sbobino-companion-product-20261114/official-release-prep/`.

Domanda dell'utente: l'app personale `/Applications/Sbobino.app` **2.0.36**
(`com.sbobino.desktop`) non contiene Companion, quindi il telefono non può
collegarsi; serve un **update ufficiale** dell'app desktop, non un candidato LAN
rinominato.

## 1. Esito in una riga

La release **2.0.37 non esiste** (ultimo tag/release pubblico `v2.0.36`,
`aa171fa`, stable, 12 ott 2026): la pipeline ufficiale è **bloccata da lock
hard-coded su `v2.0.36`** in due workflow, dal pin `REVIEWED_PRODUCT_SHA`, e dal
fatto che Companion + tooling pyannote portabile sono **solo nel worktree, non
committati**. Il gate **AS-THIRD/VM non esiste più nel repo**: il contratto
reale è GitHub-hosted (`macos-15`, `macos-15-intel`, `windows-2025`).

## 2. Conflitto skill ↔ repo (essenziale)

| Fonte | Requisito | Stato reale |
| --- | --- | --- |
| `~/.codex/skills/sbobino-release-vm-validation/SKILL.md:12-15,24,42,51-53` | VM UTM `AS-THIRD`, runner `self-hosted,macos,apple-silicon,as-third`, prova `AS-THIRD.validation-report.json` `status=passed` prima della promozione | **Non implementato**: `grep -rni "as-third"` nel repo = **0 hit** |
| `~/.codex/skills/sbobino-release-gate/SKILL.md:15,23,40` | usa la skill VM "quando quel contratto si applica" | `release.yml` usa solo runner hosted: `runs-on: macos-15` / `macos-15-intel` (`.github/workflows/release.yml:48,51,189,440-443,516,720`) |
| contratto testato | — | `sbobino_desktop/scripts/test_release_workflow_contract.py:596` (`runs-on: macos-15-intel`), `:614` (`runs-on: macos-15`), `:597-599` vieta runner `-large`/`larger` |
| gate promozione reale | — | `sbobino_desktop/scripts/promote_candidate_release.sh` richiede solo le 7 proof JSON + 4 manifest operativi; **nessuna** `AS-THIRD.validation-report.json` |

Conclusione: la skill VM è **superata** rispetto al repo. Non serve accendere
VM né chiedere all'utente un runner self-hosted; non invento waiver: il gate
corrente è quello hosted, e la prova "clean room" oggi è il portability smoke
che installa la DMG pubblica in `/Applications` su runner hosted
(`release.yml:670-692`). Da segnalare a chi mantiene le skill.

## 3. Stato remoto (read-only, 14 nov 2026)

| Voce | Valore |
| --- | --- |
| Release `Latest` | `v2.0.36`, stable, `2026-10-12T12:02:28Z` |
| Release stabili conservate | `v2.0.36`, `v2.0.35` (retention 2) |
| Tag remoti | ultimo `v2.0.36` → `aa171fa31ebc…`; **nessun `v2.0.37`** |
| `origin/main` | `692cdb5` (CI + Verification installers: **success**, 29 ott) |
| `main` locale | `ea42911` (`atop` di `692cdb5`, **non pushato**) → `checks` API: `No commit found` |
| Asset pubblici 2.0.36 | 28, incl. `latest.json`, DMG `aarch64`/`x86_64`, `app.tar.gz`+`.sig`, setup NSIS Windows, 3 `pyannote-runtime-*`, 3 `speech-runtime-*` |
| `latest.json` 2.0.36 | endpoint updater `releases/latest/download/latest.json` → **serve solo release non-prerelease**: un candidato prerelease non aggiorna nessuno finché non viene promosso |

Versione target verosimile: **2.0.37** (2.0.36 è l'ultima pubblicata; nessun tag
intermedio). Il numero va deciso da root, ma la mappa di edit è in §6.

## 4. Classifica del worktree (37 voci: 20 `M` + 17 `??`)

Necessari alla release Companion:

| Gruppo | File | Collegamento funzionale | Prova |
| --- | --- | --- | --- |
| A. Server/protocollo Companion | `crates/infrastructure/src/companion.rs` (+1307), `companion_identity.rs` (nuovo, 13,7 KB), `apps/desktop/src-tauri/src/commands/companion.rs`, `src/lib.rs` (+685), `examples/companion_smoke.rs` | health TLS, job ASR, pairing cifrato durevole, revoca, lease | R6 `6c85252…` compilato da questi sorgenti; pairing iPhone riparato `192.168.1.167:63101`, health 200 `loadStored`, 3+3 turni reali |
| B. AI v2 (history/citazioni) | `crates/infrastructure/src/companion_ai.rs` (+431), `docs/companion-ai-v1.md` | `/v1/ai/jobs` + `/ack`, `history`, `request_versions:[1,2]` | report iOS `docs/reviews/iphone-foundation-qwen-20261114.md`: history 0/1/2, citazioni, ACK/GET `acknowledged` |
| C. Turni misurati / speaker | `crates/application/src/services/transcription_service.rs`, `crates/application/tests/transcription_service_tests.rs`, `crates/domain/src/artifact.rs` | `speaker_turns_v1`, guardia ambiguità D1, encoder f32 | test v2 8/8 su sorgenti; `docs/reviews/companion-measured-turns-phone-20261113.md` 93 PASS telefono |
| D. Readiness/diarizzazione | `crates/infrastructure/src/fluid_diarization_runtime.rs` (+363) | reason code chiusi (`pyannote_missing|repair_required|arch_mismatch`), fallback Intel | guard nei test; `pyannote_repair_required` |
| E. UI/i18n/a11y | `App.tsx` (+240), `CompanionPanel.tsx` (+279), `CompanionPanel.test.tsx`, `i18n.ts`, `settingsGeneralAccessibility.test.tsx`, `test/settingsRuntimeFreshness.test.tsx` (nuovo) | pannello Companion, freschezza stato modelli locali, stringhe it/en, a11y | test frontend inclusi; R6 UI invariata dal 13 nov |
| F. Packaging pyannote portabile | `scripts/package_pyannote_asset.sh`, `scripts/setup_bundled_pyannote.sh`, `scripts/lib/pyannote_portable_runtime.py` (nuovo), `scripts/sanitize_pyannote_portable_runtime.sh` (nuovo), `scripts/test_pyannote_portable_runtime.py` (nuovo) | audit strict condiviso generatore/packager/riparatore; blocca rpath host prima dello zip | R2 `tree afd03b0…`, audit 449 Mach-O / 438 firmati / 132 warning / 0 violation (evidence UltraDisk) |
| G. CI/doc | `.github/workflows/ci.yml` (+3: test portabilità su Linux e sulle due matrici macOS), `docs/REPOSITORY_STATE.md`, `docs/companion-diarization-readiness.md` (nuovo, contratto wire) | coerenza documentazione e test | `python3 -m unittest … test_pyannote_portable_runtime.py` |

Da **escludere** dalla release (altrui/artefatti, non toccare): `.agents/`,
`.codex/` (config agenti), `.playwright-cli/`, `output/` (playwright+stability),
`promo-video/` (217 MB, worker promo), `AGENTS.md`,
`docs/piano-stabilizzazione-sol-luna-2026-09-14.md`, `scripts/__pycache__/`,
`scripts/lib/__pycache__/`.

## 5. BLOCKER reali (ordinati)

| # | Blocco | Evidenza | Criterio di sblocco |
| --- | --- | --- | --- |
| B1 | `Release Candidate` **rifiuta ogni tag ≠ `v2.0.36`** e impone che `apps`, `crates`, `scripts`, `Cargo.toml`, `Cargo.lock` siano **identici** a `REVIEWED_PRODUCT_SHA=70a7e0a` | `.github/workflows/release.yml:17,21,26,35` | aggiornare nome job, default input, lock tag e pin SHA al commit revisionato della 2.0.37 |
| B2 | `Promote Release Candidate` è **lockato a `v2.0.36`** | `.github/workflows/promote-release.yml:9,31-32` | sbloccare/parametrizzare (o promuovere via `promote_candidate_release.sh` locale, che non ha lock) |
| B3 | Companion + tooling pyannote **non committati**: `companion_identity.rs`, `pyannote_portable_runtime.py`, `sanitize_…sh`, `test_pyannote…py`, `settingsRuntimeFreshness.test.tsx` sono `??`; 20 file sono `M` | snapshot `dirty-before.txt` (37 voci) | commit selettivo dei gruppi A–G; **non** degli extra; HEAD è 1 commit avanti a origin e non pushato |
| B4 | Gli asset pyannote pubblicati 2.0.36 sono stati prodotti dal sanitizer **pre-ottobre** (espone `/opt/homebrew/lib`); la copia nel repo è ugualmente RED | audit repo: **2 violation** `host_rpath` (`lib/libpython3.11.dylib`, `lib/Resources/Python.app/Contents/MacOS/Python`), 434 binari, 423 firmati — `pyannote-portable/evidence/red-audit-repo-snapshot.json`; verifica diretta `otool -l …/libpython3.11.dylib` | rigenerare gli zip con `setup_bundled_pyannote.sh --force` (sanitize+audit, `setup:298,309-311`) e far passare `package_pyannote_asset.sh:21,42`; pubblicare solo output con 0 violation |
| B5 | Portabilità **Intel/Windows non provata** con il nuovo audit: nessun asset `x86_64` R2 sanitizzato; il packager Windows è invariato | payload R2 = solo `aarch64-apple-darwin`; `package_windows_pyannote_runtime.ps1` non modificato | primo run CI su `macos-15-intel` + `windows-2025`; se il sanitizer non ripara, il job **deve** fallire (comportamento voluto), non essere aggirato |
| B6 | Nessuna prova del **prodotto ufficiale** con Companion: identità, pairing, UI e AI sono stati provati solo su bundle LAN con identità di test | R6 = `com.sbobino.companion.baseline.lan20261112`; bundle ufficiale 2.0.36 = `com.sbobino.desktop` | build ufficiale `com.sbobino.desktop` + install in `/Applications` + pairing reale + smoke + portability smoke hosted (come `release.yml:670-692`) |

Non-blocker ma da sapere: il profilo `public` **non deve** incorporare pyannote
(`release_readiness.sh:82-88`); l'app ufficiale installa il runtime dagli asset
di release (`pyannote-runtime-*`, `compat_level=1`, `lib.rs:114`). Il candidato
LAN con payload innestato è un artefatto di test, non il modello di spedizione.
Update Mac/Windows passa da `latest.json` firmato: notarization presente ma
**condizionale** ai secret (`release.yml:166-177`), altrimenti DMG non firmata.

## 6. Mappa esatta degli edit di versione (per 2.0.37)

Verificato con `check_release_versions.sh` (coerente ora a 2.0.36, exit 0):

1. `sbobino_desktop/apps/desktop/package.json` (`version`)
2. `sbobino_desktop/apps/desktop/package-lock.json` (root + `packages[""].version`)
3. `sbobino_desktop/apps/desktop/src-tauri/tauri.conf.json` (`version`)
4. `sbobino_desktop/apps/desktop/src-tauri/Cargo.toml`
5. `sbobino_desktop/crates/{domain,application,infrastructure}/Cargo.toml`
6. `sbobino_desktop/Cargo.lock` (4 package `sbobino-*`)
7. `sbobino_desktop/docs/release-notes/v2.0.37.md` (nuovo; sezioni obbligatorie `Fixes`, `New and improved`, `Compatibility`, `Known issues`, `Refs` — `check_release_notes.sh:19-23`)
8. `.github/workflows/release.yml` (job name:17, default input, lock:26, `REVIEWED_PRODUCT_SHA`:21)
9. `.github/workflows/promote-release.yml` (default:9, lock:31-32)
10. `docs/REPOSITORY_STATE.md` (stato pubblicato/versione; oggi dice 2.0.36)

Non serve toccare `Info.plist` (nessuna chiave versione) né
`tauri.windows.conf.json` (nessuna versione). `latest.json`,
`setup-manifest.json`, `runtime-manifest.json`, `pyannote-manifest.json` sono
**generati** da `generate_release_candidate_metadata.py` / `assemble_local_release.sh`
e validati contro il tag.

## 7. Identità e pairing (R6 → ufficiale)

- R6 usa `com.sbobino.companion.baseline.lan20261112` con data dir isolata; il
  pairing è cifrato e legato a **quell'identità/profilo**
  (`companion_identity.rs:1-18`, record AES-GCM `companion-pairing:v1`).
- L'app ufficiale `com.sbobino.desktop` avrà uno store diverso: **serve un
  riabbinamento una volta sola** (nuovo QR/TLS pin), non si copiano token,
  certificati o chiavi dal profilo R6.
- Revoca: comando dedicato `revoke_companion_pairing` + stato
  `inactive_paired|unknown` (`commands/companion.rs`), quindi il pairing R6 è
  revocabile a mano quando non serve più.

## 8. Mini-piano (prima candidato installabile vero, poi update pubblico)

Fase 1 — candidato ufficiale installabile (Mac ARM):

1. Commit selettivo gruppi A–G (esclusi gli extra), `cargo fmt`, test mirati.
2. Bump versione 2.0.37 (punti 1–7 di §6) + release notes 2.0.37.
3. `SBOBINO_RELEASE_PROFILE=public ./scripts/prepare_local_release.sh 2.0.37 <out>` su
   Apple Silicon con chiavi updater: produce DMG `com.sbobino.desktop`, `app.tar.gz`+firma
   e **rigenera** gli zip pyannote; qui l'audit strict deve dare 0 violation.
4. Installare davvero la DMG in `/Applications` (non copiare un bundle di test),
   riabbinare l'iPhone, verificare health/job/AI e la UI Companion.

Fase 2 — pubblicazione e update:

5. Aggiornare i lock B1/B2 al commit revisionato, tag `v2.0.37`, dispatch
   `Release Candidate` (prerelease: nessun update per gli utenti 2.0.36).
6. Attendere le 7 proof + portability smoke `aarch64`/`x86_64` + Windows GUI.
7. `promote_candidate_release.sh 2.0.37` → stable + `latest` → l'updater di
   `2.0.36` serve `latest.json` 2.0.37. Rollback: restano `v2.0.36`/`v2.0.35`.

## 9. Test necessari e perché mancano oggi

| Test | Perché manca | Dove appartiene |
| --- | --- | --- |
| build ufficiale `public` + audit pyannote 0 violation | il payload locale è RED e nessun run CI ha usato il nuovo audit | CI `release.yml` su `macos-15` |
| runtime `x86_64` sanitizzato | R2 è solo arm64; il packager Intel non è mai stato eseguito col nuovo audit | CI `macos-15-intel` |
| runtime Windows | sanitizer è macOS-only, packager `.ps1` invariato | CI `windows-2025` |
| install reale DMG + pairing identità ufficiale | provato solo su bundle LAN | umano su Mac + iPhone |
| update `2.0.36 → 2.0.37` da `latest.json` | richiede release promossa | dopo promozione |
| Intel ASR realtime | **non** è un PASS atteso: il contratto pretende `EXPECT_PREFLIGHT_REJECTION=1` su x86_64 (`release.yml:474-481`) | CI Intel |

## 10. Check eseguiti (read-only)

- `./scripts/check_release_versions.sh` → **PASS** (2.0.36 su 7 sorgenti + lock).
- `bash -n` su 7 script toccati → PASS; `ast.parse` su 2 file Python nuovi → PASS;
  YAML parse su `ci.yml`/`release.yml`/`promote-release.yml` → PASS.
- `gh release list/view`, `gh api` checks su `692cdb5` (success) e `ea42911`
  (`No commit found`), `git ls-remote` tags/main → sola lettura.
- Nessuna build, nessun `prepare_local_release.sh`, nessun test Rust, nessuna
  azione su workflow/tag/install/profili/rete.

Evidenze complete e snapshot prima/dopo:
`/Volumes/UltraDisk/sbobino-companion-product-20261114/official-release-prep/`.

Non è un PASS di release: R6 e le prove telefono restano prove di **prodotto
candidato**, non della build ufficiale `com.sbobino.desktop`.

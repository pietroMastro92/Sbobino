# Companion 2.0.38 — packaging blocker: causa reale, fix strutturale, prove

Data: 2026-11-15 · Slice: implementazione (fix + versione 2.0.38), nessun commit/stage/tag/dispatch
Repo: `/Users/pietromastro/Documents/sbobino_tauri` @ `2aebad88f52a77cf510cb335602eecf57229609f` (`main`, allineato a origin)
Precedente: `v2.0.37` annotato `f8d50ff` → `5198720` (immutato, non riscritto) · stabile `2.0.36`
Run falliti di riferimento: <https://github.com/pietroMastro92/Sbobino/actions/runs/43628701223>, <https://github.com/pietroMastro92/Sbobino/actions/runs/43632133897>
Workspace di lavoro (temporanei, cache, Cargo/npm, prove): `/Volumes/UltraDisk/sbobino-companion-product-20261115/packaging-2038/` (APFS locale, `nodev,nosuid,noowners`, 83 GiB liberi)

**Esito: B1 confermato e risolto; B2 (come diagnosticato nel report run 2) non esiste — era un artefatto
del parser `otool`, non un load command. Il difetto reale che resta è un'altra classe di 28 install name
morti. Fix strutturale applicato a entrambi, con prove su Mach-O reali e sul loader. Versione 2.0.38
pronta per review.**

---

## 1. Cosa è stato misurato (prima di toccare il codice)

Payload di riferimento, read-only: `/Volumes/UltraDisk/sbobino-companion-product-20261112/desktop/
pyannote-portable/payload-r2/pyannote`, `tree_sha256 = afd03b07d34bb525bf22652ee1b0607f67c819d6ee
85f45ff2a5db13a6a16d5d`, 29 205 file, 855 477 217 byte, 449 Mach-O.

Audit con il codice di `HEAD` sulla radice originale: **0 violazioni, 132 warning** (36
`non_system_install_name`, 84 `unresolved_rpath_dependency`, 11 `unsigned_macho`, 1
`unrecognized_dependency`). Cioè: il payload che avrebbe dovuto essere rosso era verde.

Censimento vero dei load command (`evidence` locale, sezione 6):

| Classe | Misura reale | Nel report run 2 |
| --- | --- | --- |
| install name assoluti non di sistema | **26** (`/DLC/...` ×21, `/opt/llvm-openmp/lib/libomp.dylib`, `/DLC/torchcodec/.dylibs/libpython3.11.dylib`, `/DLC/torchaudio/...`, `/DLC/sklearn/...`) | 36 |
| install name relativi non `@` | **2** (`bazel-out/osx-*/...` su `google/_upb/_message.abi3.so`) | — |
| dipendenze assolute non di sistema | **0** | 11 |
| rpath assoluti non di sistema | **0** | — |
| riferimenti "interni alla radice di staging" | **0** | 21 |

I 21 riferimenti "interni" e gli 11 `-id`/11 dipendenze del report run 2 sono **righe di intestazione
per-architettura** di Mach-O universali lette come dipendenze. Prova diretta su un file reale citato nel
report:

```
$ lipo -archs .../charset_normalizer/md.cpython-311-darwin.so
x86_64 arm64
$ otool -D  .../charset_normalizer/md.cpython-311-darwin.so
.../md.cpython-311-darwin.so (architecture x86_64):
.../md.cpython-311-darwin.so (architecture arm64):
$ otool -l  .../md.cpython-311-darwin.so | grep -c LC_ID_DYLIB
0
$ otool -L  .../md.cpython-311-darwin.so
(architecture x86_64): /usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1345.120.2)
(architecture arm64):  /usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1345.120.2)
```

Il file **non ha** LC_ID_DYLIB e **non ha** dipendenze assolute: l'unica dipendenza è `libSystem`. Il
parser di `HEAD` saltava solo la prima riga, quindi per ognuno degli 11 Mach-O universali produceva una
dipendenza fantasma pari al path assoluto del file stesso, e per 10 di essi anche un `-id` fantasma. Il
report run 2 ha misurato il parser, non il payload.

Conseguenza operativa: il rimedio n.2 proposto in `proposed-fix-stage-and-sanitize.patch.md` non è
applicabile alla lettera — `install_name_tool -change <ref> ...` su quei 21 riferimenti fallisce, perché
non sono LC_LOAD_DYLIB. Applicarlo avrebbe rotto la build in un punto diverso, più difficile da leggere.

Secondo difetto del parser, indipendente: `stripped.split(" ", 1)[0]` troncava i path contenenti spazi
(`/Volumes/My Disk/...` → `/Volumes/My`). Non osservabile sui runner, osservabile su qualunque volume
con spazio nel nome, incluso il mio staging.

Terzo difetto, di test: a `HEAD` la suite macOS-only **è già rossa su macOS**
(`evidence/redgreen/RED-HEAD-original-suite.log`, 17 test, 1 failure + 5 error). Ogni test che chiama
`sanitize_runtime` su `tempfile.TemporaryDirectory()` ottiene `/var/folders/...`, che `Path.resolve()`
trasforma in `/private/var/...`, che `refuse_personal_root` rifiuta per progetto. Lo step
`Verify native Pyannote portability tooling` di `ci.yml:147-148` gira su `macos-15` e
`macos-15-intel`: era quindi rosso indipendentemente da questa slice.

## 2. Causa confermata B1 (invariata rispetto al run 2)

`setup_bundled_pyannote.sh` creava lo staging con `mktemp -d` senza template. Su macOS (BSD `mktemp`)
quella forma equivale a `-t tmp` e **ignora `TMPDIR`**: `export TMPDIR="$RUNNER_TEMP"` nel workflow non
aveva effetto, lo staging finiva in `/var/folders/.../T/tmp.XXXX`, e `sanitize_runtime` lo rifiutava dopo
il build. Riproduzione locale e `man mktemp` già in `evidence/mktemp-tmpdir-repro.log` del run 2:
confermati, nessuna nuova ricognizione.

## 3. Fix applicato

Solo quattro file di prodotto/tooling più i metadati; nessun workflow, nessuna dipendenza nuova, nessuna
modifica a Windows, Companion, UI o comportamento a runtime dell'app.

### 3.1 Staging deterministico — `sbobino_desktop/scripts/setup_bundled_pyannote.sh`

- `tmp_parent_dir()` (`:98`) risolve `TMPDIR` in path **assoluto canonico** (`cd && pwd -P`), creandolo se
  manca, e rifiuta un `TMPDIR` relativo.
- `stage_parent_dir()` (`:117`) applica il guard personale **prima** di creare qualunque cosa e di nuovo
  dopo la risoluzione dei symlink: `/Users/runner/work/_temp` è accettato, `/private/...`,
  `.../Library/Application Support/...`, `pairing.enc`, `master_key_v1` sono rifiutati con messaggio
  azionabile *prima* del download del modello.
- `create_stage_dir()` (`:145`) usa l'unico idioma portabile: `mktemp -d "$parent/tmp.XXXXXXXXXX"`
  (template esplicito, quotato: gli spazi nel parent funzionano, nessun flag GNU-only).
- `cleanup_stage_dir()` (`:157`) è fail-closed: rifiuta path vuoto, relativo, `/`, o fuori da
  `$STAGE_PARENT/tmp.*`.
- `--stage-dir-probe` (`:33`, `:50`, `:667`) risolve, crea e rimuove la directory reale **senza
  download**, così il contratto di staging è testabile in isolamento.
- `python_supports_venv_creation()` (`:215`) usa lo stesso idioma invece di `mktemp -d`.

Guard **non allentato**: `PERSONAL_PATH_MARKERS` e `refuse_personal_root` sono invariati; il nuovo
`guard-root` (sotto) espone la stessa funzione, non una variante.

### 3.2 Parser, audit e sanitizer — `sbobino_desktop/scripts/lib/pyannote_portable_runtime.py`

Parser:
- `_otool_section_lines()` (`:149`) scarta le intestazioni di file e per-architettura: nessuna
  dipendenza fantasma, nessun `-id` fantasma.
- `parse_otool_dependencies()` (`:158`) conserva i path con spazi e deduplica per architettura.
- `read_install_names()` (`:237`) restituisce **tutti** gli install name distinti (uno per slice);
  `read_linked_dependencies()` (`:268`) rimuove ogni install name proprio, non solo il primo.

Audit (`audit_runtime`, `:451`) — nuove regole, nessun allentamento:
- `classify_absolute_reference()` (`:388`) ancora la decisione al **layout del payload**, non al prefisso:
  `inside` (path reale dentro la radice), `suffix` (path con radice estranea il cui tail corrisponde
  esattamente a un file del payload), `ambiguous` (più file corrispondono: mai scegliere), `external`.
  Un tail di un solo componente non può mai "trovare" un file: niente basename guess.
- `absolute_runtime_reference` (**violazione**): install name o dipendenza assoluta che nomina un file
  del payload. Scatta identico con radice `/Volumes/...`, `/Users/...` o `/tmp/...`: il caso "verde solo
  perché la radice inizia con `/Volumes`" non è più rappresentabile.
- `ambiguous_runtime_reference` (**violazione**): tail ambiguo, fail closed.
- `non_portable_install_name` (**violazione**, prima era warning): install name assoluto non di sistema
  che non nomina il payload (prefissi di delocazione tipo `/DLC`, prefix conda) oppure install name
  relativo non `@` (`bazel-out/...`). Un payload non deve poter essere spedito con path morti.
- `absolute_dependency` (**violazione**, prima era warning): dipendenza assoluta non di sistema che non
  esiste nel payload. Una dipendenza deve essere apribile dal loader: fail closed.
- `ambiguous_rpath_dependency` (nuovo warning): il fallback per basename non sceglie più arbitrariamente
  fra due candidati; lo dichiara. Misurato: `libtorch_cpu.dylib` → `@rpath/libomp.dylib` (due copie nel
  payload).
- Invariati: `host_*`, `absolute_rpath`, `personal_path_reference`, `outside_payload_dependency`,
  `unresolved_*` come warning, verifiche di firma per Mach-O e bundle.

Sanitizer (`sanitize_runtime`, `:791`) — due fasi:
1. **Piano read-only**: per ogni Mach-O calcola le modifiche, validando *tutto* prima di scrivere.
   `resolve_internal_dependency()` (`:737`) accetta un target solo se (a) è dentro la radice dopo
   `resolve(strict=True)`, (b) non è fuggito via symlink, (c) il testo di riferimento coincide con uno
   degli install name del target oppure il target è il binario stesso. Altrimenti `SystemExit`: niente
   "aggiusto qualsiasi path personale", niente mapping inventati, niente indovinelli per basename.
2. **Applicazione**: `-change <assoluto> @loader_path/<rel>` per le dipendenze interne, `-id @rpath/<name>`
   per gli install name non portabili (una sola chiamata per file, tutte le slice), copia in
   `lib/embedded-dylibs` per i prefissi host, rimozione degli rpath assoluti/host. Poi firma: prima i
   Mach-O interni (comprese le copie embedded, prima non rifirmate), poi i bundle `.app` che li
   contengono, poi verifica. Un secondo giro su payload già sanitizzato è un **no-op a livello di byte**.

Preservati: librerie di sistema, `@loader_path`, `@executable_path`, `@rpath`.

### 3.3 Test

`scripts/test_pyannote_portable_runtime.py` (17 → 39 test):
- `test_universal_binary_architecture_headers_are_not_dependencies`,
  `test_install_name_parser_skips_universal_arch_headers`,
  `test_dependency_path_with_spaces_is_not_truncated` — regressione del parser.
- `ReferenceClassificationTests` — `inside` / `suffix` / `ambiguous` / `external` / `not_absolute`.
- `RelocatablePayloadTests` — fixture Mach-O nativa (2 dylib + eseguibile) compilata con `cc`, con
  install name e dipendenze assolute; audit rosso alla radice di build e **anche da un'altra radice**;
  sanitize; audit verde; `codesign --verify`; **`mv` dell'intero albero in un path con spazi**;
  esecuzione dell'eseguibile e `ctypes.CDLL` sul dylib → valore corretto (8). Idempotenza byte-level.
  Fail-closed su dipendenza mancante, su escape via symlink, su riferimento ambiguo.
- `test_sanitize_repairs_a_dead_external_install_name` — replica il layout reale dei wheel
  (`vendor/.dylibs/...` con id `/DLC/vendor/...`) e verifica rewrite + audit + caricamento dopo `mv`.
- `test_system_install_name_is_preserved_and_not_flagged` — un LC_ID_DYLIB sotto `/usr/lib` non è una
  violazione e il sanitizer lo lascia intatto.
- `UniversalBinaryTests` — fixture universale reale (`cc -arch x86_64` + `-arch arm64` + `lipo`): due id
  per slice, nessun artefatto di intestazione, rewrite di tutte le slice, architetture preservate.
- Igiene: `non_personal_test_directory()` e `non_personal_staging_base()` scelgono la prima radice che
  il guard **spedito** accetta (`SBOBINO_RELOCATION_TEST_ROOT` / `SBOBINO_STAGING_TEST_ROOT`, poi temp di
  sistema, poi `~/.cache/sbobino-portable-tests`), così la suite è verde su macOS invece di fallire su
  `/private`. È il fix del terzo difetto della sezione 1; la suite passa anche senza variabili
  d'ambiente (percorso di fallback esercitato).

`scripts/test_setup_pyannote_staging.py` (nuovo, 11 test): staging con `TMPDIR` senza spazi e **con
spazi**, nessun fallback al temp di sistema, cleanup verificato, rifiuto di parent personale e di
`TMPDIR` relativo, test documentale del comportamento BSD (dimostra che `mktemp -d` ignora `TMPDIR`),
guard statico "ogni `mktemp` ha un template".

## 4. RED / GREEN reali

Stessa suite (50 test) eseguita contro il codice di `HEAD` e contro il codice nuovo, in due alberi
separati con i file di `HEAD` estratti da `git show`:

| | Test | Esito |
| --- | --- | --- |
| RED (`HEAD` + test nuovi) | 50 | **FAILED (failures=20, errors=12)** → 32 rossi |
| GREEN (fix) | 50 | **OK**, 8.8 s |

Log: `redgreen/RED.log`, `redgreen/GREEN.log`. Il RED è autentico: mancano le API nuove
(`classify_absolute_reference`), il probe, il template `mktemp`, e le regole di audit.

## 5. Prova end-to-end sul payload reale (copia APFS, fuori repo)

Tutto in `packaging-2038/proof/`, payload clonato con `cp -cRp` (CoW), 29 205 file, mai modificato
l'originale.

1. **Audit pre** (copia a radice diversa): 28 violazioni `non_portable_install_name`, 97 warning
   (85 `unresolved_rpath_dependency` = varianti torchcodec inattive, 11 `unsigned_macho`, 1
   `ambiguous_rpath_dependency`). `audit-pre.json`.
2. **Sanitize**: 27 Mach-O modificati, **28 install name** riparati (il Mach-O universale
   `_message.abi3.so` ne ha due), 0 dipendenze, 0 rpath.
3. **Audit post**: **0 violazioni**, 96 warning (85 + 10 + 1). `audit-post.json`.
4. **Idempotenza**: seconda esecuzione → `modified 0`, e `tree_sha256 =
   8be550398a176110d29ab226fa60ba3f7751d9b00e7364004ea5732de9c4fb64` identico prima e dopo
   (`inventory-after-sanitize*.json`).
5. **Spostamento** dell'intero albero in `proof/relocated payload with spaces/payload` (path con spazi):
   - censimento load command: **0 assoluti non di sistema**; 73 id `@rpath`, 77 dipendenze
     `@loader_path`, 210 `@rpath`, 668 di sistema, 37 rpath relativi;
   - `codesign --verify --strict` su `Python.app`: *valid on disk* + *satisfies its Designated
     Requirement*;
   - audit: **0 violazioni** (`audit-relocated-final.log`);
   - inventario **identico** al post-sanitize (`files 29 205`, `bytes 854 960 399`,
     `tree 8be55039...`, `inventory-relocated-final.json`): il `mv` non altera i byte.
   Nota di igiene: una qualunque esecuzione dell'interprete impacchettato **senza**
   `PYTHONDONTWRITEBYTECODE` scrive `__pycache__` dentro il runtime (osservato e rimosso solo nella
   copia di prova); il payload spedito non ne contiene.
6. **Caricamento reale** dal tree spostato (interprete impacchettato, `env -i`): 15 controlli,
   **0 fallimenti** — import + uso di `charset_normalizer.md`, `fontTools.varLib.iup`,
   `grpc._cython.cygrpc`, `google._upb._message`, `PIL.Image` (+ creazione immagine), `scipy.linalg`
   (+ `inv`), `sklearn.utils._isfinite`, `numpy.linalg`, `torch`, `torchcodec`, `pyannote.audio`.
   `relocation-imports.log`.
7. **Entry di runtime** `pyannote_diarize.py` sul tree spostato, audio TTS **sintetico e offline**
   (`say` + `afconvert`, 8.83 s, due voci): exit 0, JSON valido con **2 speaker** e confini coerenti con
   le due voci sintetizzate (0.03–3.81 s speaker 1, 4.45–8.16 s speaker 2). Nessun dato personale.
   `diarize-tts-result.json`.
8. **Regressione audit su artefatto reale** (`audit-regression/`): sul `_message.abi3.so` reale (Mach-O
   universale) è stato iniettato con `install_name_tool` un id assoluto **interno** sotto `/Volumes` —
   la forma esatta che il report run 2 attribuiva a mypyc. Audit: **1 violazione
   `absolute_runtime_reference`**, con dettaglio `resolves inside the payload: ...` e *nessun* hit sui
   marker personali (la radice non inizia con `/Users`). Sanitize → audit verde → albero spostato in un
   path con spazio → `google._upb._message` carica e protobuf serializza. `audit-regression-*.log`.

## 6. Gate eseguiti (tutti verdi)

| Gate | Comando / esito |
| --- | --- |
| Suite packaging portabile (esistente + nuova) | `python3 -m unittest scripts/test_pyannote_portable_runtime.py scripts/test_setup_pyannote_staging.py` → 50 test, OK |
| Sintassi | `bash -n setup_bundled_pyannote.sh` OK; `py_compile` sui tre file Python OK |
| Contratti runtime sorgente (read-only) | `check_macos_runtime_source.sh` OK; `check_windows_powershell_source.sh` OK (6 file) |
| Contratto workflow (read-only, nessun pin toccato) | `test_release_workflow_contract.py` → 29 test OK (1 skipped, richiede `pwsh`) |
| Altri contratti CI | `test_evaluate_live_latency` + `test_run_whisper_live_replay` + `test_prepare_fluid_diarizer_models` + `test_package_fluid_diarizer` + `test_run_fluid_speaker_replay` → 39 test OK |
| Policy test saltati | `check_no_skipped_tests.sh` → nessuna dichiarazione skip/todo |
| Coerenza versione | `check_release_versions.sh 2.0.38` → package.json / tauri.conf.json / 3 crate / Cargo.lock tutti 2.0.38 |
| Release notes | `check_release_notes.sh 2.0.38 docs/release-notes/v2.0.38.md` → passed (range `v2.0.37..v2.0.38`) |
| Whitelist diff | `git status`/`git diff --stat`: 11 file modificati + 3 nuovi, nessun workflow, nessun `package_windows_*`, nessuna dipendenza |

## 7. File toccati

| File | Cambio |
| --- | --- |
| `sbobino_desktop/scripts/setup_bundled_pyannote.sh` | staging con template, guard preventivo, cleanup fail-closed, `--stage-dir-probe` |
| `sbobino_desktop/scripts/lib/pyannote_portable_runtime.py` | parser per-architettura, classificazione layout-based, nuove regole di audit, sanitizer a due fasi |
| `sbobino_desktop/scripts/test_pyannote_portable_runtime.py` | 24 test nuovi + igiene radice di test |
| `sbobino_desktop/scripts/test_setup_pyannote_staging.py` | **nuovo**, 8 test di staging |
| `sbobino_desktop/docs/release-notes/v2.0.38.md` | **nuovo** |
| `docs/reviews/companion-2038-packaging-fix-20261115.md` | **nuovo** (questo report) |
| `sbobino_desktop/apps/desktop/package.json` | 2.0.38 |
| `sbobino_desktop/apps/desktop/package-lock.json` | 2.0.38 (2 occorrenze, nessun cambio dipendenze) |
| `sbobino_desktop/apps/desktop/src-tauri/tauri.conf.json` | 2.0.38 |
| `sbobino_desktop/apps/desktop/src-tauri/Cargo.toml` | 2.0.38 |
| `sbobino_desktop/crates/{domain,application,infrastructure}/Cargo.toml` | 2.0.38 |
| `sbobino_desktop/Cargo.lock` | 4 entry `sbobino-*`, nessuna dipendenza toccata |

Non toccati: `.github/**` (root aggiorna `REVIEWED_PRODUCT_SHA` dopo la review), `package_windows_*`,
`package_pyannote_asset.sh` (nessun cambio di contratto necessario: audit e sanitizer condivisi bastano),
`sanitize_pyannote_portable_runtime.sh`, `resources/pyannote` (ignorato, 29 192 file,
`tree_sha256 = b633b846...` — **tree diverso** dal payload R2, non rigenerato qui), app/Companion/UI/ASR.

## 8. Limiti dichiarati (nessun claim oltre la misura)

- Tutte le prove di payload sono su **macOS arm64**. Intel e Windows non sono provati qui: andranno
  provati sul primo run hosted del candidato 2.0.38, come previsto.
- Il payload verificato è la **copia** del R2 (input di questa slice), non il `resources/pyannote`
  locale, che è un albero più vecchio e diverso. Il run hosted rigenererà gli asset da zero: è quello il
  payload che entrerà nell'artefatto. Nessun remap inventato sul vecchio albero.
- Le 85 dipendenze `@rpath` non risolte sono le varianti torchcodec FFmpeg 4/5/6/7 precompilate nei wheel
  (solo la variante inclusa viene importata) e le 10 firme mancanti sono `.so` coperti dalla firma del
  bundle: **invariate** rispetto al payload stabile, fuori dalla portata di questa slice (richiederebbero
  wheel diversi).
- `resources/pyannote` locale continuerà a fallire il nuovo audit finché non viene rigenerato o
  riparato: è il comportamento voluto (fail closed su path morti), ma va detto a chi costruisce in
  locale. Rigenerazione: `TMPDIR=<volume di build> scripts/setup_bundled_pyannote.sh --force`.
- Nessun avvio app, nessun server, nessun Keychain, nessun download di modelli grandi (Whisper/Parakeet),
  nessuna modifica a impostazioni di sistema, VPN, firewall, telefono o SIM.
- Nessun commit, stage, push, tag, dispatch, promozione o worktree. Nessun fake product proof locale.

## 9. Azioni per root prima del dispatch

1. Review dei 14 file (11 modificati + 3 nuovi) e aggiornamento di `REVIEWED_PRODUCT_SHA` in `.github/workflows/release.yml` al
   commit di questa slice (nessun altro pin va toccato).
2. **Wiring dei test nuovi in CI** (fuori da questa slice per mandato): aggiungere
   `scripts/test_setup_pyannote_staging.py` allo step `Verify native Pyannote portability tooling`
   (`ci.yml:147-148`). Senza questo, la regressione di staging non è presidiata in CI — il file esiste ed
   è verde, ma nessun workflow lo esegue.
3. Opzionale, non richiesto da questa slice: `docs/REPOSITORY_STATE.md` cita ancora 2.0.37.
4. Dopo il commit: tag annotato `v2.0.38` sul commit revisionato, dispatch del candidato, poi verifica del
   primo run nativo su macOS (entrambe le architetture) e Windows.

## 10. Evidenze (sha256)

Directory base: `/Volumes/UltraDisk/sbobino-companion-product-20261115/packaging-2038/`

| File | sha256 |
| --- | --- |
| `redgreen/RED.log` | `d5c6e8f1e633e70628e8ffafc55869b426edee3d56bc541bbd77dac99512889d` |
| `redgreen/GREEN.log` | `9adae2fc6e9b7e899c92e1b2808dad6f8928adf0620aec020f90a88fe736fb51` |
| `redgreen/RED-HEAD-original-suite.log` | suite macOS preesistente rossa a `HEAD` (1 failure, 5 error) |
| `audit-original-r2.json` | `c570d197b4043b2a668c3d35fb1efef0f9125b542d0f75450de2322b3e7b3fe2` |
| `proof/audit-pre.json` | `d92455b61eaf6b465f0a94a7c0219cae61e6510a4eea47931890405766637a84` |
| `proof/audit-post.json` | `a89a81b38166b2b86a3eef997b063a7c2919982c00842fa6f46231a452d82f9d` |
| `proof/sanitize-receipt.json` | `afa24f7e810ede07572ce5e3941f30b631f6b36962a8707a47de8071e50fde7c` |
| `proof/sanitize-receipt-second.json` | `5154da859b9110f4c8671acbd0ba562f8eeeab1c95eeb98c9b39725c536a1353` |
| `proof/inventory-after-sanitize.json` | `5a43cb39bd7ea116a0ca972a821aba1ee50c8d1ff13959e86fbfe5b9a5ed4858` |
| `proof/inventory-after-sanitize-second.json` | `8d4f04587a8e1a74f65a53562a99d5a8342e4afe48b3f138e7caba1d152d74e5` |
| `proof/relocation-imports.log` | `61bce5d15c7d464d87cab2daab559ad28fc1974ec66a7860e632ce44f61f6126` |
| `proof/inventory-relocated-final.json` | `2ca7b2c3aa6998f4cd8ef9d3379bb0f8a000b2655578e9518a7cfea0ddc1ae72` |
| `proof/relocation-imports-final.log` | `009112cd6a013880e8534b3a2f2d315f8870b5cb58823ffb34cbf516f36b4ef8` |
| `proof/sanitize-final-receipt.json` | `9741eded8c039c9b7f36ae80a509f15b96fe8836e3bfa7d0877c03655d02303d` |
| `proof/diarize-tts-result.json` | `e6af6341e09ef8094355715c06b7c6767a3d21670253b7e0024156f6007f6e0b` |
| `proof/audit-regression-injected.json` | `e1b0292822b246d3b27d114c291469c1adf83da50635b675e81c234c1f36f949` |
| `proof/audit-regression-sanitize.json` | `120e91fbe048f3a09002b504e0489799530813240af3526197f702bd863999c5` |
| `proof/audit-relocated-final.log` | `767b80986484b905f38edc9899e45078dad1ed1f04995f8c934124a101687555` |
| `measure-r2-original.log` | `0456e307cc424377b0e004771332fbab08a56fb9935dcf9cf5fe74d461a2b1a8` |

Tree hash: originale R2 `afd03b07d34bb525bf22652ee1b0607f67c819d6ee85f45ff2a5db13a6a16d5d`;
dopo sanitize (stabile su due giri) `8be550398a176110d29ab226fa60ba3f7751d9b00e7364004ea5732de9c4fb64`.

## 11. Fonti

- Apple, *Run-Path Dependent Libraries* — https://developer.apple.com/library/archive/documentation/DeveloperTools/Conceptual/DynamicLibraries/100-Articles/RunpathDependentLibraries.html
- Apple Developer Forums, thread 736719 (`@rpath`/`@loader_path` e install name) — https://developer.apple.com/forums/thread/736719
- LLVM, *llvm-install-name-tool* — https://www.llvm.org/docs/CommandGuide/llvm-install-name-tool.html
- `man mktemp` macOS 13.6.5 (BSD): `-d` ≡ `-t tmp`, template da `_CS_DARWIN_USER_TEMP_DIR` — https://man.freebsd.org/cgi/man.cgi?apropos=0&manpath=macOS+13.6.5&query=mktemp&sektion=1
- `otool(1)` / `lipo(1)` / `codesign(1)` locali: output per architettura e verifica strict del bundle.

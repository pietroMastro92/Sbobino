# Companion — riparazione locale del packaging Pyannote (2.0.38)

Data: 2026-11-15 · Slice: packaging locale (fix + evidenza locale completa) · **nessun dispatch, nessuna PR, nessun tag, nessun upload, nessun bump di versione**

Repo: `/Users/pietromastro/Documents/sbobino_tauri` @ `31d832b7083ba86588774c0230a9bf7d5996d5a7` (`main`)
Hold attivo: [`NO-GITHUB-DISPATCH.md`](../../../Volumes/UltraDisk/sbobino-companion-product-20261115/local-release-repair/NO-GITHUB-DISPATCH.md) — invariato.
Lane esterna (tutto lo scratch, i log e i payload): `/Volumes/UltraDisk/sbobino-companion-product-20261115/local-release-repair/packaging/`.

Esito: **tre difetti reali del percorso di release riparati e verificati localmente**, catena completa
(hydration nativa reale da sorgenti originali → sanitize → audit → packager ufficiale → entrypoint di prodotto →
rilocazione con spazi nel path → firme) eseguita e verde. Restano esplicitamente **non verificati** Intel runtime,
Windows, build Tauri/Cargo e la corsa hosted (hold intatto).

---

## 1. Ambito e stato del checkout

| Voce | Valore |
| --- | --- |
| HEAD | `31d832b7083ba86588774c0230a9bf7d5996d5a7` |
| File toccati da questa lane | `sbobino_desktop/scripts/lib/pyannote_portable_runtime.py`, `sbobino_desktop/scripts/setup_bundled_pyannote.sh`, `sbobino_desktop/scripts/test_pyannote_portable_runtime.py`, `sbobino_desktop/scripts/test_setup_pyannote_staging.py`, nuovo `sbobino_desktop/scripts/local_packaging_gate.sh` |
| File NON toccati | Rust (`companion.rs`, `fluid_diarization_runtime.rs`: hash root `c7017471…` / `ace61dfb…` preservati), workflow YAML, versioni pacchetto (restano 2.0.38), `apps/desktop/src-tauri/resources/pyannote` (snapshot tracciato), altri report |
| Non eseguito per mandato | `cargo`/`tauri build` (lane root), Windows, runtime Intel |

Hash sorgente (sha256, working tree) in `evidence/environment.txt`; modulo baseline (= HEAD):
`f58ae438061c8d06cc4e84f91684e6931251e05b87b91b64e05a206488527ce8`.

### Ambiente locale vs hosted

| | Locale | Hosted (log 2038) |
| --- | --- | --- |
| macOS | 27.2 (26B123) | 15 (runner `macos-15`) |
| Xcode / cctools | Xcode 27.2, `cctools-1040` | Xcode 16.4 |
| Python ARM di release | python.org CPython **3.11.9 universal2** (payload ufficiale del pkg) | idem (installato in `/Library/Frameworks`) |
| Python "ordinario" | Homebrew `python@3.11` 3.11.16 | — |

---

## 2. Difetto A — `install_name_tool -id` su un oggetto LLVM bitcode universale

### Causa root (misurata)

`python.o` del runtime python.org universal2 3.11 è un **container universale** (`0xcafebabe`) le cui **due slice
non sono Mach-O**: sono wrapper **LLVM bitcode** (`0x0b17c0de`). `otool` esce **0** e stampa una diagnostica dove
starebbero i load command:

```
python.o (architecture x86_64): is an LLVM bit-code file
python.o (architecture arm64):  is an LLVM bit-code file
```

`is_macho()` guardava solo i primi 4 byte (magic del container) e il parser trattava la riga diagnostica come
`LC_ID_DYLIB`; il sanitizer pianificava quindi `-id @rpath/python.o`, che `install_name_tool` rifiuta:
`input file … is not a Mach-O file`. Esattamente il job ARM del run 2038
(`2026-11-15T08:15:22Z`, step `Hydrate native Pyannote assets`, subito dopo `Bundling portable Python native dependencies`).

### RED riprodotto (stesso input ufficiale, stesso punto della catena)

Testa stretta (stesso file reale, modulo pre-fix vs post-fix), `packaging/red1` e `packaging/green1`:

```
$ python3 run_portable.py pre-fix/pyannote_portable_runtime.py sanitize red1
command failed: /usr/bin/install_name_tool -id @rpath/python.o …/red1/python.o (exit 1)
error: …/install_name_tool: input file: …/red1/python.o is not a Mach-O file

$ python3 run_portable.py <working-tree module> sanitize green1
sanitized 0 Mach-O, … skipped 1 non-Mach-O container(s)
```

Catena completa RED (`run-p-red/logs/hydrate.log`): `setup_bundled_pyannote.sh --force` con il **modulo pre-fix**
e il framework python.org 3.11.9 reale; il modello viene clonato da HF, `pyannote.audio==4.0.4` + torch/torchaudio/
torchcodec vengono installati davvero, stdlib/libpython embeddati, e si arriva alla stessa riga:

```
298: Bundling portable Python native dependencies
298: command failed: /usr/bin/install_name_tool -id @rpath/python.o …/tmp.nAz7vvTb88/python/aarch64-apple-darwin/lib/python3.11/config-3.11-darwin/python.o (exit 1)
299: error: … input file: …/config-3.11-darwin/python.o is not a Mach-O file
```

### Fix

Nel modulo condiviso (unica fonte per generatore, packager e repair tool):

* **parsing reale del container** (`read_macho_container`): thin/fat32/fat64, entrambe le endianness, bounds check
  su header, tabella, offset/size, slice sovrapposte, conteggio architetture; ogni slice classificata
  `macho` / `llvm_bitcode` / `unknown` dal **proprio** header (`cputype`, `cpusubtype`, `filetype`).
* **`is_macho` / `iter_macho` / `scan_macho_payload`**: un container con sole slice bitcode non è Mach-O; un
  container **misto** (Mach-O + non) è un errore fail-closed elencato (`mixed_slice_macho`), non un rewrite parziale.
* **filtro diagnostiche** (`otool_diagnostics`): le righe `… is an LLVM bit-code file` / `… is not a Mach-O file`
  non possono più diventare load command; se compaiono per una slice dichiarata Mach-O il tool **fallisce**.
* **`LC_ID_DYLIB` solo per slice `MH_DYLIB`**: un file `.o` non inventa un install name (deciso dall'header, non da
  una risposta vuota del tool).
* **audit/sanitize/receipt**: il container non-Mach-O è **disclosed** (`non_macho_container` warning; `skipped_non_macho`
  nel receipt), il file resta **byte-identico** e non viene mai firmato.

### GREEN

`run-p-green/logs/hydrate.log` (catena completa, stesso comando di release):

```
Using Python executable: …/fixtures/pyorg-framework/Versions/3.11/bin/python3.11   (3.11.9 universal2)
sanitized 95 Mach-O, removed 57 rpath(s), repaired 42 install name(s) and 0 dependency reference(s),
          copied 0 dylib(s), signed 95 binary/binaries and 0 bundle(s), skipped 1 non-Mach-O container(s)
pyannote portable runtime audit … macho: 447, signed ok: 438, warnings: 151, violations: 0 → AUDIT OK
pyannote pipeline loaded successfully
```

`python.o` nel payload ha lo stesso sha256 dell'input python.org:
`d9e9a239850a6975208c60337228828921ea196038d992079fbd628f1a2189ed`.

---

## 3. Difetto B — install name per-architettura perso sull'host Intel (CI main)

### Causa root

`read_install_names` dipendeva dal **dump multi-architettura di una singola invocazione** `otool -D`. Sul runner
`macos-15-intel` quel dump riportava solo la slice x86_64, quindi l'id arm64 non arrivava né al test né al receipt
(`test_install_names_are_reported_per_architecture`, `test_sanitize_rewrites_every_slice_and_stays_green`).

### Fix

Ogni slice viene interrogata **esplicitamente** con `otool -arch <arch>` (flag di selezione architettura, indipendente
dall'host), con:
* nomi arch da `lipo -archs`, **accoppiati per indice** e **incrociati** con il `cputype` del nostro header
  (disallineamento ⇒ errore, mai query sulla slice sbagliata);
* **stdout vuoto per una slice Mach-O = errore** (mai "nessun install name"): è proprio il modo silenzioso in cui
  `otool -arch` sbagliata non stampa nulla.

### Evidenza

* RED (modulo pre-fix, fixture fat reale): `test_a_native_only_otool_dump_does_not_hide_an_architecture` **FAIL** —
  il test simula il dump "solo x86_64" misurato sull'hosted Intel e pretende entrambi gli id.
* GREEN: stesso test passa; `test_intel_query_mode_reads_every_slice_of_a_universal_binary` esegue il **vero**
  reader sotto `arch -x86_64 /usr/bin/python3` (Rosetta, "Intel query mode") su un fat Mach-O reale e legge
  entrambe le slice.
* Non riproducibile localmente: il comportamento del `otool` di Xcode 16.4 su `macos-15-intel` (nessun host Intel
  disponibile, nessuna VM scaricata). Il fix non dipende più da quel comportamento.

---

## 4. Difetto C (nuovo, trovato dalla catena completa) — le copie "embedded" non venivano sanificate

### Causa root

Il sanitizer copiava i dylib host-managed in `lib/embedded-dylibs`, ne riscriveva l'`-id`, ma **non sanificava la
copia**: `libssl.3.dylib` continuava a linkare `…/libcrypto.3.dylib` in assoluto e a esporre un `LC_RPATH` host.
L'audit (e il validatore del companion, `validate_portable_pyannote_native_dependencies`) rifiuta esattamente
questo. Misurato con l'hydration **ordinaria** (Homebrew python@3.11), che è l'unica che raggiunge quel ramo:

```
VIOLATION host_rpath: lib/embedded-dylibs/libsqlite3.dylib -> /opt/homebrew/Cellar/sqlite/3.53.4/lib
VIOLATION host_dependency: lib/embedded-dylibs/libssl.3.dylib -> /opt/homebrew/Cellar/openssl@3/3.6.4/lib/libcrypto.3.dylib
```

Lo stesso schema esiste sul runtime python.org (`_ssl` → `libssl`/`libcrypto` sotto `/Library/Frameworks`): senza
questo fix la release ARM avrebbe fallito **dopo** aver speso l'intera hydration (il finding sarebbe emerso a
`assert_python_native_runtime_is_portable`).

### Fix

`sanitize_runtime` diventa un **fixpoint** (max 4 passi, altrimenti errore): ogni passo è plan-then-mutate, quindi le
copie appena embeddate sono viste dal passo successivo e sanificate come qualsiasi altro binario del payload
(dipendenze host → nuovo embed, LC_RPATH host/assolute rimosse, firma). Inoltre, collisione di basename fra due file
**realpath diversi** ⇒ fail-closed; lo stesso file raggiunto via `opt/…` e `Cellar/…` è lo stesso realpath e passa.
Il secondo `sanitize` resta un no-op a livello di byte (`passes: 0`).

### Evidenza

* RED (pre-fix): `HostManagedEmbeddingTests` — la copia embedded conserva dipendenza host e compare
  `host_dependency`; il test fallisce.
* GREEN: due embed (`libsbobino_host`, `libsbobino_dep`), `passes >= 2`, rpath host rimosse, dipendenze
  `@loader_path/…`, audit verde, e il client continua a stampare `5` dopo lo spostamento del payload.
* Catena completa: rilancio di `run-h-green.sh` (Homebrew) → EXIT 0:

```
sanitized 96 Mach-O, removed 60 rpath(s), repaired 34 install name(s) and 7 dependency reference(s),
          copied 5 dylib(s), signed 102 binary/binaries and 1 bundle(s), skipped 0 non-Mach-O container(s)
… macho: 443, signed ok: 434, warnings: 150, violations: 0 → AUDIT OK
pyannote pipeline loaded successfully
```

Copie embeddate misurate nel payload finale (`evidence/embedded-dylibs-homebrew.txt`):

```
libssl.3.dylib    id=@rpath/libssl.3.dylib    rpaths=            deps=… @loader_path/libcrypto.3.dylib …
libcrypto.3.dylib id=@rpath/libcrypto.3.dylib rpaths=            deps=… /usr/lib/libSystem.B.dylib
libsqlite3.dylib  id=@rpath/libsqlite3.dylib  rpaths=            deps=… /usr/lib/libz.1.dylib …
```

---

## 5. Modifiche puntuali

### `scripts/lib/pyannote_portable_runtime.py` (unica fonte condivisa)

| Area | Prima | Dopo |
| --- | --- | --- |
| appartenenza Mach-O | magic del container | header **per slice**, bounds check, `filetype`/arch reali |
| container misti / troncati | trattati come Mach-O | `mixed_slice_macho` / `malformed_macho_container`, fail-closed |
| diagnostiche `otool` | interpretate come load command | filtrate + errore se su slice dichiarata Mach-O |
| install name | dump multi-arch singolo | `otool -arch <arch>` per slice, accoppiato a `lipo -archs` e incrociato col `cputype` |
| `-id` | anche su oggetti senza `LC_ID_DYLIB` | solo slice `MH_DYLIB` |
| embedding host | copia + `-id` | fixpoint (≤4 passi), coll. basename fail-closed, copie sanificate e firmate |
| inventory | `type: file` | + metadati `macho` (formato, arch, filetype per slice) |
| receipt | — | + `skipped_non_macho`, `passes` |

Nessun allentamento delle regole esistenti: `HOST_PREFIXES`, `SYSTEM_PREFIXES`, marker personali, fail-closed su
riferimenti ambigui/esterni, ordine di firma (Mach-O interni → bundle `.app`), idempotenza.

### `scripts/setup_bundled_pyannote.sh`

* nuova opzione **`--resource-root DIR`** (default invariato: `apps/desktop/src-tauri/resources/pyannote`), con
  guardia personal-path sul path richiesto **e** sul path risolto, rifiuto di path relativi/root/file, e nessuna
  creazione prima della convalida. Serve a eseguire e verificare la hydration reale **senza mutare le risorse
  tracciate dell'app**.
* `replace_runtime_dir()`: il `rm -rf` del runtime passa da una guardia fail-closed (path assoluto, non `/`, dentro
  `RESOURCE_ROOT`).
* `--stage-dir-probe` stampa anche `resource_root=` (contract test, nessun download).

### Test aggiunti (nei file già eseguiti dalla CI)

`test_pyannote_portable_runtime.py`: `MachOContainerParsingTests` (thin/fat32/fat64, endianness, bounds,
sovrapposizioni, conteggi implausibili, bitcode), `NonMachOContainerTests` (il caso 2038: nessun -id, nessun
codesign, byte-identico, audit non fallisce, container misto fail-closed), `HostManagedEmbeddingTests` (fixpoint di
embedding + caricamento reale dopo spostamento), `HostArchitectureQueryTests` (dump "solo nativo" + Intel query mode
reale). `test_setup_pyannote_staging.py`: `ResourceRootContractTests` (default invariato, override onorato, rifiuto
di path relativo/personale, guardia sul `rm -rf`).

### Nuovo `scripts/local_packaging_gate.sh`

Gate locale **una-comando**, nessun dispatch: preflight (host arm64, toolchain, ≥12 GiB, path guard, work dir fuori
dal checkout) → i tre entrypoint unitari → hydration **reale** (`--force`) in una root esterna → packager ufficiale,
audit strict, entrypoint di prodotto su parlato reale, riordini di import, relocation con spazi, firme, provenienza
`python.o`. Opzioni: `--unit-only`, `--verify-only DIR`, `--source-framework`, `--interpreter-dir`, `--work-dir`
(obbligatoria). Riepilogo macchina in `evidence/gate-summary.txt`.

Nota di processo: la prima esecuzione completa del gate ha prodotto tutte le verifiche verdi ma un exit code
`FAIL`, perché una riga `INFO` (interprete risolto) veniva contata come fallimento. Bug del gate corretto
(`INFO` non è fatale) e gate rieseguito da zero: `GATE RESULT: PASS`.

---

## 6. Comandi (esatti)

```bash
# gate completo, una comando (root può rilanciarlo così)
cd /Users/pietromastro/Documents/sbobino_tauri/sbobino_desktop
./scripts/local_packaging_gate.sh \
  --work-dir /Volumes/UltraDisk/sbobino-companion-product-20261115/local-release-repair/packaging/gate-final \
  --interpreter-dir /Volumes/UltraDisk/sbobino-companion-product-20261115/local-release-repair/packaging/fixtures/pyorg-framework/Versions/3.11/bin \
  --source-framework /Volumes/UltraDisk/sbobino-companion-product-20261115/local-release-repair/packaging/fixtures/pyorg-framework/Versions/3.11

# solo unit (secondi)
./scripts/local_packaging_gate.sh --work-dir /Volumes/UltraDisk/…/packaging/gate-unit --unit-only
```

Hydration completa equivalente a `release.yml` (`Hydrate native Pyannote assets`: `--force`, `TMPDIR=$RUNNER_TEMP`):

```bash
cd /Users/pietromastro/Documents/sbobino_tauri/sbobino_desktop
TMPDIR=/Volumes/UltraDisk/…/packaging/run-p-green/tmp \
PIP_CACHE_DIR=/Volumes/UltraDisk/…/packaging/cache/pip \
PATH=/Volumes/UltraDisk/…/packaging/fixtures/pyorg-framework/Versions/3.11/bin:$PATH \
bash scripts/setup_bundled_pyannote.sh --force --python-version 3.11 \
  --resource-root /Volumes/UltraDisk/…/packaging/run-p-green/output/pyannote
```

Verifica del payload prodotto (packager ufficiale, audit, entrypoint, relocation, firme, provenienza):
`packaging/verify_payload.sh <resource-root> <evidence-dir> <framework-dir>`.

L'input python.org è il payload ufficiale `python-3.11.9-macos11.pkg` (`work/pyorg/py311.pkg`, sha256 in
`hosted-candidate-2038/evidence/evidence-sha256.txt`) espanso con `pkgutil --expand-full`. Poiché il framework non è
installato in `/Library/Frameworks` (installazione di sistema fuori mandato) è stata preparata una copia privata
eseguibile, con due sole classi di aggiustamento, tutte dentro la lane:

1. `LC_LOAD_DYLIB` dei **tre launcher** (`bin/python3.11`, `bin/python3.11-intel64`,
   `Resources/Python.app/Contents/MacOS/Python`) puntato a uno shim con la stessa forma
   `…/Python.framework/Versions/3.11/Python` (cambia solo il prefisso, così lo script di release prende **lo stesso**
   ramo di rewrite che prende su un'installazione reale);
2. i dylib **vendored** del framework (openssl/ncurses/tcl) copiati in
   `lib/python3.11/lib-dynload/python-org-libs/` con riferimenti `@loader_path`, perché in `/Library/Frameworks`
   localmente non esistono; su un'installazione reale il sanitizer li embedda da `/Library/Frameworks` (ramo
   coperto dal run Homebrew del §4).

Before/after sha256 per ogni file toccato in `logs/pyorg-fixture-provenance.txt`. Contenuto, versioni e `python.o`
non sono stati alterati (`d9e9a239…` prima e dopo, verificato anche nel payload finale dal gate).

---

## 7. Evidenza locale (eseguita)

| Gate | Comando | Esito |
| --- | --- | --- |
| RED stretto (python.o reale) | `run_portable.py pre-fix/… sanitize red1` | **RED** atteso, messaggio identico all'hosted |
| RED catena completa | `run-p-red.sh` (modulo pre-fix + python.org) | **RED** atteso alla stessa riga del run 2038 |
| GREEN catena completa (python.org universal2) | `run-p-green.sh` | **PASS** — 95 Mach-O sanificati, 447 audit, 0 violazioni, pipeline caricata |
| GREEN catena completa (Homebrew 3.11.16, ramo embedding) | `run-h-green.sh` | **PASS** (exit 0) — 96 Mach-O, 5 dylib embeddati e sanificati, 443 audit, 0 violazioni, pipeline caricata |
| Gate una-comando (hydration reale + verifiche) | `local_packaging_gate.sh --work-dir … --interpreter-dir … --source-framework …` | **PASS** — hydration 246 s, 27 righe PASS, 0 FAIL, sul patch finale |
| Packager ufficiale | `package_pyannote_asset.sh` sui due payload freschi | **PASS** — 408 MiB / 102 183 entry (python.org) e 397 MiB / 101 973 entry (Homebrew) |
| Entrypoint di prodotto | `pyannote_diarize.py` su parlato (`say`→16 kHz mono) | **PASS** — 1 turno rilevato, sia in place sia dopo relocation con spazi |
| Riordini di import | `pyannote.audio→torchcodec` e `torchcodec→pyannote.audio` | **PASS** in entrambe le posizioni |
| Firme | audit + `codesign --verify --strict` su `bin/python3`, `libpython3.11.dylib`, `Python.app` | **PASS** — 438 Mach-O verificati, 0 invalide, 9 oggetti wheel non firmati (disclosed) |
| Provenienza input | sha256 `python.o` input vs payload | **PASS** — identici |
| Non regressione sull'asset 2.0.36 in repo | audit read-only modulo pre-fix vs post-fix | **identico**: 434 binari, 134 violazioni (52 `absolute_rpath`, 52 `personal_path_reference`, 28 `non_portable_install_name`, 2 `host_rpath`), 158+11 warning |
| Unit packaging/staging/review | `test_pyannote_portable_runtime.py`, `test_setup_pyannote_staging.py`, `test_pyannote_review_regressions.py` | **PASS** — 61 + 16 + 5 = **82 test** |
| Contract release/workflow | `test_release_workflow_contract.py` (30, 1 skip), `test_macos_bundle_scripts.py` (1), `check_macos_runtime_source.sh`, `check_no_skipped_tests.sh`, `check_release_versions.sh` | **PASS** — nessun file di workflow modificato |
| Intel query mode | `arch -x86_64 /usr/bin/python3` sul reader reale + test con dump "solo nativo" | **PASS** |

Warning disclosure (payload python.org, `run-p-green/verify/audit.json`): 141 `unresolved_rpath_dependency`
(torch/torchcodec: `@rpath/libtorch*.dylib`, `libc10`, `libavutil.60`, `libpython3.11` — risolti a runtime dal
contesto dell'eseguibile, non risolvibili staticamente), 9 `unsigned_macho` (oggetti wheel coperti dalla firma del
bundle app), 1 `non_macho_container` (`python.o`). **Nessuno di questi è stato riclassificato "closure strict": la
closure stretta non è dichiarata verde.**

---

## 8. Limiti e ciò che resta non verificato

1. **Nessuna corsa hosted**: hold `NO-GITHUB-DISPATCH` rispettato (nessun commit/push/tag/dispatch/rerun/upload).
2. **Runtime Intel (x86_64)** non ricostruito: la lane Intel usa conda/torchcodec compilato e richiede un host Intel
   (nessuna VM scaricata). Il contratto per-arch è coperto dai test, non da un payload Intel reale.
3. **Windows**: fuori ambito (script `package_windows_*` non toccati).
4. **Build Tauri/Cargo e app finale**: di root. Questa lane non ha eseguito `cargo` né `tauri build`.
5. **Xcode**: locale 27.2 (`cctools-1040`) vs hosted 16.4; il fix non dipende più dal dump multi-arch, ma il
   comportamento esatto dei tool 16.4 su `macos-15`/`macos-15-intel` non è riproducibile qui.
6. **Fixture python.org**: il framework non è installato in `/Library/Frameworks`; gli aggiustamenti sono limitati e
   documentati (§6). Un hosted run reale usa l'installazione ufficiale senza aggiustamenti.
7. **Closure stretta** non verde (warning torch/torchcodec), come già documentato nel modulo.
8. **Coerenza temporale dell'evidenza**: il run `run-h-green` (ramo embedding) è precedente all'ultima modifica,
   che tocca solo `inventory_macho_metadata` (metadato del manifest, non usato né da `sanitize` né da `audit`);
   il gate finale è stato rieseguito **dopo** quella modifica, e i test sono stati rieseguiti sull'hash definitivo
   `d0bed6d7…` (`evidence/local-test-runs.txt`).

---

## 9. Criterio per togliere l'hold (proposta, decisione di root)

Tutti i punti sotto sono **locali** e già verdi tranne quelli marcati:

- [x] RED riprodotto con input ufficiale e stesso punto della catena
- [x] GREEN della catena completa (idratazione reale + packager + audit + entrypoint + relocation + firme)
- [x] secondo GREEN con l'interprete ordinario (ramo embedding host)
- [x] 82 test packaging + contract release/workflow, nessuna modifica ai contratti
- [x] non regressione misurata sull'asset 2.0.36 in repo
- [ ] build Tauri nativa completa (root, dopo lane Rust + questa)
- [ ] (facoltativo, hosted) prima corsa reale: ARM pistA e Intel/Windows restano da misurare

---

## 10. File di evidenza

```
/Volumes/UltraDisk/sbobino-companion-product-20261115/local-release-repair/packaging/
├── evidence/environment.txt              # hash sorgenti, HEAD, tool versions
├── evidence/local-test-runs.txt          # 82 test + contract release/workflow
├── evidence/embedded-dylibs-homebrew.txt # copie embedded sanificate (fixpoint su payload reale)
├── evidence/hydrate-provenance.txt       # pip cache, modello, righe chiave dei run
├── logs/pyorg-fixture-provenance.txt     # before/after sha256 della fixture python.org
├── pre-fix/pyannote_portable_runtime.py  # modulo baseline (HEAD) usato per i RED
├── run_portable.py                       # esecutore CLI su un modulo esplicito
├── prepare_pyorg_fixture.sh              # costruzione della fixture eseguibile
├── verify_payload.sh                     # verifica payload (packager/audit/entrypoint/relocation/firme)
├── run-p-red.sh  run-p-red/logs/hydrate.log
├── run-p-green.sh run-p-green/logs/hydrate.log  run-p-green/verify/
├── run-h-green.sh run-h-green/logs/hydrate-fixed2.log  run-h-green/output/  run-h-green/verify/
├── gate-final/evidence/20261115T100256Z/ # esito del gate una-comando sul patch finale (PASS, hydration reale)
└── gate-verify/evidence/20261115T092601Z/ # gate in modalità --verify-only (PASS)
```

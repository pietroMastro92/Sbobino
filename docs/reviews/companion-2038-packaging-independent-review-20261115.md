# Companion 2.0.38 — packaging: review indipendente (read-only)

Data: 2026-11-15 · Reviewer: DeepSeek OpenCode Go (sub-agente di review, nessun sub-agente)
Repo: `/Users/pietromastro/Documents/sbobino_tauri` @ `2aebad88f52a77cf510cb335602eecf57229609f`
Delta revisionato: 12 file modificati (`git diff --stat`: 1094 insertions, 127 deletions) + 3 file nuovi
(`sbobino_desktop/docs/release-notes/v2.0.38.md`, `sbobino_desktop/scripts/test_setup_pyannote_staging.py`, questo file).
Input: `docs/reviews/companion-2038-packaging-fix-20261115.md`, `packaging-2038/{proof,redgreen}`.
Scratch (fuori repo, APFS, CoW): `/Volumes/UltraDisk/sbobino-companion-product-20261115/packaging-2038-independent-review/`.

**Nessuna modifica a produzione, CI, versioni, tag, artefatti preesistenti o dati personali.**
Nessun commit/stage/push/tag/dispatch/promote/worktree. Nessun avvio app, server, telefono, SIM, LLM,
Keychain, VPN, firewall, download di modelli o profili di rete. L'unica scrittura nel repo è questo file.

---

## 0. Esito

Il fix di prodotto (parser `otool`, audit, sanitizer, staging) è **indipendentemente verificato e
riprodotto bit-per-bit**: la mia sanitize di una copia fresca del payload R2 produce esattamente l'albero
dell'implementer (29205/29205 sha256 e `tree_sha256 8be55039…` identici). Nessun P1 nel codice di prodotto.

Restano **1 P1 di wiring di release** (fuori dal prodotto, azione root: il report §9 dice "nessun altro
pin va toccato", ma tre workflow sono ancora bloccati su `2.0.37`) e **4 P2** (ordine del guard di
staging, due lacune del modello di chiusura dell'audit, aliasing introdotto dal rewrite degli ID).

Nessuno dei P2 impedisce il run hosted arm64; il P1 sì, se root segue la checklist del report alla lettera.

---

## 1. Cosa ho riprodotto (numeri miei, non ereditati)

Censimento dei load command fatto con un **parser Mach-O byte-level proprio** (`work/macho_census.py`),
non con `otool` testuale e non con il parser in review. 449 Mach-O, 0 errori di parsing.

| Misura | Implementer | Mia misura indipendente |
| --- | --- | --- |
| Mach-O nel payload | 449 | **449** |
| File fat | 11 universali (2 slice) | **12 fat** = 11 a 2 slice + `scipy/.../libgcc_s.1.1.dylib` a 1 slice (spiega esattamente gli 11 header fantasma: un fat a 1 slice stampa solo l'header di riga 1, che il vecchio parser saltava) |
| ID assoluti non di sistema (R2) | 26 (`/DLC` ×25, `/opt/llvm-openmp` ×1) | **26**, stessi path |
| ID relativi non `@` (R2) | 2 (`bazel-out/...`, per-arch, stesso `_message.abi3.so`) | **2**, stesso file |
| Dipendenze assolute non di sistema (R2) | 0 | **0** |
| rpath assoluti non di sistema (R2) | 0 | **0** |
| Violazioni audit su R2 col codice nuovo | 28 | **28** (`audit-original-r2-newcode.json`) |
| Violazioni audit post-sanitize | 0 | **0** (96 warning) |
| `modified` / ID riparati / dep / rpath | 27 / 28 / 0 / 0 | **27 / 28 / 0 / 0** |
| Seconda sanitize | 0 modifiche, tree identico | **0 modifiche, 0 file diversi su 29205** |
| `tree_sha256` post-sanitize | `8be55039…` | **`8be550398a176110d29ab226fa60ba3f7751d9b00e7364004ea5732de9c4fb64`** |
| Test suite (50) | OK | **OK in 8.2 s** (TMPDIR in scratch, `SBOBINO_*_TEST_ROOT` impostati) |

Unica differenza di *conteggio*: il censimento dell'implementer deduplica per file, il mio per slice
(74 vs 73 ID `@rpath`, 683 vs 668 dep di sistema). Ricalcolando per-file dai miei dati ottengo
**73 / 668 / 210 / 77 / 37**, cioè gli stessi numeri del report: nessuna discrepanza, solo unità di misura.

RED autentico: `redgreen/RED.log` fallisce per API assenti (`payload_file_index`, `read_install_names`),
`--stage-dir-probe` sconosciuto, `mktemp` senza template — non per test artefatti.

---

## 2. Findings

### P1 — Il dispatch `v2.0.38` fallisce ancora: tre workflow sono hard-locked a `2.0.37`

- **Trigger**: root aggiorna `REVIEWED_PRODUCT_SHA` come da report §9 ("nessun altro pin va toccato"),
  crea il tag `v2.0.38` e fa dispatch di `release.yml` con `tag_name=v2.0.38`.
- **Conseguenza**: `validate-candidate` esce 1 al primo step, prima di costruire qualsiasi cosa. È
  esattamente il tipo di run fallito che questa slice doveva chiudere. `promote-release.yml` ha lo stesso
  lock (fallirebbe dopo), e `verification-packages.yml` fallisce il check di coerenza versione.
- **File:line**:
  - `.github/workflows/release.yml:9` (`default: v2.0.37`), `:17` (nome job), **`:25-26`
    (`if [[ "$TAG_NAME" != "v2.0.37" ]] … exit 1`)**
  - `.github/workflows/promote-release.yml:9`, **`:31-32`** (stesso lock)
  - `.github/workflows/verification-packages.yml:95` (`check_release_versions.sh 2.0.37`)
- **Prova eseguibile**:
  ```
  $ grep -rn 'v2\.0\.37' .github/workflows
  release.yml:9 / :17 / :25
  promote-release.yml:9 / :31
  $ bash sbobino_desktop/scripts/check_release_versions.sh 2.0.37
  Expected version '2.0.37' but found '2.0.38'.        # exit 1
  ```
  `python3 scripts/test_release_workflow_contract.py` → 29 OK (1 skipped), quindi **il contratto CI non
  copre questo lock**: verde oggi non significa dispatchabile domani.
- **Nota di scope**: non è un difetto del delta di prodotto; è una correzione obbligatoria della
  checklist root. Il report deve dire "aggiornare `REVIEWED_PRODUCT_SHA` **e** i lock/tag/default dei tre
  workflow", non "nessun altro pin". Consiglio (root): aggiungere un'asserzione al contract test che il
  lock del workflow coincida con la versione prodotto.

### P2 — Ordine del guard di staging: viene creata una directory dentro l'area protetta *prima* del rifiuto

- **Trigger**: `TMPDIR` assoluto che attraversa un symlink dentro un root protetto, con l'ultimo
  componente inesistente (es. `…/tmplink/newdir`, `tmplink -> ~/Library/Application Support`).
- **Conseguenza**: `stage_parent_dir` esegue `guard-root` sul testo (`:130`, passa: nessun marker),
  poi `tmp_parent_dir` fa `mkdir -p "$requested"` (`:108`) **attraverso il symlink**, poi `guard-root`
  sul path canonico (`:137`) rifiuta. La directory è già stata creata dentro l'area protetta.
  Contraddice il report §3.1 ("applica il guard personale **prima** di creare qualunque cosa").
  Nessuna cancellazione, nessun accesso a Keychain: solo creazione di directory.
- **File:line**: `sbobino_desktop/scripts/setup_bundled_pyannote.sh:108` (mkdir) chiamato da `:136`,
  tra i due guard a `:130` e `:137`.
- **Prova eseguibile** (scratch, finto `Library`):
  ```
  $ TMPDIR="$PWD/ce/guard/tmplink/newdir" bash setup_bundled_pyannote.sh --stage-dir-probe
  refusing personal/lockbox path '…/fakehome/Library/Application Support/newdir' …
  staging parent '…/Library/Application Support/newdir' resolves to a personal/lockbox path …
  exit=1
  $ find ce/guard/fakehome/Library -maxdepth 3
  …/Library
  …/Library/Application Support
  …/Library/Application Support/newdir      <-- creato prima del rifiuto
  ```
  Il caso diretto (`TMPDIR=~/Library/...`, senza symlink) è invece corretto e coperto dal test nuovo:
  il primo guard rifiuta prima di `mkdir`.
- **Fix suggerito** (root, 3 righe): risolvere con `realpath` l'antenato esistente più profondo e
  passarlo a `guard-root` **prima** di `mkdir -p`; oppure rifiutare un `TMPDIR` che non esiste già.

### P2 — L'audit dichiara "risolta" una dipendenza `@rpath` che il loader non riesce ad aprire

- **Trigger**: un binario ha `@rpath/libX.dylib` non raggiungibile da nessun suo `LC_RPATH`, e nel
  payload esiste **un solo** file con basename `libX.dylib` altrove.
- **Conseguenza**: `audit_runtime` considera la dipendenza risolta tramite `by_name` (`:607-613`), quindi
  nessun `unresolved_rpath_dependency` — **anche con `--strict-closure`**. Il gate può essere verde su un
  payload che a runtime non linka. È la stessa classe di difetto che il report imputa al vecchio parser
  (misurare il tool invece del payload), in forma residua.
- **File:line**: `sbobino_desktop/scripts/lib/pyannote_portable_runtime.py:606-613`.
- **Prova eseguibile** (fixture Mach-O reale, `work/counterexamples.py`):
  ```
  audit violations: [solo l'ID assoluto della fixture stessa]
  audit warnings  : []
  strict violations: [solo l'ID assoluto della fixture stessa]     # nessun closure finding
  loader exit: -6
  dyld: Library not loaded: @rpath/liborphan.dylib
        Reason: tried: '…/bin/../empty/liborphan.dylib' (no such file)
  ```
- **Misura sul payload reale**: 172 dipendenze `@rpath` per-slice non risolvibili via `LC_RPATH`; **88**
  vengono dichiarate risolte solo dal fallback per basename; le altre 84 restano warning (le 12 varianti
  torchcodec inattive). Oggi le 88 sono proprio le varianti inattive che si appoggiano a `libc10`/
  `libtorch` già caricati, quindi il fallback "indovina bene" per caso; non distingue però una variante
  attiva rotta da una dormiente.
- **Impatto pratico**: il gate di release **non usa** `--strict-closure`, quindi non blocca nulla oggi;
  il problema è che l'opzione documentata come "strict closure" non è una chiusura reale. Fix: trattare
  il fallback per basename come `ambiguous`/`unresolved` dichiarato (o risolverlo solo se il path è
  raggiungibile via rpath dell'eseguibile principale già caricato), e non contarlo come risolto.

### P2 — `_resolve_rpath_entries` ignora il valore esatto `@loader_path` / `@executable_path`

- **Trigger**: `LC_RPATH` esattamente `@loader_path` (senza slash finale) — 8 binari sul payload R2,
  incluso `torch/lib/libtorch_cpu.dylib` — oppure `@executable_path/…` su una dylib.
- **Conseguenza**: `entry.startswith("@loader_path/")` è falso per `@loader_path`, quindi l'rpath viene
  scartato; la risoluzione fallisce e interviene il fallback per basename. Sul payload reale questo
  produce **l'unico warning non-torchcodec** (`libtorch_cpu -> @rpath/libomp.dylib`), che il report
  attribuisce a "due copie nel payload" ma è in realtà un falso positivo da rpath non parsato. Con
  `--strict-closure` diventerebbe una violazione su un payload che il loader carica correttamente
  (provato con `DYLD_PRINT_LIBRARIES`). Inoltre `@executable_path/…` è ancorato a `binary.parent`, che è
  sbagliato per una dylib (è la directory dell'eseguibile principale); sul payload attuale non è
  triggerato (i 4 `@executable_path` stanno solo su `bin/python*` e sul launcher di `Python.app`).
- **File:line**: `pyannote_portable_runtime.py:439-448` (e `:434-436` per `@loader_path/`).
- **Prova eseguibile**:
  ```
  >>> _resolve_rpath_entries(Path('/payload/lib/x.dylib'), ['@loader_path'])
  []
  >>> _resolve_rpath_entries(Path('/payload/lib/x.dylib'), ['@loader_path/'])
  [PosixPath('/payload/lib')]
  ```
  Conteggio rpath reali: 8 × `@loader_path`, 5 × `@loader_path/`, 4 × `@executable_path/…`.

### P2 — Il rewrite degli ID collassa dylib distinti sullo stesso install name: aliasing dyld dipendente dall'ordine di import

- **Trigger**: più dylib del payload con lo stesso basename e ID non portabile. Misurato post-fix:
  `@rpath/libomp.dylib` ×2, `@rpath/libc++.1.0.dylib` ×2, `@rpath/libpython3.11.dylib` 2→3 (già ×2 prima).
- **Conseguenza**: dyld, quando una load command chiede `@rpath/libomp.dylib`, riusa un'immagine già
  caricata il cui *install name* coincide, invece di caricare il file risolto dall'rpath. L'immagine
  effettivamente usata dipende dall'ordine di import. Misurato sul payload spostato, `env -i`:
  ```
  import torch, poi sklearn : caricate ENTRAMBE le libomp; openmp nthreads = 8
  import sklearn, poi torch : caricata SOLO sklearn/.dylibs/libomp.dylib; openmp nthreads = 4
  ```
  Nessun crash in nessuno dei due ordini (entrambe le copie dichiarano compat 5.0.0), quindi il rischio è
  latente, non un guasto osservato. Ma la nota di release "only load-command paths and code signatures
  change, so no end-user behavior is affected" non è rigorosa: l'insieme delle immagini caricate cambia.
- **File:line**: `pyannote_portable_runtime.py:860` (`"@rpath/{}".format(binary.name)`).
- **Fix suggerito**: ID univoci quando il basename è duplicato nel payload (es. `@rpath/<relpath>` o
  suffisso stabile), e una regola di audit che segnali ID duplicati fra file distinti.

---

## 3. Verifiche che passano (punti 1–5 del mandato)

1. **Load command veri, fat/thin, per-slice, path con spazi.** Parser byte-level proprio su 449 file:
   ID e dipendenze per ogni slice, nessun artefatto di header. `_message.abi3.so` ha davvero 2 ID
   per-arch (`bazel-out/osx-x86_64…`, `bazel-out/osx-aarch_64…`) e `otool -L` non riporta dipendenze
   assolute; il vecchio parser produceva su quel file una dipendenza fantasma pari al proprio path.
   Path con spazi: il payload è stato spostato in `…/relocated payload with spaces/payload` e tutti i
   comandi/loader funzionano (sotto).
2. **Risoluzione e relocatability.** Sanitize indipendente di una copia fresca R2 (`cp -cRp`): 27 file /
   28 ID, 0 dipendenze, 0 rpath; audit 0 violazioni; `codesign --verify --strict` su `Python.app` OK
   (`adhoc`, `org.python.python`); seconda esecuzione: 0 modifiche e **0 file diversi** su 29205
   (sha256 calcolati da me sui file reali e confrontati con l'inventory registrato; non ho riusato il
   loro hashing per la verifica di identità); `tree_sha256 8be55039…` riprodotto con la stessa
   definizione di hash. Il tree sanitizzato è **byte-identico** all'evidenza dell'implementer, quindi
   l'evidenza corrisponde davvero al codice spedito.
   Caricamento reale da un **quarto** path (copia del tree spostato, non il loro): `env -i` +
   `DYLD_PRINT_LIBRARIES=1`, import di `charset_normalizer.md`, `google._upb._message`, `PIL.Image`,
   `scipy.linalg`, `sklearn.utils._isfinite`, `numpy.linalg`, `torch` 2.12.0, `torchcodec` 0.13.0,
   `pyannote.audio` → 1349 load, 800 unici, 251 non di sistema: **0 fuori dal payload**, 0 riferimenti
   al root R2, a `/opt/homebrew`, `/usr/local`, `/Library/Frameworks` o `/DLC`. Nessun `DYLD_*`/env del
   mio shell sopravvive a `env -i`, quindi la prova non è mascherata.
3. **Piano read-only prima di ogni scrittura; guard e cleanup.** Fixture con un ID riparabile e una
   dipendenza mancante: `sanitize_runtime` esce con `SystemExit` e **0 file/dir cambiati** (sha256 +
   `mtime_ns` + size prima/dopo), l'ID resta assoluto. `PERSONAL_PATH_MARKERS` e
   `refuse_personal_root` sono byte-identici a `HEAD` (nessun allentamento): `/Users/runner/work/_temp`
   e `~/.cache/...` accettati; `/private/...`, `~/Library/...`, `/Library/Application Support/...`
   rifiutati. `cleanup_stage_dir` rifiuta path vuoto/relativo/`/`/fuori da `$STAGE_PARENT/tmp.*`;
   `--stage-dir-probe` crea e rimuove davvero (test verdi). Resta il P2 sull'ordine `mkdir`/guard.
4. **Prova di relocation non contaminata.** Vedi punto 2: il loader è stato osservato sui path
   effettivamente mappati, in un path con spazi, con ambiente svuotato. Il payload originale R2 esiste
   ancora su disco, quindi un load command residuo verso il vecchio root *sarebbe* stato visibile: non
   ce n'è nessuno. L'evidenza dell'implementer `diarize-tts-result.json` (run sul tree spostato, audio
   sintetico offline) riporta exit 0 e 2 speaker con confini coerenti; **non l'ho rieseguita** (avrebbe
   richiesto la pipeline completa) — la mia prova di caricamento copre tutti gli import e i dylib.
5. **Versione, updater, note, CI.** `check_release_versions.sh 2.0.38` OK su package.json,
   tauri.conf.json, 3 crate e Cargo.lock; diff `Cargo.lock`/`package-lock.json` solo versioni (4 e 2
   occorrenze), nessuna dipendenza toccata; `plugins.updater.pubkey` **identico a HEAD** (endpoint e
   `windows.installMode` invariati); `check_release_notes.sh 2.0.38` passed; note oneste su Intel/Windows
   non provati, firma ad-hoc, Companion LAN, varianti torchcodec. Nuovo step CI
   `Verify native Pyannote staging contract` dentro `native-macos`, che gira su **entrambi** i runner
   (`macos-15` arm64 e `macos-15-intel` x86_64): confermato in `ci.yml:149-150` (job `native-macos`,
   matrice `:110-121`). La suite completa (50 test) passa sia con i miei override di scratch sia nel
   percorso che userà la CI: senza `SBOBINO_*_TEST_ROOT`, con `TMPDIR` di sistema (rifiutato) e quindi
   fallback a `$HOME/.cache/sbobino-portable-tests` → 50 OK, 9.2 s.

---

## 4. Limiti di questa review

- Tutte le misure sul payload sono **macOS arm64**. Intel (conda + torchcodec compilato) e Windows non
  sono stati eseguiti: restano non provati, come dichiara il report.
- Non ho eseguito build Cargo complete, frontend, ASR/Companion, né le 12 varianti torchcodec inattive:
  i loro 84 warning sono verificati come conteggio e come causa (rpath non risolvibile), non caricandole.
- Il comportamento di `--strict-closure` è dedotto dal codice e da fixture, non da un run di release
  (il flag non è wired).
- L'aliasing degli ID è misurato ma non ha prodotto crash: rischio latente, non guasto.
- Il P1 è di wiring di release (root), non del delta di prodotto; non l'ho corretto per mandato.
- `git status` mostra `sbobino_desktop/scripts/{,lib/}__pycache__/` non tracciati e non ignorati: un
  `git add -A` li committerebbe. Igiene pre-commit per root, non un finding di prodotto.

---

## 5. Evidenze (scratch read-only)

Base: `/Volumes/UltraDisk/sbobino-companion-product-20261115/packaging-2038-independent-review/work/`

| File | sha256 |
| --- | --- |
| `macho_census.py` (parser byte-level) | `94057a8e2a3215f8825661716c244f654cd890eabede11d89e97dcb7f3372284` |
| `census-original-r2.json` | `6e48affe707031d0b236f3953f4703b0d468c62b026ee15c09f7eb04bc40ba71` |
| `census-relocated.json` | `33316903dbc183929d49a90082fc2e6b6e428116abeda9b04708135e1865862c` |
| `census-sanitized-independent.json` | `52ac0ac9934a25fc0b055e4dc12d3ea227f98b907df041402aac8844ba11740d` |
| `audit-original-r2-newcode.json` (28 violazioni) | `ec7d45fcc2a0fe031356815fffce61520f2e7c02e543bd5f24f3836306e0f4e8` |
| `audit-relocated-newcode.json` (0 violazioni) | `9c6105354a79995db251a77d57747e1250f38b186e620bfb75281676ca901319` |
| `audit-sanitized-independent.json` (0 violazioni) | `749dc6a7883e0d689c5ca898e54df5284c60e0e46d9dcbf123d445ef9c0b9b79` |
| `sanitize-receipt-independent.json` (27/28/0/0) | `8cb006577bce2a1aed35746bb85ad03b00376cba9958837d2f9c814c944d7289` |
| `sanitize-receipt-second-independent.json` (0/0) | `3fded634ece124ff9a548e639426033b8cbe064870c9202c37386df89bb84213` |
| `dylib-run.stderr` (DYLD_PRINT_LIBRARIES, `env -i`) | `4d859bac9735dc87a09fea7516e8282fbd2501f53dd0619744e763064c3d33cc` |
| `counterexamples.py` (closure/rpath) | `066c0f56eb2e87c299b50cd34a81a93d60e6481fc6163541bb2cde04044949e7` |
| `nowrite.py` (piano prima delle scritture) | `c226eb1a9801f76b2b3ce68fefd69551a7dc5b6b88676923394c6fb7363b0805` |

Riproduzione minima:
```
python3 macho_census.py <payload> > census.json
python3 lib/pyannote_portable_runtime.py audit <payload> --json audit.json
python3 lib/pyannote_portable_runtime.py sanitize <copia-payload> --json receipt.json
PYTHONDONTWRITEBYTECODE=1 python3 macho_census.py <copia-sanitizzata> > census.json
# loader: env -i PATH=/usr/bin:/bin PYTHONHOME=<root> PYTHONPATH=… DYLD_PRINT_LIBRARIES=1 <root>/bin/python3 -c 'import torch, torchcodec, pyannote.audio'
```

## 6. Azioni per root prima del dispatch

1. **P1**: aggiornare i lock a `v2.0.38` in `release.yml:9/17/25`, `promote-release.yml:9/31` e
   `verification-packages.yml:95` insieme a `REVIEWED_PRODUCT_SHA`; aggiungere il controllo al contract
   test. Il report §9 va corretto ("nessun altro pin va toccato" è falso).
2. P2 opzionali, non bloccanti per arm64: guard prima di `mkdir` (`setup_bundled_pyannote.sh:108/136`),
   fallback basename (`:606-613`), `@loader_path` esatto (`:439-448`), ID univoci per basename duplicati
   (`:860`).
3. Igiene: non usare `git add -A` (ci sono `__pycache__` non tracciati); committare solo l'allowlist
   dichiarata dal report.

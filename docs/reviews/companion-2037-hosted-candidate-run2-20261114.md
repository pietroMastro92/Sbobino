# Companion 2.0.37 — candidato hosted run 2: esito run 43632133897

Data: 2026-11-14 · Slice: verifica release (nessun fix, nessuna promozione, nessuna PR)
Repo: `/Users/pietromastro/Documents/sbobino_tauri` @ `2aebad88f52a77cf510cb335602eecf57229609f` (= `main`)
Tag: `v2.0.37` annotato `f8d50ffb18bfa104861e023c75d1303c770d5033` → commit `5198720f0b2d7dde93a44472435a2d630b5376eb` (invariato)
Run: <https://github.com/pietroMastro92/Sbobino/actions/runs/43632133897> — `workflow_dispatch` "Release Candidate", 19:26:52Z → 20:12:18Z

**Esito: FAILURE. Nessun candidato prerelease prodotto, 0/7 proof, gate di promozione non raggiunto.**
Il fix workflow-only del run 1 è stato eseguito davvero (è nel log dello step) ma **è un no-op su macOS**.

## Due provenienze (distinte, verificate)

| Cosa | SHA |
| --- | --- |
| Workflow ref effettivo (`.github/workflows/release.yml` eseguito) | `2aebad88f52a77cf510cb335602eecf57229609f` |
| Input dispatch `tag_name` | `v2.0.37` (immutato) |
| Tag annotato | `f8d50ffb18bfa104861e023c75d1303c770d5033` → `5198720f0b2d7dde93a44472435a2d630b5376eb` |
| `REVIEWED_PRODUCT_SHA` nel workflow | `f9055b289ede447a265e1cb51a177b304dbcf3bd` |

- `git diff f9055b2..HEAD -- core apps crates scripts Cargo.toml Cargo.lock` → **vuoto** (nessuna modifica al prodotto).
- File cambiati `f9055b2..HEAD`: `release.yml`, `promote-release.yml`, `verification-packages.yml`, `docs/REPOSITORY_STATE.md`, due review. Nient'altro.
- Tutti i checkout nel workflow usano `ref: ${{ inputs.tag_name }}`; nessun `github.sha`. Le proof si costruiscono con
  `git rev-parse HEAD` dentro il checkout del tag (righe 502, 593, 616, 710), quindi avrebbero riportato `5198720…`,
  non `2aebad8`. Non sono state prodotte, quindi la distinzione resta dichiarata e non misurata.
- Runner ufficiali hosted: `ubuntu-latest`, `macos-15`, `macos-15-intel`, `windows-2025`. Occorrenze `AS-THIRD` in
  codice/config: **0**.
- Guard invariato: `PERSONAL_PATH_MARKERS` contiene ancora `/private/`
  (`sbobino_desktop/scripts/lib/pyannote_portable_runtime.py:74-80`); `/Users/` e `/home/` restano solo in
  `LOAD_COMMAND_PATH_MARKERS` (`:86`).

## Matrice reale per job (nessun aggregato)

| Job | Esito | Note |
| --- | --- | --- |
| Validate v2.0.37 candidate tag | success | tag bloccato a v2.0.37; diff prodotto vs `f9055b2` vuoto |
| Build native Windows x86_64 | success | artefatti `sbobino-windows-x86_64` 626.857.627 B + harness 111.058 B |
| Build native macOS aarch64 (`macos-15`) | **failure** | step `Hydrate native Pyannote assets`, job 132560913987, 19:29:12Z |
| Build native macOS x86_64 (`macos-15-intel`) | **failure** | step `Hydrate native Pyannote assets`, job 132560913948, 19:37:16Z |
| publish-candidate | skipped | richiede entrambi gli artifact macOS |
| release-quality-contract-tests / distribution-readiness / windows-distribution-readiness / portability-smoke / intel-pyannote-parakeet-smoke / synthetic ASR | skipped | 0/7 proof |
| upload-validation-assets | failure (cascata) | `Unable to download artifact(s): Artifact not found for name: distribution-readiness-aarch64` |

Artefatti del run: solo i due Windows. Nessun artefatto macOS, nessuna proof, nessun manifest, nessuna firma.

## Blocker (file:line)

- **B1 — il fix è un no-op**: `release.yml:95` esporta `TMPDIR="$RUNNER_TEMP"` (presente ed eseguito: log aarch64
  riga 550), ma `setup_bundled_pyannote.sh:573` usa `STAGE_DIR=$(mktemp -d)`. Su macOS BSD `mktemp` la forma
  senza template **ignora `TMPDIR`** e usa `_CS_DARWIN_USER_TEMP_DIR`; `man mktemp`: *"If no arguments are passed or
  if only the -d flag is passed mktemp behaves as if -t tmp was supplied"*. Lo staging finisce quindi in
  `/var/folders/…/T/tmp.XXXX` (log aarch64 righe 560, 873) → `sanitize_runtime` risolve il path in
  `/private/var/folders/…` → `refuse_personal_root` (riga 678) rifiuta con `matched '/private/'`.
  Riproduzione locale dei quattro idiomi `mktemp`: `evidence/mktemp-tmpdir-repro.log`.
- **B2 — secondo blocco, indipendente**: anche facendo onorare `TMPDIR`, stadiare sotto `RUNNER_TEMP`
  (`/Users/runner/work/_temp`) non passerebbe l'audit. Misurato sul payload aarch64 costruito in locale
  (449 Mach-O, post-sanitize): 46 load command assoluti non di sistema, **21 puntano alla radice di staging**
  (10 `-id`, 11 dipendenze; estensioni mypyc/charset_normalizer/fontTools). Oggi non sono violazioni solo perché
  quella radice inizia con `/Volumes`; sotto `/Users/runner/…` diventano `personal_path_reference`, perché
  `LOAD_COMMAND_PATH_MARKERS` include `/Users/` e `sanitize_runtime` ripara solo `HOST_PREFIXES`.
  Script read-only: `evidence/measure_stage_absolute_refs.py`.
- **B3 — publish-candidate** skipped → nessuna release `v2.0.37`, nessun asset.
- **B4 — validazioni** tutte skipped → **0/7 proof** pubbliche; 4 requisiti manifest + sig updater non prodotti.
- **B5 — upload-validation-assets** failure da cascata (artifact `distribution-readiness-aarch64` inesistente),
  non un difetto indipendente.

Log: `logs/run-43632133897-failed.log:1044` (aarch64), `:712` (x86_64), `:713`/`:1045` (exit code), `:1060` (cascata);
log integrale del job aarch64 in `logs/job-132560913987-aarch64.log`.

## Conclusione sulla diagnosi

Il rimedio scelto al run 1 era motivato con la precedenza di `TMPDIR` in **Python** (`tempfile`), che è vera, ma il
punto che fallisce è **shell `mktemp`**, non Python: il fix non ha mai potuto agire. In più, il secondo blocco mostra
che **sui runner hosted macOS non esiste una radice di staging accettata da entrambi i controlli**:
`/private/var/folders` è vietato da `refuse_personal_root`, `/Users/runner/work/_temp` è accettato dal guard ma
vietato dall'audit sui load command assoluti del payload. Il "solo workflow" è quindi esaurito.

Dettaglio, misure e riproduzione: `evidence/findings-guard-staging-run2.md`, `evidence/pre-run-checks.md`.

## Rimedi possibili (per root, nessuno applicato)

1. **Staging che onora `TMPDIR`** — `setup_bundled_pyannote.sh:573` (e `:118`):
   `mktemp -d "${TMPDIR:-/tmp}/tmp.XXXXXXXXXX"`. Necessario ma **non sufficiente** (attiva B2).
2. **Payload indipendente dalla radice di build** — in `sanitize_runtime`, oltre a `HOST_PREFIXES`, riscrivere a
   `@rpath/<name>` / `@loader_path/<rel>` gli `-id` e le dipendenze assolute che cadono dentro l'albero.
   Con 1+2 il guard resta intatto, lo staging può stare in `RUNNER_TEMP` e l'audit torna verde.
3. **Riparazione mirata dei soli self-reference** — alternativa più stretta a 2: riscrivere i binari con
   `read_install_name(binary) == str(binary)`. Copre i 21 ref misurati, non quelli futuri di forma diversa.
4. **Guard che accetta il temp di sistema** — in `refuse_personal_root`, ignorare `/private/` quando il path
   risolto è dentro `tempfile.gettempdir()`. Da abbinare comunque a 2 (da solo incontra B2).
5. **Contratto di test** — caso che verifica (a) staging sotto `$TMPDIR` quando è impostato, (b) `sanitize` che
   elimina ogni load command assoluto interno alla radice, (c) rifiuti invariati su `Library/Application Support`
   e `pairing.enc`. Impedisce il "verde in locale, rosso su hosted".

Tutti i rimedi risolutivi toccano `sbobino_desktop/scripts/…`, cioè l'albero di prodotto congelato: serve una
decisione di root su **riapertura del freeze + nuovo commit/tag** (il tag `v2.0.37` non va mosso), oppure
accettare che la 2.0.37 non sia pubblicabile da runner hosted con il guard revisionato.
Bozza non applicata: `evidence/proposed-fix-stage-and-sanitize.patch.md`.

## Stato release e asset (verificato via API dopo il run)

- `gh release view v2.0.37` → **release not found** (nessuna bozza, nessun prerelease).
- `gh api git/ref/tags/v2.0.37` → `f8d50ff…`, tipo `tag` (annotato, non ritaggato).
- `gh release list` / `/releases/latest` → `v2.0.36` ancora `Latest`, `isPrerelease=false`, `publishedAt 2026-10-12T12:02:28Z` (intatta).
- Nessun `latest.json`, `setup-manifest.json`, `runtime-manifest.json`, `pyannote-manifest.json`,
  `release-readiness-proof.json`, `release-notes.md` pubblicato: feed updater stabile non toccato.
- Firme `*.app.tar.gz.sig` / `.nsis.zip.sig`: non prodotte, non verificabili in questa slice.

## Verifiche eseguite in locale (gate, indipendenti dal run)

- `python3 -m unittest scripts.test_release_workflow_contract.py` → 29 test, OK (1 skip: native pwsh su macOS).
- `scripts.test_pyannote_portable_runtime` (17 test) per radice temporanea:
  - `TMPDIR` di default macOS (`/var/folders` → `/private/var/folders`): FAILED (failures=1, errors=5) — comportamento
    atteso e invariato: è il difetto originale, non una regressione;
  - `TMPDIR` su volume esterno (nessun marker): **OK** (riproduce il 17/17 dichiarato da root);
  - `TMPDIR` tipo `RUNNER_TEMP` (`/Users/...`): FAILED (4) perché le fixture creano load command assoluti sotto la
    radice temporanea — stessa classe di B2.
- Verifica pre-dispatch (provenienza, guard, runners, gate promozione, stato release): `evidence/pre-run-checks.md`.

## Verifiche non eseguite (dichiarato)

Nessun avvio app/server/telefono/SIM, nessun build CPU locale del prodotto, nessuna modifica a segreti, settings,
tag, release, CI o file tracciati; nessun download dei 626 MB di artefatti (run fallito, artifact run-scoped).
Payload Intel, audit pyannote Intel, preflight Intel atteso in rifiuto, portability smoke, ASR live, codesign/notarize,
manifest e firme updater: **non prodotti** dalla pipeline (job skipped), nessuna evidenza.
Il job Windows è verde come build, ma **non certifica** alcun payload nuovo: `publish-candidate` è skipped, quindi
nessun asset Windows è pubblico né firmato per la release.

## Prossimo passo atteso da root

1. Scegliere fra rimedio 1+2 (strutturale, raccomandato) e 1+3 (minimo).
2. Riaprire il freeze, ri-revisionare il nuovo commit, rifare il pin e un nuovo tag/commit candidato; **non** muovere `v2.0.37`.
3. Solo dopo: nuovo dispatch `Release Candidate` e nuova verifica completa (3 build + 4 validazioni + 7 proof + 4 manifest + firme).
4. Nessuna promozione, nessun asset manuale, nessun rerun identico (fallirebbe con certezza: B1 è deterministico).

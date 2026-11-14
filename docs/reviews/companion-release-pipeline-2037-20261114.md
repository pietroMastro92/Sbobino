# Pipeline di release 2.0.37 — aggiornamento pin (14 novembre 2026)

Stato: **diff di workflow pronto per review, non un PASS di release**. Nessun
commit, stage, tag, push, dispatch, prerelease o promozione. Nessuna build,
nessuna VM, nessuna credenziale letta, nessun accesso di rete. R6 non è stato
toccato (né processo né porta né profilo).

Oggetto: spostare i lock della pipeline dal freeze 2.0.36 al prodotto
revisionato 2.0.37, mantenendo il tag **esatto** e il `git diff` contro lo SHA
revisionato. Review target: `f9055b289ede447a265e1cb51a177b304dbcf3bd`
("prepare Desktop 2.0.37 with persistent iPhone Companion", 36 file).

## 1. Esito in una riga

I tre workflow di release sono ora legati a `v2.0.37` e a
`REVIEWED_PRODUCT_SHA=f9055b2` senza alcun allentamento del gate: il lock resta
esatto, il controllo `git diff` resta fail-closed sulle stesse cinque path di
prodotto, e una simulazione isolata su cloni temporanei prova che il gate
accetta un commit che tocca solo docs/workflow e rifiuta sia un tag diverso sia
una deriva di prodotto. Restano fuori, come atteso: tag, candidata, promozione,
build public, zip Pyannote a 0 violazioni e run hosted.

## 2. Ownership e perimetro

Modificati (4 file tracciati):

| File | Modifica |
| --- | --- |
| `.github/workflows/release.yml` | default input, nome job, lock tag, `REVIEWED_PRODUCT_SHA` |
| `.github/workflows/promote-release.yml` | default input, lock tag |
| `.github/workflows/verification-packages.yml` | `check_release_versions.sh 2.0.36` → `2.0.37` |
| `docs/REPOSITORY_STATE.md` | descrizione attuale corretta, storia conservata |

Nuovo: questo report. Evidenze e log fuori dal repo, in
`/Volumes/UltraDisk/sbobino-companion-product-20261114/official-release-pipeline/`.

**Non** toccati: `apps/`, `crates/`, `sbobino_desktop/scripts/` e ogni altro file
di prodotto (il diff contro `f9055b2` sulle cinque path del gate è **vuoto**),
`ci.yml`, i workflow storici `arm-runtime-validation.yml`,
`intel-runtime-validation.yml`, `windows-port.yml`, il dispatcher, gli script di
quality gate, app iOS, `promo-video/`, `output/`, `.agents/`, `.codex/`,
`.playwright-cli/`, `AGENTS.md`, i config/untracked altrui e i report storici.

## 3. Modifiche esatte

`.github/workflows/release.yml`:

| Riga | Prima | Dopo |
| --- | --- | --- |
| 9 | `default: v2.0.36` | `default: v2.0.37` |
| 17 | `name: Validate v2.0.36 candidate tag` | `name: Validate v2.0.37 candidate tag` |
| 21 | `REVIEWED_PRODUCT_SHA: 70a7e0abdf69f8261209ff9513cec0224530d53e` | `REVIEWED_PRODUCT_SHA: f9055b289ede447a265e1cb51a177b304dbcf3bd` |
| 25-26 | lock `v2.0.36` + messaggio | lock `v2.0.37` + messaggio |

Invariati nel merito: `if [[ "$TAG_NAME" != ... ]]` con `exit 1` prima del
checkout, `ref: ${{ inputs.tag_name }}`, `fetch-depth: 0`, e

```bash
git diff --exit-code "$REVIEWED_PRODUCT_SHA" HEAD -- \
  sbobino_desktop/apps sbobino_desktop/crates sbobino_desktop/scripts \
  sbobino_desktop/Cargo.toml sbobino_desktop/Cargo.lock
```

`.github/workflows/promote-release.yml`: `default: v2.0.37` (riga 9) e lock
esatto a `v2.0.37` con messaggio aggiornato (righe 31-32). Restano
`check_release_versions.sh "$VERSION"` con `SBOBINO_RELEASE_PROFILE=standalone-dev`,
`check_release_notes.sh` e `promote_candidate_release.sh`.

`.github/workflows/verification-packages.yml:95`: `check_release_versions.sh
2.0.37`. Questo era un guasto reale annunciato: il workflow verifica la
coerenza interna dell'albero corrente (ora 2.0.37) e si attiva anche su push
quando il file stesso cambia, quindi col pin 2.0.36 sarebbe fallito al primo
push di questo diff.

Vale la stessa proprietà per la candidata: da quando `release.yml` viene
eseguito su un albero 2.0.37, `check_release_versions.sh "$VERSION"` con
`VERSION=v2.0.37` è coerente senza altri edit.

## 4. Riferimenti di versione lasciati di proposito

| Punto | Perché resta |
| --- | --- |
| `.github/workflows/ci.yml:162` — smoke Intel con `2.0.36` | è l'argomento che sceglie la **release da cui scaricare gli asset runtime pubblicati** (`release_intel_pyannote_parakeet_smoke.sh`: `TAG="v$VERSION"` → `gh release download "$TAG"`). Non esiste alcun asset 2.0.37; spostarlo a 2.0.37 romperebbe la CI immediatamente. Va deciso solo dopo la pubblicazione degli asset 2.0.37, non con questo diff. |
| `verification-packages.yml:51,88,102,119` — `RUNTIME_SOURCE=34906f9…`, `v2.0.34` | provenienza immutabile degli asset runtime verificati; non è la versione del prodotto. |
| `arm-runtime-validation.yml`, `intel-runtime-validation.yml`, `windows-port.yml` — `v2.0.34` | validazioni storiche a dispatch manuale, fuori dal contratto della candidata. |
| `apps/desktop/src/test/settingsRuntimeFreshness.test.tsx` (`2.0.36` in fixture) | stringhe di fixture dentro il prodotto: cambiarle violerebbe il freeze byte-identico a `f9055b2`. |
| `docs/companion-ai-v1.md`, `README.md`, `docs/reviews/*` datati | affermazioni storiche su ciò che è pubblicato; restano vere finché 2.0.37 non esiste. |

Nessun pin è stato reso configurabile o rimosso: la semantica "reviewed" resta
un confronto di albero con uno SHA letterale, non un input del dispatch.

## 5. Test reali eseguiti

Ambiente: `TMPDIR` su `/Volumes/UltraDisk` (mount verificato, 106 GiB liberi),
nessuna rete, nessuna dipendenza installata, nessuna scrittura nel working tree
del repo oltre ai 4 file sopra.

| Gate | Comando | Esito | Log |
| --- | --- | --- | --- |
| Contratto workflow (baseline, pre-edit) | `python3 scripts/test_release_workflow_contract.py` | PASS 29 test, 1 skip (`pwsh` assente) | `logs/baseline-test_release_workflow_contract.log` |
| Quality gate release (baseline) | `python3 scripts/test_release_quality_gates.py` | PASS 5 test | `logs/baseline-test_release_quality_gates.log` |
| Contratto workflow (post-edit) | idem | PASS 29 test, 1 skip | `logs/post-test_release_workflow_contract.log` |
| Quality gate release (post-edit) | idem | PASS 5 test | `logs/post-test_release_quality_gates.log` |
| Coerenza versioni | `./scripts/check_release_versions.sh 2.0.37` | PASS (exit 0) | `logs/version-and-notes-gates.log` |
| Note di rilascio | `./scripts/check_release_notes.sh 2.0.37 docs/release-notes/v2.0.37.md` | PASS (exit 0) | idem |
| Coerenza interna (senza argomento, come in `ci.yml`) | `./scripts/check_release_versions.sh` | PASS (exit 0) | idem |
| YAML parse + `bash -n` dei blocchi `run` | `evidence/verify-workflows.py` | PASS: 58 blocchi bash validi, 23 blocchi `pwsh` saltati per shell | `logs/verify-workflows.log` |
| Guardia tag/SHA candidata (simulata) | `evidence/guard-simulation.sh` | PASS 6/6 casi | `logs/guard-simulation.log` |
| Guardia lock promozione (simulata) | `evidence/promote-guard-simulation.sh` | PASS 3/3 casi | `logs/promote-guard-simulation.log` |
| Whitespace | `git diff --check` | PASS | `evidence/after-status.txt` |
| Prodotto invariato vs review | `git diff --stat f9055b2… -- sbobino_desktop/{apps,crates,scripts,Cargo.toml,Cargo.lock}` | **vuoto** | comando in §6 |

## 6. Prova isolata della guardia tag/SHA

Lo script `evidence/guard-simulation.sh` **estrae i due blocchi `run:` dal
workflow reale** (via YAML, con assert su nome/ordine degli step), clona il repo
con `git clone --shared` in una directory temporanea su UltraDisk e li esegue
lì. Il repo sorgente non viene mai toccato: HEAD resta `f9055b2`, nessun tag
creato, nessun commit.

| Caso | Setup | Atteso | Ottenuto |
| --- | --- | --- | --- |
| A | tag `v2.0.37` sul commit revisionato | lock 0, diff 0 | lock 0, diff 0 |
| B | commit in più con **solo** docs + workflow, tag `v2.0.37` | lock 0, diff 0 | lock 0, diff 0 |
| C | commit in più che modifica `sbobino_desktop/scripts/check_release_versions.sh` | lock 0, diff 1 | lock 0, diff 1 (fail-closed) |
| D | tag `v2.0.36` | lock 1 | lock 1 con il messaggio "locked to v2.0.37" |
| E | `REVIEWED_PRODUCT_SHA` = `aa171fa` (albero 2.0.36) contro HEAD revisionato | lock 0, diff 1 | lock 0, diff 1 (80 file, +18410/−566) |
| F | tag pulito + SHA revisionato | diff 0 | diff 0 |

Il caso B è esattamente la forma che avrà il commit di root (pin + docs sopra
`f9055b2`): il gate resta verde perché nessuna path di prodotto cambia. Il caso
E prova che il pin distingue davvero la release precedente da quella nuova.
`evidence/promote-guard-simulation.sh` estrae il blocco `if [[ ... ]]` da
`promote-release.yml` e verifica `v2.0.37`→0, `v2.0.36`→1, `v2.0.38`→1.

Limite dichiarato: il confronto è di **albero**, non di ancestry. Un tag
puntato a un commit non discendente ma con le stesse cinque path passerebbe.
Non ho aggiunto `git merge-base --is-ancestor` per non cambiare la semantica
del gate in un diff di soli pin: se root vuole la garanzia di discendenza, è una
riga da aggiungere **prima** del `git diff`, con la sua review.

## 7. `docs/REPOSITORY_STATE.md`

Testo storico conservato (consolidation, recovery su UltraDisk, limiti di
prova), descrizione attuale corretta: data a 2026-11-14; la 2.0.37 è **committed**
in `f9055b2` (non più "uncommitted preparation"); `main` locale è due commit
avanti a `origin/main` (`692cdb5`) e non pushato; nessun tag `v2.0.37`, nessuna
candidata, nessuna promozione; `v2.0.36` resta latest e l'unica versione servita
dall'updater; i pin ora puntano al commit revisionato; motivo esplicito per cui
`ci.yml` resta a 2.0.36 e i workflow storici a 2.0.34; la sezione Companion non
parla più di "draft non committato" ma di implementazione committata e non
rilasciata, mantenendo le prove datate 2026-10-27 come storia.

## 8. Cosa deve fare root (non eseguito qui)

1. Review di `evidence/pipeline-pin-diff.patch` (120 righe: 3 workflow + doc) e
   di questo report.
2. Commit dei 4 file + report. Suggerito un commit unico separato, es.
   `chore: bind 2.0.37 release pipeline to reviewed product`, sopra `f9055b2`.
   Nessun file di prodotto deve entrare in quel commit.
3. Push di `main` (2 commit già presenti + questo). Attenzione: il push che
   tocca `verification-packages.yml` **attiva** il workflow "Verification
   installers"; ora è coerente a 2.0.37, ma è un run hosted reale (macOS 15,
   macOS 15 Intel, Windows 2025) senza firma di produzione.
4. Tag `v2.0.37` sul commit che contiene i pin (caso B) — non serve ri-taggare
   `f9055b2`: il gate accetta entrambi, ma il tag sul commit dei pin registra la
   pipeline esatta usata.
5. Dispatch `Release Candidate` con `tag_name=v2.0.37` (prerelease), attesa
   delle 7 proof + portability smoke; poi `Promote Release Candidate`.
   Rollback restano `v2.0.36` e `v2.0.35`.

## 9. Non-claim e rischi residui

- **Non è un PASS di release.** Nessun artefatto 2.0.37 esiste, il tag non
  esiste, `latest.json` serve ancora 2.0.36.
- Il payload Pyannote nel repo resta RED (2 `host_rpath` noti): la build public
  deve rigenerarlo e l'audit strict deve dare 0 violazioni. Questo diff non lo
  corregge e non lo maschera.
- Build/DMG e validazione runtime sono di altri worker (UltraDisk, config Tauri
  temporanea con restore); qui non è stata eseguita alcuna build.
- Intel/Windows restano non provati con il nuovo audit: il primo run hosted può
  fallire ed è il comportamento voluto.
- `release.yml` non è stato eseguito: la prova della guardia è la simulazione
  sui suoi blocchi reali, non un dispatch.
- R6 (`63101`, PID 25290) non è stato letto, fermato o modificato; nessuna
  credenziale, nessun segreto, nessun server o UI toccati.

## 10. Evidenze

Directory:
`/Volumes/UltraDisk/sbobino-companion-product-20261114/official-release-pipeline/`

- `evidence/before-head-status.txt`, `after-head.txt`, `after-status.txt`,
  `before-file-sha256.txt`, `after-file-sha256.txt`;
- `evidence/pipeline-pin-diff.patch`;
- `evidence/guard-simulation.sh`, `evidence/promote-guard-simulation.sh`,
  `evidence/verify-workflows.py`;
- `logs/baseline-*.log`, `logs/post-*.log`, `logs/version-and-notes-gates.log`,
  `logs/verify-workflows.log`, `logs/guard-simulation.log`,
  `logs/promote-guard-simulation.log`;
- clone di simulazione in `tmp/` (usa e getta, nessun ref del repo sorgente).

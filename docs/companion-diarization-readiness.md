# Readiness diarization Companion — contratto wire (12 novembre 2026)

Stato: **contratto implementato** nella slice readiness/admission. Questo documento
descrive solo il protocollo e le sue garanzie; non è un PASS della diarizzazione e
non sostituisce la prova fisica.

## 1. Perché esiste

`capabilities` contiene `companion_diarization_v1` in modo statico: è una
**capacità di protocollo**, non una promessa che il Desktop possa eseguire la
diarizzazione in questo momento. Nella prova LAN del 12 novembre un job con
`diarization=true` è stato accettato senza alcun backend, l'ASR è stato salvato e
`diarization_status` è risultato `failed`. Questa slice aggiunge un annuncio
veritiero e un rifiuto anticipato, senza cambiare `capabilities` e senza
regredire i Desktop precedenti.

## 2. `GET /v1/health` — campo additivo

```json
{
  "version": 1,
  "desktop_version": "2.0.36",
  "engine": "whisper_cpp",
  "model": "ggml-base.bin",
  "asr_available": true,
  "capabilities": ["companion_diarization_v1"],
  "ai": { "...": "..." },
  "readiness": {
    "diarization": {
      "ready": false,
      "reason_code": "no_backend",
      "message": "La diarizzazione non è disponibile su questo Desktop. Installa un motore da Impostazioni > Modelli locali."
    }
  }
}
```

Regole:

- Il blocco è **additivo**: `version`, `engine`, `model`, `asr_available`,
  `capabilities` e `ai` mantengono forma e semantica esistenti.
- `ready` descrive **una richiesta esplicita per-job** (`diarization=true`), non il
  toggle globale delle impostazioni. Un Desktop con il toggle globale spento ma
  senza alcun backend riporta `ready: false`, mai `ready: true` con
  `reason_code: "disabled"`.
- `reason_code` appartiene a un insieme **chiuso** (tabella § 3). Il client non deve
  interpretare altri valori.
- `message` è una frase umana **statica** scelta dal codice chiuso. Non deriva mai dai
  diagnostici interni: quelli contengono path assoluti e output di probe (per esempio
  `LC_RPATH /opt/homebrew/lib` e il percorso di `libpython3.11.dylib`) e non devono
  attraversare il wire. Il campo è sempre presente e non vuoto in questa
  implementazione; un client deve comunque tollerare l'assenza e usare un proprio
  messaggio di fallback.
- Il calcolo **non** aggiunge I/O: riusa `runtime_health_preflight()` già eseguito da
  `/v1/health` e `selected_backend_health`, che è pura. Nessun import Python, nessun
  `otool`, nessuno spawn, nessuna rete, nessuna costruzione di diarizer e nessun nuovo
  polling o cache architetturale.
- Se il preflight non è calcolabile, `/v1/health` mantiene la risposta d'errore
  esistente e **omette** `readiness`: assenza = sconosciuto, non "non pronto".

## 3. Codici ammessi

| `reason_code` | Significato | Azione consigliata |
| --- | --- | --- |
| `ok` | Un backend selezionato è pronto per un job esplicito. | Procedere. |
| `no_backend` | Nessun backend disponibile per la richiesta per-job. | Installare un motore da Impostazioni > Modelli locali. |
| `fluid_missing` | Risorse FluidAudio assenti o non verificate. | Configurare il motore locale. |
| `pyannote_missing` | Runtime o modello Pyannote non installati. | Installare Pyannote. |
| `pyannote_repair_required` | Installazione Pyannote incompleta o da riparare. | Riparare da Impostazioni > Modelli locali. |
| `pyannote_receipt_required` | Verifica durevole Pyannote mancante o non valida. | Eseguire la riparazione/verifica. |
| `pyannote_arch_mismatch` | Runtime Pyannote incompatibile con l'host. | Riparare il runtime. |
| `device_unsupported` | Il device scelto non è valido per il backend selezionato (per esempio Fluid su `mps`, Pyannote su `ane`). | Scegliere `cpu` o `auto`. |

Il codice interno del selettore non viene esposto: `pyannote_not_ready`,
`fluid_helper_missing`, `fluid_checksum_invalid`, `speaker_diarization_not_ready`
e simili vengono mappati sui valori chiusi qui sopra. Stati di riparazione
sconosciuti restano `pyannote_repair_required`.

## 4. `PUT /v1/jobs/<uuid>` — admission

Con diarizzazione **effettiva** (`diarization=true` esplicito, oppure parametro
assente e toggle globale attivo) e `ready == false`, il server risponde:

```http
HTTP/1.1 409 Conflict
Content-Type: application/json
Cache-Control: no-store
```

```json
{
  "message": "La diarizzazione non è disponibile su questo Desktop. Installa un motore da Impostazioni > Modelli locali.",
  "reason_code": "diarization_unavailable",
  "readiness": {
    "diarization": {
      "ready": false,
      "reason_code": "no_backend",
      "message": "La diarizzazione non è disponibile su questo Desktop. Installa un motore da Impostazioni > Modelli locali."
    }
  }
}
```

Ordine e garanzie:

1. Auth, schema della query (`diarization` ∈ {true,false}) ed `extension` mantengono
   la precedenza: un `401`/`400` esistente non diventa `409`.
2. Il controllo avviene **prima** di leggere il corpo e **prima** di creare lo
   snapshot del job. Un rifiuto non consuma né persiste il corpo e non lascia
   job: `GET /v1/jobs/<uuid>` risponde `404`.
3. `diarization=false` resta invariato: percorso ASR-only, nessun blocco.
4. Parametro assente: si usa il toggle globale solo per decidere se il gate si
   applica; `ready` continua a modellare la semantica per-job.
5. Se il preflight non è calcolabile, il gate non inventa un verdetto: la richiesta
   prosegue e l'errore resta quello del job a valle.
6. Un job già accettato con lo stesso `<uuid>` non viene rifiutato retroattivamente:
   la `PUT` ripetuta resta idempotente e risponde con lo snapshot esistente.

## 5. Compatibilità client (iPhone)

- `readiness` è **opzionale** nel modello. Assenza del blocco = Desktop precedente =
  **sconosciuto**: il client mantiene il comportamento esistente (guardia
  `capabilities`, upload consentito, `diarization_status` come unico segnale a valle).
- `ready: true` → la richiesta esplicita procede.
- `ready: false` → errore tipizzato e azionabile **prima** di `retainAudio`,
  `persistPendingJob` e upload. Nessun downgrade silenzioso a `diarization=false`.
- `409` con `reason_code == "diarization_unavailable"` dopo un health che diceva
  `ready: true` (snapshot stale) è un rifiuto definitivo: il client può pulire solo
  la pending e la copia audio di **quel** tentativo. Qualsiasi altro `409`, stato
  incerto o errore di trasporto conserva pending e copia per la ripresa. Il file
  originale dell'utente non viene mai eliminato.

## 6. Fuori perimetro (slice distinte)

- **Fallimento tardivo dopo l'accettazione**: se il backend si guasta dopo che il
  job è stato accettato (health e admission erano `ready`), il job resta accettato e
  `diarization_status` può risultare `failed` con ASR salvato. La readiness non è una
  garanzia di successo end-to-end.
- Rigenerazione dell'asset Pyannote bundled e del packaging (causa radice separata).
- UI desktop per lo stato "non disponibile" e scelte di prodotto.
- Durability/ACK del job, retry automatici, recovery oltre al cleanup minimo.

Nota di trasporto: il gate server precede la lettura del body, ma un client HTTP eager potrebbe avere già inviato byte sulla rete. Il preflight iOS ready=false garantisce zero PUT; il solo 409 server non prova zero byte trasferiti.

## 7. Turni misurati (`speaker_turns_v1`) — additivo

Questa sezione descrive la conservazione dei turni realmente restituiti dal
diarizer. Non è una certificazione della qualità della diarizzazione e non
cambia le etichette dei segmenti ASR.

### 7.1 Artifact desktop

`TranscriptionService` cattura i `turns` dell'esito `diarize` della **stessa**
esecuzione, indipendentemente dai segmenti ASR. L'assegnazione per segmento
(maggioranza rigida >50%, invariata) resta l'unica responsabile di
`speaker_id`/`speaker_label` dentro `timeline_v2`.

Solo quando `speaker_diarization_status == "completed"` **e** almeno un turno è
strutturalmente valido, l'artifact riceve la chiave riservata:

```json
"speaker_turns_v1": "[{\"speaker_id\":\"speaker_1\",\"speaker_label\":\"Speaker 1\",\"start_seconds\":0.0,\"end_seconds\":1.4},{\"speaker_id\":\"speaker_2\",\"start_seconds\":1.2,\"end_seconds\":2.0}]"
```

Regole:

- `metadata` è già una mappa JSON: nessuna migrazione SQLite, nessun nuovo file.
- Ordine e sovrapposizioni misurati sono preservati. Nessun merge, sort,
  interpolazione o clamp ai segmenti o a proporzioni di testo.
- Un record invalido viene scartato singolarmente: `speaker_id` vuoto dopo il
  trim, `start_seconds`/`end_seconds` non finiti, `start_seconds < 0`,
  `end_seconds <= start_seconds`. `speaker_label` vuota dopo il trim viene
  omessa, non sostituita. Non si inventa né si duplica alcun turno.
- La chiave è **riservata**: un valore presente in `request.metadata` viene
  sempre rimosso prima della scrittura. Solo l'uscita del diarizer di questa
  esecuzione può popolarla. Failure, cancellazione, `diarization=false` e
  risultato vuoto lasciano la chiave assente e non "resuscitano" un valore
  iniettato dal richiedente.
- `timeline_v2` resta `version: 2` e invariata: nessun turno viene aggiunto ai
  segmenti e nessuna etichetta viene riscritta.

### 7.2 Wire Companion

`GET/PUT /v1/jobs/<uuid>` può includere, solo nello snapshot terminale
`completed` con `diarization_status == "completed"`, un campo additivo:

```json
"speaker_turns": [
  {"speaker_id":"speaker_1","speaker_label":"Speaker 1","start_seconds":0.0,"end_seconds":1.4},
  {"speaker_id":"speaker_2","start_seconds":1.2,"end_seconds":2.0}
]
```

- Unità: secondi. È un array JSON, non una stringa JSON (a differenza della
  chiave `metadata` che resta una stringa).
- `speaker_label` è opzionale e omessa quando assente.
- Campo **omesso** (mai `null`, mai array vuoto) per: job non `completed`,
  `diarization_status` diverso da `completed`, metadata assente o illeggibile,
  nessun turno valido. Un array misto scarta i singoli record invalidi; se non
  ne resta nessuno, il campo è assente.
- `timeline_v2` slim resta invariata: `words` continuano a non attraversare il
  wire e lo schema/versione non cambiano.
- Compatibilità: un client vecchio ignora il campo; un client nuovo su Desktop
  vecchio non lo trova e mantiene la ricostruzione dai segmenti. Nessuna nuova
  capability obbligatoria.
- Gli snapshot Companion precedenti (senza il campo) continuano a
  deserializzarsi: il campo ha `serde(default)` ed è `None`.

### 7.3 Client iOS

`DesktopJobSnapshot.speakerTurns` è opzionale (`nil` = nessuna misura). Il
contratto lato client:

- I turni misurati sostituiscono la ricostruzione **solo** se lo snapshot è
  `completed` e `diarization_status == "completed"` e almeno un record è valido;
  altrimenti si usa la ricostruzione legacy, byte per byte identica.
- I record invalidi vengono scartati uno per uno senza far fallire la
  trascrizione; non vengono completati intervalli mancanti.
- Le etichette dei segmenti già decodificati vincono per lo stesso
  `speaker_id`: nessun rename. Per gli ID presenti solo nei turni si usa
  `speaker_label` se presente, altrimenti il fallback `Voce N` esteso dopo gli
  ID dei segmenti.
- `TranscriptDiarization.origin` (`measured` / `segment_reconstruction`) è
  opzionale e retro-compatibile: `nil` = payload precedente, e un valore
  futuro sconosciuto viene conservato senza perdere i turni.

Le etichette dei segmenti misti (D1) restano un limite aperto: questa sezione
non le modifica.

# FluidAudio Phonon-2: identità, supporto lingue e fattibilità (2026-10-13)

## Esito

**Phonon-2 è un modello ASR distinto da EmbeddingGemma 2 e da Parakeet Redux.** È una riaddestramento con quantizzazione consapevole di `nvidia/parakeet-tdt-0.6b-v3` realizzata da Fermion Research; il modello e la documentazione ufficiale dichiarano supporto per l’inglese. FluidAudio lo espone come modello Core ML per Apple Neural Engine (ANE) su iOS 18/macOS 15. ([modello Fermion](https://huggingface.co/FermionResearch/Phonon-2), [Phonon-2 in FluidAudio](https://github.com/FluidInference/FluidAudio/blob/793a2c5a0e99705a55fefbad00e65ca684329349/Documentation/ASR/Phonon2.md), [sorgente FluidAudio](https://github.com/FluidInference/FluidAudio/blob/793a2c5a0e99705a55fefbad00e65ca684329349/Sources/FluidAudio/ASR/Parakeet/SlidingWindow/TDT/AsrModels.swift))

Per Sbobino va escluso dalla proposta di adozione: il multilingue è un requisito centrale confermato dall’utente. La velocità Apple Silicon è interessante come dato di ricerca sul runtime, ma non giustifica aggiungere un motore inglese specializzato.

La direzione da benchmarkare per l’uso estensivo di FluidAudio è quindi:

1. **Parakeet Ultra Core ML** come candidato orientato alla qualità multilingue;
2. **Parakeet Redux Core ML** come candidato più piccolo, con regressioni per lingua da misurare;
3. **Parakeet v3** come controllo stabile;
4. **Whisper** per lingue fuori dal perimetro Parakeet e per l’hardware/runtime in cui FluidAudio non è disponibile.

Questa è una conclusione documentale. Non sostituisce il benchmark sugli audio di Sbobino: non è stata eseguita un’inferenza locale in questo lavoro.

## Cosa dimostra il post e cosa no

Il post di Alex collega il risultato a FluidAudio e dichiara **5,6 secondi per un’ora**, **2 minuti e 19 secondi per 24 ore** e un passaggio da **480× a 644× real-time** su M5 Pro. Sono numeri riportati dall’autore del post, non un protocollo riproducibile pubblicato nel post. ([post X](https://x.com/Alex_tra_memory/status/2109868778615976113), [link breve indicato nel post](https://t.co/5lxvlmWDMq))

Il link breve risolve al modello [`FluidInference/phonon-2-coreml`](https://huggingface.co/FluidInference/phonon-2-coreml), non a EmbeddingGemma 2. Il repository FluidAudio integra infatti `FermionResearch/Phonon-2` attraverso quel pacchetto Core ML. ([catalogo modelli FluidAudio](https://github.com/FluidInference/FluidAudio/blob/793a2c5a0e99705a55fefbad00e65ca684329349/Documentation/Models.md), [nomi dei modelli nel codice](https://github.com/FluidInference/FluidAudio/blob/793a2c5a0e99705a55fefbad00e65ca684329349/Sources/FluidAudio/ModelNames.swift))

Il PR che contiene l’ottimizzazione corrente documenta risultati compatibili ma non identici al post: **5,55 s / 649× / WER 16,7%** su un’ora di Earnings-22 e **133,8 s / 646×** su 24 ore; il confronto precedente era 7,5 s / 480× e 142,1 s / 608×. Il PR è stato unito il 13 ottobre 2026 nel commit `793a2c5a0e99705a55fefbad00e65ca684329349`. ([PR #1019](https://github.com/FluidInference/FluidAudio/pull/1019), [commit HEAD](https://github.com/FluidInference/FluidAudio/commit/793a2c5a0e99705a55fefbad00e65ca684329349))

Il confronto Phonon-2/v3/Ultra pubblicato usa lo stesso M5 Pro; il nostro benchmark Sbobino e il demo EG2 precedente usano altre configurazioni. Non possiamo confrontare direttamente le loro velocità finché non fissiamo audio, hardware, revisione dei pesi, modalità di calcolo, warm-up e inclusione dell’I/O.

## Identità dei tre modelli

| Modello | Ruolo e origine | Lingue dichiarate | Runtime e compatibilità documentati | Pesi / codice |
|---|---|---|---|---|
| **EmbeddingGemma 2** | Modello multimodale di **embedding** di Google DeepMind. Produce vettori unificati per ricerca, clustering e RAG; la scheda non lo presenta come trascrittore di parole. | La scheda indica oltre 100 lingue per l’embedding. Questo non costituisce una prova di ASR in italiano. | Pipeline `feature-extraction`; la scheda distingue configurazioni testo, testo+audio e multimodale. Non è il runtime Core ML di FluidAudio. | La scheda del modello indica Apache-2.0. ([model card ufficiale](https://huggingface.co/google/embeddinggemma-2)) |
| **Phonon-2** | Riaddestramento QAT dell’encoder di Parakeet TDT v3 di Fermion Research. Gli encoder usano cinque valori appresi; decoder, tokenizer e contratto TDT restano della famiglia v3. | **Inglese.** La scheda Fermion e FluidAudio lo indicano esplicitamente. | FluidAudio: Swift/Core ML/ANE, `AsrModelVersion.phonon2`, iOS 18/macOS 15. Fermion pubblica anche runtime MLX e altri engine; ciò non rende FluidAudio multipiattaforma. | Modello CC-BY-4.0; codice/runtime pubblicati separatamente con licenze proprie, tra cui Apache-2.0 per `phonon-coreml`. ([scheda Fermion](https://huggingface.co/FermionResearch/Phonon-2), [NOTICE del modello](https://huggingface.co/FermionResearch/Phonon-2/blob/main/NOTICE), [runtime Core ML Fermion](https://github.com/fermionresearch/phonon-coreml), [FluidAudio LICENSE](https://github.com/FluidInference/FluidAudio/blob/793a2c5a0e99705a55fefbad00e65ca684329349/LICENSE)) |
| **Parakeet Redux** | Riaddestramento ternario di Parakeet v3 realizzato da moondream; stessa architettura, tokenizer e contratto TDT. | **25 lingue** della famiglia v3, incluso l’italiano nella valutazione FLEURS pubblicata dal model card. | Photon su CPU/MPS/CUDA nel progetto moondream; FluidAudio fornisce il modello Core ML/ANE per iOS 18/macOS 15. | Model card CC-BY-4.0. ([model card Redux](https://huggingface.co/moondream/parakeet-redux), [modelli FluidAudio](https://github.com/FluidInference/FluidAudio/blob/793a2c5a0e99705a55fefbad00e65ca684329349/Documentation/Models.md)) |

Il fatto che EmbeddingGemma 2 includa un encoder audio e molte lingue riguarda lo spazio degli embedding. Non dimostra che il modello possa sostituire un ASR o che abbia dato origine a Phonon-2. Le due famiglie vanno tenute separate nelle valutazioni.

## Origine del training e fatti di licenza

La `NOTICE` di Phonon-2 attribuisce a Fermion Research la riaddestramento e la quantizzazione dell’encoder; indica inoltre che architettura FastConformer, decoder TDT, tokenizer e configurazione delle feature derivano da Parakeet v3 di NVIDIA. Lo stesso documento elenca CHiME-6 e SPGISpeech tra gli audio di training e riporta le rispettive attribuzioni. Sono fatti riportati dai documenti dei modelli, non una valutazione giuridica. ([NOTICE Phonon-2](https://huggingface.co/FermionResearch/Phonon-2/blob/main/NOTICE), [card Parakeet Redux](https://huggingface.co/moondream/parakeet-redux))

Per una distribuzione bisogna tenere separati:

- i **pesi** del modello, che le schede Phonon-2 e Redux indicano come CC-BY-4.0;
- il **codice FluidAudio**, pubblicato nel repository con LICENSE Apache-2.0;
- il **runtime `phonon-coreml`**, la cui repository indica Apache-2.0, e le dipendenze/runtime elencati nella relativa documentazione.

Questa nota registra le licenze dichiarate dalle fonti; non interpreta obblighi di redistribuzione.

## Lingue e fedeltà attesa

Phonon-2 va escluso dalla selezione automatica per italiano e per una modalità “tutte le lingue”: la card Fermion dice “English” e la pagina FluidAudio lo descrive come inglese-only. Il tokenizer ereditato da v3 e la compatibilità del contratto TDT non aggiungono supporto linguistico che la scheda non dichiara. ([card Fermion](https://huggingface.co/FermionResearch/Phonon-2), [Phonon-2 FluidAudio](https://github.com/FluidInference/FluidAudio/blob/793a2c5a0e99705a55fefbad00e65ca684329349/Documentation/ASR/Phonon2.md))

Per Redux, il model card pubblica sulla FLEURS test un WER italiano di **3,24** contro **2,61** di Parakeet v3. La stessa card riporta aggregate diverse a seconda del sottoinsieme e del tipo di rumore; questi numeri vanno quindi trattati come indicazioni da replicare, non come un verdetto per i file di Sbobino. ([card Redux](https://huggingface.co/moondream/parakeet-redux))

FluidAudio documenta inoltre Parakeet Ultra come post-training di v3 con le stesse 25 lingue e una valutazione migliore di v3 nei benchmark pubblicati, inclusa la media FLEURS riportata nel catalogo. La dimensione dell’encoder int8 è indicata come 595 MB; il pacchetto ASR completo è circa 632 MB secondo i metadata verificati; è un candidato di qualità da verificare, non una raccomandazione già validata sul corpus Sbobino. ([catalogo FluidAudio](https://github.com/FluidInference/FluidAudio/blob/793a2c5a0e99705a55fefbad00e65ca684329349/Documentation/Models.md), [documentazione Ultra](https://github.com/FluidInference/FluidAudio/blob/793a2c5a0e99705a55fefbad00e65ca684329349/Documentation/ASR/ParakeetUltra.md), [benchmark FluidAudio](https://github.com/FluidInference/FluidAudio/blob/793a2c5a0e99705a55fefbad00e65ca684329349/Documentation/Benchmarks.md))

## Runtime nativo e artefatti del modello

Il percorso FluidAudio è nativo Swift/Core ML: `AsrModelVersion` include `.phonon2`, il modello è risolto verso `FluidInference/phonon-2-coreml`, e il caricamento usa le unità Core ML con ANE come configurazione predefinita. Il controllo piattaforma del codice richiede iOS 18 o macOS 15 per Phonon-2 e Redux. ([`AsrModels.swift`](https://github.com/FluidInference/FluidAudio/blob/793a2c5a0e99705a55fefbad00e65ca684329349/Sources/FluidAudio/ASR/Parakeet/SlidingWindow/TDT/AsrModels.swift), [`ModelNames.swift`](https://github.com/FluidInference/FluidAudio/blob/793a2c5a0e99705a55fefbad00e65ca684329349/Sources/FluidAudio/ModelNames.swift), [Phonon-2 Core ML](https://huggingface.co/FermionResearch/Phonon-2-CoreML))

La repository Fermion `phonon-coreml` documenta un package Swift per Apple Silicon/macOS 15, preparazione iniziale dell’ANE e trascrizione a finestre; la card Core ML documenta anche taglio su pause e finestre fino a 15 secondi per l’audio lungo. Questo è un percorso nativo utile come riferimento, ma non prova che l’attuale bridge Rust/Tauri di Sbobino possa selezionarlo. ([runtime Fermion](https://github.com/fermionresearch/phonon-coreml), [card Core ML](https://huggingface.co/FermionResearch/Phonon-2-CoreML))

### Revisione e dimensioni da congelare

- FluidAudio auditato: `793a2c5a0e99705a55fefbad00e65ca684329349` (HEAD del 13 ottobre 2026; PR #1019 appena unito). ([commit](https://github.com/FluidInference/FluidAudio/commit/793a2c5a0e99705a55fefbad00e65ca684329349), [PR #1019](https://github.com/FluidInference/FluidAudio/pull/1019))
- Modello HF `FluidInference/phonon-2-coreml`: revisione API `1702fc22e76f203d5ca464cd5d3ef130a45ce2c0`. ([pagina revisionata](https://huggingface.co/FluidInference/phonon-2-coreml/tree/1702fc22e76f203d5ca464cd5d3ef130a45ce2c0))
- La configurazione ladder corrente usa un encoder condiviso da **341.127.152 byte** e cinque componenti per **378.075.366 byte** complessivi nella revisione HF sopra indicata; i componenti includono decoder, joint, preprocessor e vocabulary. ([metadati HF del modello](https://huggingface.co/api/models/FluidInference/phonon-2-coreml?blobs=true))
- La documentazione corrente contiene ancora una tabella precedente che descrive l’encoder `Encoder_sparse-g8.mlmodelc` come **321 MB**; la sezione ladder di PR #1019 descrive invece il blob condiviso corrente da **341 MB** con funzioni 5,04/10,16/15 secondi. Le due cifre non vanno fuse in un unico risultato. ([tabella modelli, sezione ancora precedente](https://github.com/FluidInference/FluidAudio/blob/793a2c5a0e99705a55fefbad00e65ca684329349/Documentation/Models.md), [ladder corrente](https://github.com/FluidInference/FluidAudio/blob/793a2c5a0e99705a55fefbad00e65ca684329349/Documentation/ASR/Phonon2.md), [PR #1019](https://github.com/FluidInference/FluidAudio/pull/1019))

Il PR segnala anche che la cache locale può mantenere encoder/chunker precedenti finché non viene rimossa la directory del modello Phonon-2. Ogni benchmark deve registrare SHA del modello, commit FluidAudio e stato della cache; altrimenti una misura può usare ancora la forma singola da 321 MB. ([PR #1019](https://github.com/FluidInference/FluidAudio/pull/1019), [nomi e revisioni nel codice](https://github.com/FluidInference/FluidAudio/blob/793a2c5a0e99705a55fefbad00e65ca684329349/Sources/FluidAudio/ModelNames.swift))

## Numeri pubblicati da FluidAudio: cosa misurano

I valori seguenti sono benchmark del progetto FluidAudio su inglese e non sono misure del corpus Sbobino:

| Prova pubblicata | v3 | Ultra | Phonon-2 | Lettura corretta |
|---|---:|---:|---:|---|
| LibriSpeech test-clean WER | 2,27% | 2,13% | 2,47% | Phonon-2 è meno accurato nel test inglese pubblicato. |
| LibriSpeech test-other WER | 4,12% | 3,81% | 4,62% | Ultra precede i due modelli nel dato pubblicato. |
| LibriSpeech test-clean RTFx | 149–152× | 151× | 193× nella ladder corrente | La velocità Phonon-2 è il vantaggio principale su ANE. |
| Earnings-22, un’ora, WER | 16,5% | 13,5% | 16,7% nella ladder corrente | Sul lungo-formato pubblicato è il più veloce ma non il più fedele. |
| Earnings-22, un’ora, tempo | 10,7 s | 7,9 s | 5,55 s / 649× | Include il protocollo del benchmark FluidAudio; non è una misura Sbobino. |

Le fonti attribuiscono il guadagno recente di Phonon-2 alla ladder di finestre e al taglio sulle pause, non a una superiorità linguistica. Il report deve dunque separare fedeltà, gestione dei confini e velocità. ([Phonon2.md](https://github.com/FluidInference/FluidAudio/blob/793a2c5a0e99705a55fefbad00e65ca684329349/Documentation/ASR/Phonon2.md), [Benchmarks.md](https://github.com/FluidInference/FluidAudio/blob/793a2c5a0e99705a55fefbad00e65ca684329349/Documentation/Benchmarks.md), [PR #1019](https://github.com/FluidInference/FluidAudio/pull/1019))

Lo stesso PR riporta per la prova da 24 ore un RSS di picco da 5,8 GB a 3,0 GB dopo il passaggio alla mappatura PCM; la mappatura di un WAV PCM16 mono da 24 ore a 16 kHz corrisponde a circa 2,76 GB di audio (calcolo del coordinatore), e il dato non equivale alla memoria del modello. Inoltre il caricamento/compilazione del modello è fuori dal timer caldo. Questi numeri non vanno usati come requisito RAM di Sbobino senza una misura che separi pesi, cache Core ML, audio mappato e processo completo. ([PR #1019](https://github.com/FluidInference/FluidAudio/pull/1019))

La scheda upstream di Phonon-2 riporta una media WER di 5,21 su sette set inglesi e un download encoder da 164 MB per il formato MLX; la variante Core ML e la ladder FluidAudio hanno artefatti e misure differenti. Non è corretto usare il numero MLX come memoria o accuratezza della distribuzione Core ML. ([card upstream](https://huggingface.co/FermionResearch/Phonon-2), [card Core ML](https://huggingface.co/FermionResearch/Phonon-2-CoreML))

## Implicazione per Sbobino

La scelta coerente con il prodotto è **non adottare Phonon-2 nella pipeline principale**. Non copre italiano e non aumenta la copertura multilingue; introdurrebbe un modello, una selezione per lingua, una cache e un percorso Apple-only che non sono necessari per il requisito centrale dell’app.

Il prossimo confronto FluidAudio dovrebbe usare audio italiani e multilingue identici per v3, Ultra e Redux, con Whisper come fallback per le lingue non coperte o per le piattaforme non Apple. Va misurato almeno:

- WER/CER e fedeltà letterale per lingua, termini tecnici, nomi e numeri;
- errori su parlato veloce, rumore, silenzio e file lunghi;
- qualità ai confini delle finestre e duplicazioni dopo la ricomposizione;
- tempo a freddo, tempo a caldo, RTF, memoria e peso installato;
- disponibilità effettiva su macOS/iOS e comportamento su hardware non Apple.

Solo se un benchmark Sbobino dimostrerà un vantaggio concreto per l’italiano e per le altre lingue prioritarie si potrà considerare Ultra o Redux per l’uso estensivo. Un miglioramento medio non basta: ogni lingua deve avere risultati separati, incluse le regressioni, e le lingue fuori dalle 25 dichiarate richiedono un altro ASR. La policy nativa deve risolvere lo stesso motore per disponibilità e avvio, considerando lingua, piattaforma, sistema operativo, modello disponibile e memoria.

## Verifica del codice e del bridge Rust

Il coordinatore ha ispezionato i sorgenti, senza compilarli o eseguirli. Il repository [`fluidaudio-rs`](https://github.com/FluidInference/fluidaudio-rs/tree/bd1f0f36cb7d43e8a564b38dc01eb365864b3b79) al commit `bd1f0f36cb7d43e8a564b38dc01eb365864b3b79` offre un bridge Swift statico tramite ABI C. Il [manifest Swift](https://github.com/FluidInference/fluidaudio-rs/blob/bd1f0f36cb7d43e8a564b38dc01eb365864b3b79/Package.swift) fissa FluidAudio **0.14.8** (revision `56607d90b97de7d95a731200563bf49aa5beef20`); il [bridge](https://github.com/FluidInference/fluidaudio-rs/blob/bd1f0f36cb7d43e8a564b38dc01eb365864b3b79/swift/FluidAudioBridge.swift) inizializza ASR con `downloadAndLoad()` senza selezione pubblica v3/Ultra/Redux/Phonon-2. Il suo [build script](https://github.com/FluidInference/fluidaudio-rs/blob/bd1f0f36cb7d43e8a564b38dc01eb365864b3b79/build.rs) collega framework Apple e runtime Swift. È una base nativa possibile per Tauri, ma occorre aggiornare e validare il bridge; non porta CoreML su Windows/Linux.

Nei manifest Cargo/package di Sbobino non è presente una dipendenza FluidAudio. Nessuna integrazione eseguita. La [release upstream più recente verificata](https://github.com/FluidInference/FluidAudio/releases/tag/v0.17.7) è v0.17.7 dell’8 ottobre; #1019 è invece nel main del 13 ottobre. Per riprodurre questa ottimizzazione bisogna fissare il commit, non assumere che il tag corrente la contenga.

La nuova strategia di finestre senza overlap è attivata da `hasEncoderLadder`: gli encoder a forma singola di v3/Ultra/Redux mantengono il percorso precedente. Le funzioni corte e i tagli sulle pause di Phonon-2 non accelerano automaticamente gli altri pesi. Senza pausa valida il codice conserva l’overlap; i test upstream aggiunti coprono selezione delle pause, slicing dei mel e lettura PCM, ma non sono stati eseguiti qui. Il [codice delle finestre](https://github.com/FluidInference/FluidAudio/blob/793a2c5a0e99705a55fefbad00e65ca684329349/Sources/FluidAudio/ASR/Parakeet/SlidingWindow/TDT/ChunkProcessor.swift) consente di verificare questi vincoli.

Il [timer batch](https://github.com/FluidInference/FluidAudio/blob/793a2c5a0e99705a55fefbad00e65ca684329349/Sources/FluidAudioCLI/Commands/ASR/Parakeet/SlidingWindow/TranscribeCommand.swift) parte dopo il caricamento del modello e misura `transcribe(url)`, comprendendo la lettura/conversione nel percorso file. Il demo documenta la stessa esclusione del caricamento. Per la ladder, la prima compilazione ANE è dichiarata circa 55 s per funzione sul M5 Pro, circa tre minuti complessivi; i successivi caricamenti sfruttano la cache. Disco, RSS e allocazioni CoreML sono misure diverse; il WAV da 24 ore a 16 kHz mono PCM16 occupa circa **2,76 GB**, calcolati da durata × frequenza × due byte.

I metadata HF scaricati fissano anche le alternative multilingue:

| Modello CoreML | Revisione HF | Componenti ASR standard su disco, byte |
|---|---|---:|
| Ultra | `95eaa59a39d4394f047a4dc5cce480388a60d1b6` | 632.162.964 |
| Redux | `8c5ef97a29cd120dc76b354b3f22b7fec3b486f9` | 220.304.193 |

Totali ricavati dai metadata di encoder, decoder, joint, preprocessor e vocabolario: non comprendono cache compilate, binario o memoria residente. Le [card Ultra](https://huggingface.co/FluidInference/parakeet-ultra-coreml) e [Redux](https://huggingface.co/FluidInference/parakeet-redux-coreml) dichiarano entrambe 25 lingue, italiano incluso. Il WER medio FLEURS pubblicato su 24 lingue × 100 clip è v3 14,81%, Ultra 11,67%, Redux 13,06%; Ultra migliora tutte le 24 lingue secondo gli autori, Redux soltanto 13. Sono dati pubblicati, non misure locali. Redux riduce il download ma non garantisce maggiore velocità o minore RAM: sul loro ANE è più lento di v3 e la prima compilazione può durare diversi minuti; va confrontato anche sul GPU del Mac reale.

Sorgenti e snapshot conservati su `/Volumes/UltraDisk/sbobino-phonon2-audit-20261013/`. I modelli grandi non sono stati scaricati, né è stata eseguita inferenza. Il prossimo spike deve confrontare le baseline Sbobino e i tre candidati multilingue sullo stesso Mac M3, usando corpus pubblici congelati, avvio freddo e tre misure a caldo, file completi e valutazione per lingua. Nessuna promessa di miglioramento su M3, Intel o Windows deriva da questa verifica documentale.

## Fonti primarie e identificativi congelati

- FluidAudio repository e licenza: [repository](https://github.com/FluidInference/FluidAudio), [LICENSE al commit auditato](https://github.com/FluidInference/FluidAudio/blob/793a2c5a0e99705a55fefbad00e65ca684329349/LICENSE).
- FluidAudio HEAD: [`793a2c5a0e99705a55fefbad00e65ca684329`](https://github.com/FluidInference/FluidAudio/commit/793a2c5a0e99705a55fefbad00e65ca684329349).
- Integrazione Phonon-2: [Phonon2.md](https://github.com/FluidInference/FluidAudio/blob/793a2c5a0e99705a55fefbad00e65ca684329349/Documentation/ASR/Phonon2.md), [Models.md](https://github.com/FluidInference/FluidAudio/blob/793a2c5a0e99705a55fefbad00e65ca684329349/Documentation/Models.md), [PR #980](https://github.com/FluidInference/FluidAudio/pull/980), [PR #1019](https://github.com/FluidInference/FluidAudio/pull/1019).
- FluidAudio API nativa: [`AsrModels.swift`](https://github.com/FluidInference/FluidAudio/blob/793a2c5a0e99705a55fefbad00e65ca684329349/Sources/FluidAudio/ASR/Parakeet/SlidingWindow/TDT/AsrModels.swift), [`ModelNames.swift`](https://github.com/FluidInference/FluidAudio/blob/793a2c5a0e99705a55fefbad00e65ca684329349/Sources/FluidAudio/ModelNames.swift).
- Artefatto Core ML: [card `FluidInference/phonon-2-coreml`](https://huggingface.co/FluidInference/phonon-2-coreml), [revisione `1702fc22e76f203d5ca464cd5d3ef130a45ce2c0`](https://huggingface.co/FluidInference/phonon-2-coreml/tree/1702fc22e76f203d5ca464cd5d3ef130a45ce2c0), [API metadati](https://huggingface.co/api/models/FluidInference/phonon-2-coreml?blobs=true).
- Modello e origini: [FermionResearch/Phonon-2](https://huggingface.co/FermionResearch/Phonon-2), [NOTICE](https://huggingface.co/FermionResearch/Phonon-2/blob/main/NOTICE), [Phonon-2-CoreML](https://huggingface.co/FermionResearch/Phonon-2-CoreML), [runtime Swift](https://github.com/fermionresearch/phonon-coreml/releases/tag/1.1.2).
- Alternative multilingue: [Parakeet Redux](https://huggingface.co/moondream/parakeet-redux), [Parakeet Ultra](https://github.com/FluidInference/FluidAudio/blob/793a2c5a0e99705a55fefbad00e65ca684329349/Documentation/ASR/ParakeetUltra.md), [catalogo modelli](https://github.com/FluidInference/FluidAudio/blob/793a2c5a0e99705a55fefbad00e65ca684329349/Documentation/Models.md).
- Modello embedding da non confondere con Phonon-2: [Google EmbeddingGemma 2](https://huggingface.co/google/embeddinggemma-2).
- Origine del segnale pubblico: [post Alex](https://x.com/Alex_tra_memory/status/2109868778615976113), [post Shahir richiamato](https://x.com/TslShahir/status/2109660368381395414).

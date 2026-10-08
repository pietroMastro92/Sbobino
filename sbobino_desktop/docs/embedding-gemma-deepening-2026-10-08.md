# EmbeddingGemma: approfondimento su registrazioni lunghe

## Esito del recupero

Gli embedding migliorano il recupero di passaggi rispetto alle due baseline lessicali su questo holdout pubblico. **EmbeddingGemma 2 non è automaticamente la scelta migliore**: Gemma 1 Q8 mette più spesso il supporto al primo posto; Gemma 2 ha circa metà dei pesi di Gemma 1 Q8 e offre una strada nativa verificata sul Mac. Nessuna integrazione o modifica della pipeline ASR in questo spike.

Il modello provato è **EmbeddingGemma 2 text 270M**: recupera informazioni, non genera risposte e non corregge il transcript. Il controllo generativo separato usa Gemma 4 E4B soltanto per selezionare una citazione.

## Holdout congelato

- Apple M3, 16 GiB, macOS ARM64. Quattro thread CPU, un modello residente alla volta.
- 104 passaggi: 17 da un talk italiano di sviluppo, 76 da quattro talk italiani di test, 11 da AMI ES2002a inglese (52–626 s, non la riunione intera). Circa 90 minuti di audio coperto, con passaggi vicini e temi ricorrenti.
- Ogni documento riunisce due chunk reali consecutivi nominalmente di 28 s (ultimo chunk blockchain: 18,898 s), overlap 2 s, tramite `asr_spike.py::stitch`. Le coppie non condividono chunk; intervalli adiacenti mantengono l'overlap audio. Ultimi chunk senza una coppia esclusi. Nessun timestamp di parola inventato.
- Variante manuale e variante Parakeet Q8 già trascritta nello spike pubblico: stessi ID, domande ed etichette. Nessuna nuova trascrizione, nessun audio privato.
- 12 domande dev (8 rispondibili, 4 negative), 40 test (28 rispondibili, 12 negative). Query in italiano, inglese, francese, spagnolo e tedesco; documenti soltanto italiani/inglesi, senza traduzione.
- Domande ed etichette curate sulla fonte pubblica; revisione indipendente prima dell'inferenza. Corrette quattro incongruenze prima del congelamento: supporto Kevlar, supporto Kurzweil, quinta proprietà blockchain, registrazione della domanda sull'associazione industriale. L'ultimo ID resta `q-test-opt-08`, ma la registrazione è meccanica.
- Il test è separato dal dev per registrazione. Resta esplorativo e curato: 28 domande positive non certificano tutte le lingue, l'intero archivio o domande reali degli utenti.
- Scope principale: singola registrazione. Scope archivio: tutte le 104 finestre; domande come “il relatore” sono ambigue fuori dalla registrazione e il gold resta specifico alla fonte.

La metrica conta **documenti con supporto sufficiente**, non semplice pertinenza tematica. Su Parakeet il gold indica l'intervallo manuale: un hit può contenere errori ASR o perdere l'informazione. Per questo segue un controllo della citazione effettiva.

I sottotitoli manuali vengono inclusi per intersezione con il chunk e possono superarne il confine: fino a 9,393 s nel documento AMI pair-01. Il confronto manuale/ASR comprende quindi anche questo effetto di costruzione, non soltanto errori di trascrizione. Gli intervalli riportati sono finestre di recupero, non localizzazioni esatte della frase; `source.segments` conserva gli intervalli manuali originali.

## Supporto nel primo risultato / nei primi cinque

256 dimensioni, prefissi HF fissi, nessuna ottimizzazione sul test. L'euristica replica `artifacts.rs::score_chunk` e la selezione iniziale; non esegue i budget di contesto o la chat dell'app.

| Metodo | Registrazione, manuale | Registrazione, Parakeet | Archivio, manuale | Archivio, Parakeet |
|---|---:|---:|---:|---:|
| Euristica attuale chat | 16/28 · 23/28 | 15/28 · 22/28 | 14/28 · 16/28 | 13/28 · 16/28 |
| BM25 | 16/28 · 22/28 | 16/28 · 21/28 | 16/28 · 18/28 | 16/28 · 18/28 |
| Gemma 1 Q8 CPU | 26/28 · 28/28 | 28/28 · 28/28 | 25/28 · 28/28 | 26/28 · 28/28 |
| Gemma 1 QAT Q4 CPU | 24/28 · 28/28 | 23/28 · 28/28 | 23/28 · 27/28 | 20/28 · 27/28 |
| Gemma 2 LiteRT CPU | 23/28 · 28/28 | 22/28 · 28/28 | 22/28 · 28/28 | 21/28 · 27/28 |
| Gemma 2 LiteRT GPU | 23/28 · 28/28 | 24/28 · 28/28 | 22/28 · 27/28 | 23/28 · 27/28 |

BM25 sulle domande italiane rispondibili nella registrazione recupera 11/11 al primo posto su entrambe le varianti; il beneficio degli embedding emerge soprattutto nel passaggio tra lingue. Su Parakeet, Gemma 1 Q8 raggiunge 17/17 al primo posto nelle query non italiane, Gemma 2 CPU 14/17, BM25 5/17. Quattro di queste query riguardano documenti inglesi: non sono tutti casi di traduzione incrociata. I piccoli conteggi per lingua e registrazione sono in `results/summary.json`.

A 128 dimensioni alcuni supporti escono dai primi cinque: su manuale Gemma 2 26/28, Gemma 1 Q4 27/28; su Parakeet Gemma 1 Q8 27/28 e Q4 26/28. A 768 Gemma 1 Q8 arriva 27/28 al primo posto sul manuale e 28/28 su Parakeet, Gemma 2 CPU 24/28 su entrambi. Non scegliamo la dimensione sul risultato del test: 256 era la configurazione principale predefinita. CPU e GPU LiteRT non producono sempre lo stesso ranking; sullo scope archivio il supporto Gemma 2 passa da 28 a 27 casi nei riferimenti usando GPU.

## La similarità non decide se esiste una risposta

Soglia fissata come il valore immediatamente superiore al massimo cosine delle quattro negative dev, poi applicata al test senza ritocchi. Le scale dei modelli non sono intercambiabili.

| Modello CPU, scope registrazione | Manuale: positive accettate / false accettazioni negative | Parakeet: positive accettate / false accettazioni negative |
|---|---:|---:|
| Gemma 1 Q8 | 15/28 · 2/12 | 18/28 · 3/12 |
| Gemma 1 Q4 | 5/28 · 1/12 | 7/28 · 3/12 |
| Gemma 2 | 12/28 · 3/12 | 16/28 · 5/12 |

Esempio: la data precisa della visita in fabbrica, la marca della pompa di Parmitano e la ragione del target di vendite AMI non sono presenti; query tematicamente vicine vengono comunque accettate. È una diagnosi della policy provata, non la ricerca della soglia ottimale. Quattro negative dev sono insufficienti per una soglia universale; una citazione deve dimostrare proprio il fatto richiesto.

## Prestazioni locali

104 documenti manuali, 52 query ripetute tre volte. Tempi a caldo includono trasporto HTTP per llama.cpp. Un secondo run Q4 senza scoring concorrente misura le prestazioni; le sue codifiche non sostituiscono il risultato qualitativo. LiteRT è stato rieseguito con overflow impostato a errore: nessun testo supera la finestra e i vettori preesistenti rimangono identici (delta massimo 0). llama.cpp non segnala troncamenti.

| Configurazione | Pesi su disco | Indicizzazione | Query mediana / p95 | Footprint di processo |
|---|---:|---:|---:|---:|
| Gemma 1 Q8 CPU | 333,6 MB | 18,83 s | 11,0 / 14,0 ms | server 724,2 MB |
| Gemma 1 QAT Q4 CPU | 277,9 MB | 20,32 s | 11,0 / 15,4 ms | server 667,7 MB |
| Gemma 2 LiteRT CPU | 164,6 MB | 12,54 s | 48,3 / 60,7 ms | Python + runtime 341,5 MB |
| Gemma 2 LiteRT GPU | 164,6 MB | 4,27 s | 22,7 / 25,7 ms | Python + runtime 410,3 MB |

MB decimali. Picchi RSS distinti: server Q8 822,8 MB, server Q4 730,4 MB, Gemma 2 CPU 514,0 MB, GPU 358,4 MB. Il client Python llama.cpp è separato; i valori server non sono la pipeline completa. RSS e footprint non sono sommati, né viene aggiunta una presunta VRAM alla memoria unificata. I picchi server coprono entrambi i corpus, il picco Q4 anche la ripetizione prestazionale; LiteRT è un processo per corpus. Wrapper, quantizzazioni e campioni di memoria differiscono: questi numeri descrivono le configurazioni, non isolano l'architettura del modello.

Gemma 2 CPU apre in 0,45 s e fa warmup in 0,31 s; GPU 0,53/0,21 s, con cache già disponibile. Non sono misure del primo avvio su un computer nuovo. Le ripetizioni dei vettori a caldo coincidono entro 1e-5. Un indice binario float32 a 256 dimensioni richiederebbe 10,24 MB per 10.000 passaggi, esclusi testo e metadati: è un calcolo, non un benchmark su un archivio grande.

## Percorso nativo verificato

Un programma C++ standalone ha collegato l'[API C LiteRT-LM v0.18.0](https://raw.githubusercontent.com/google-ai-edge/LiteRT-LM/v0.18.0/c/embedding_engine.h) ed eseguito Gemma 2 CPU sul Mac senza Python: 768 valori finiti, norma L2 0,999999714, creazione engine 338 ms, prima codifica 351 ms. Footprint 176,5 MB, RSS 292,5 MB. È uno smoke con una query, thread predefiniti e senza prefisso di ricerca, non il benchmark completo e non un confronto di qualità.

Il [framework macOS ufficiale v0.18.0](https://github.com/google-ai-edge/LiteRT-LM/releases/download/v0.18.0/CLiteRTLM_mac.xcframework.zip) contiene arm64 e x86_64; zip 45,1 MB, dylib universale estratta 137,8 MB. L'esecuzione verificata è solo arm64. L'API C permette un binding Rust, ma binding, lifecycle, cancellazione, rpath, firma e pacchetto Tauri non sono stati implementati. Versione e header vanno fissati insieme: non usare header `main` con questo binario.

Per Windows esiste il [percorso di compilazione x86_64 da sorgenti](https://raw.githubusercontent.com/google-ai-edge/LiteRT-LM/v0.18.0/docs/getting-started/build-and-run.md); la release v0.18.0 non offre un archivio C ABI Windows standalone equivalente. Nessuna prova Intel/Windows, nessuna promessa di performance su CPU vecchie. Le model card dichiarano Apache-2.0 per [Gemma 2 LiteRT](https://huggingface.co/litert-community/embeddinggemma-2-text-270m-litert-lm) e termini Gemma per [Gemma 1 QAT Q4](https://huggingface.co/ggml-org/embeddinggemma-300M-qat-q4_0-GGUF).

## Controllo generativo delle citazioni

16 domande test congelate: 8 rispondibili, 8 negative pertinenti, attraverso BM25/Gemma 1 Q8/Gemma 2 CPU e su entrambe le varianti (96 richieste). Generatore unico Gemma 4 E4B Q4_0, llama.cpp Metal, contesto 8192, uno slot, thinking off, temperatura 0, top-k 1, massimo 256 token. Il modello embedding non genera queste risposte.

Il prompt chiede JSON `answered`/`not_found`, ID di un documento e una citazione contigua esatta; vieta parafrasi, traduzioni, unione di documenti e supposizioni. I primi cinque documenti della stessa registrazione sono riordinati cronologicamente. Stesse istruzioni per tutti i candidati, nessun invio al generatore delle etichette o delle risposte attese. Nessuna soglia cosine viene applicata qui, per isolare la qualità del contesto e la capacità di astensione.

Il controllo richiede ID nel contesto e nel supporto gold, citazione letteralmente presente nel documento, e tutti i termini essenziali della risposta. Soltanto il controllo dei termini normalizza punteggiatura; la citazione conserva segni e percentuali. Questo è un **test di selezione della prova**, non accuratezza semantica di una risposta libera e non la chat dell'app. Le negative `answered` contano comunque come false accettazioni; risposte invalide e troncate hanno conteggi separati.

| Contesto fornito a Gemma 4 | Citazioni positive valide, manuale | Citazioni positive valide, Parakeet | False risposte su negative, entrambe | Astensioni corrette, entrambe |
|---|---:|---:|---:|---:|
| BM25 | 4/8 | 6/8 | 2/8 | 6/8 |
| Gemma 1 Q8 | 5/8 | 7/8 | 2/8 | 6/8 |
| Gemma 2 CPU | 5/8 | 8/8 | 2/8 | 6/8 |

Tutte le 96 richieste sono complete, senza JSON invalidi o output troncati. BM25 si astiene anche su una positiva in ciascuna variante; i due embedding non si astengono sulle positive. Lo scarto di un caso tra Gemma 1 e Gemma 2 su Parakeet è piccolo e non dimostra superiorità generale.

Casi verificati, con finestre audio conservate nei risultati:

- **Ottimismo, 0–54 s, `q-test-opt-01`:** domanda spagnola sui due luoghi possibili del talk. BM25 non fornisce la prova utile e il generatore si astiene; i due embedding recuperano Silicon Valley e Rovigo/Polesine e producono la citazione richiesta.
- **Ottimismo, 260–314 s, `q-test-opt-07`:** si chiede un anno preciso per l'inizio della rivoluzione industriale. La fonte dice soltanto “fine del XVII secolo”; tutti rispondono `answered`. È un dettaglio non soddisfatto dalla citazione, anche quando la citazione è letterale, non l'invenzione di un anno specifico.
- **Meccanica, 468–522 s, `q-test-opt-08`:** si chiede il nome ufficiale dell'associazione. Tutti citano la generica associazione degli industriali; la fonte non fornisce il nome ufficiale. Ancora un falso `answered`, senza necessariamente inventare una nuova parola.
- **Spazio, 104–158 s, `q-test-space-02`, manuale:** tutti trasformano `bicchier` in `bicchiere`. Il fatto generale è supportato, ma la citazione richiesta non è letterale.
- **AMI, 52–106 s, `q-test-ami-01`, manuale:** tutti identificano Laura, ma ricopiano `manager .` come `manager.`. Il controllo letterale fallisce per spaziatura: questo non viene descritto come allucinazione fattuale.
- **Blockchain, 208–262 s, `q-test-bc-02`:** la citazione manuale scelta non contiene la definizione gold; i due embedding portano il generatore ad aggiungere una continuazione assente dal documento e dal contesto fornito. Su Parakeet Gemma 2 produce la definizione completa, Gemma 1 un beneficio economico generico. La domanda francese permette una lettura di “definizione” o “beneficio”: il limite è segnalato, senza correggere le etichette dopo i risultati.

Il punteggio stretto mescola violazioni sostanziali e dettagli di punteggiatura perché il compito imponeva copia esatta. Non significa 5/8 di accuratezza fattuale. Inoltre una citazione valida del transcript ASR non certifica che quel transcript sia fedele all'audio: questo probe non riascolta né corregge il parlato.

Generazione residente: mediana 8,98 s, p95 16,17 s, massimo 25,03 s; 947,2 s totali nelle 96 chiamate. Il server registra circa 961,7 s dall'avvio all'arresto. Footprint 6.933,1 MB, RSS 4.238,2 MB; client e embedding sono separati. RSS e footprint non vanno sommati tra loro né a ulteriori allocazioni GPU. L'ordine fisso dei candidati e la cache dei prefissi influenzano le latenze: non confrontiamo i tempi per dichiarare un vincitore del retrieval. Non è una misura end-to-end della chat con indicizzazione e ASR simultanei.

Il guadagno tra manuale e Parakeet dipende anche dal formato, ad esempio dalla punteggiatura AMI, e non prova che gli errori ASR aiutino. L'esito pratico è favorevole al recupero semantico e sfavorevole all'idea che ranking, soglia o istruzioni rigide bastino a rendere sicura una risposta generata.

## Proposta focalizzata su EmbeddingGemma 2 per Sbobino

**La funzione da portare per prima è «trova dove si parla di…», con passaggio originale e accesso all'audio.** EmbeddingGemma 2 permette di cercare per significato senza generare una risposta, riscrivere il transcript o caricare un modello generativo aggiuntivo. È un miglioramento della consultazione dopo la trascrizione: non rende l'ASR più preciso o veloce.

Esempio di esperienza: una domanda in spagnolo sui luoghi citati nel talk italiano recupera il passaggio su Silicon Valley e Rovigo/Polesine. Nella funzione proposta, l'interfaccia mostrerebbe il testo salvato e aprirebbe la finestra audio disponibile, così l'utente potrebbe verificare ciò che è stato detto. Questo caso è presente nel test. La funzione non deve mostrare un riassunto inventato come risultato della ricerca.

| Priorità | Funzione | Evidenza e limite |
|---|---|---|
| 1 | Ricerca per significato nella registrazione aperta, con estratti e riascolto | Gemma 2 CPU trova il supporto nei primi cinque in 28/28 query positive Parakeet. Prima area da validare nell'app, essendo lo scope principale del benchmark. |
| 2 | Ricerca nell'archivio completo, rispettando workspace e filtri | 27/28 nei primi cinque sul piccolo archivio Parakeet. Serve una prova su archivi grandi e su domande ambigue tra registrazioni. |
| 3 | Migliore selezione dei passaggi per la chat esistente | Il probe Parakeet ottiene 8/8 citazioni positive valide, ma 2/8 false risposte su negative. Migliora il contesto; non certifica la risposta del generatore. |
| Successiva | Passaggi correlati e raggruppamenti tematici suggeriti | Potenziale coerente con il modello, non misurato nello spike. Nessuna classificazione o riorganizzazione automatica dei documenti in questa fase. |

La ricerca esatta resta disponibile per nomi, codici, numeri e frasi letterali. Sul campione italiano BM25 recupera già 11/11 al primo posto: non c'è motivo di eliminare il percorso lessicale. La futura combinazione con il ranking semantico va misurata, senza sommare direttamente punteggi cosine e lessicali con scale diverse. In assenza del modello, o se il runtime fallisce, la ricerca testuale continua a funzionare. Un risultato semanticamente vicino viene presentato come passaggio pertinente, non come prova che contenga ogni dettaglio richiesto.

### Inserimento minimo nei flussi esistenti

La mappatura riguarda il checkout verificato, non codice già implementato:

- **Ricerca:** `apps/desktop/src/App.tsx::filteredArtifacts` oggi applica `includes` a titolo, fonte e transcript dei soli artifact caricati; lo storico iniziale usa `listRecentArtifacts` con limite 40. Per la ricerca completa serve recupero nel backend vicino a `commands/artifacts.rs::list_artifacts`, esposto tramite `src/lib/tauri.ts`. Riutilizzare apertura documento e seek audio esistenti; i risultati devono includere artifact, revisione, fonte testuale, posizione del passaggio e tempi reali quando disponibili. Filtri workspace/tipo ed esclusione degli eliminati precedono la selezione finale dei risultati.
- **Chat:** `commands/artifacts.rs::build_chat_context_candidates` è il punto del recupero, prima del packing. Conservare il provider e il fallback di `TranscriptEnhancer` esistenti: EmbeddingGemma 2 restituisce vettori e non è un enhancer generativo. Il packing attuale ordina cronologicamente prima di limitare numero e caratteri; può quindi scartare un passaggio prioritario tardivo. La selezione deve rispettare il budget in ordine di rilevanza e poi ordinare temporalmente i passaggi scelti. Il benchmark non ha esercitato questi budget dell'app.
- **Fonte e citazioni:** `commands/prepared_transcript.rs::effective_transcript` privilegia il testo ottimizzato; `PreparedTranscriptContext::from_artifact` riduce inoltre le ripetizioni nel testo AI. Non usare quel testo trasformato come citazione letterale dell'originale. Riutilizzare il parser della timeline, verificando che segmenti e testo indicizzato appartengano alla stessa versione. Conservare le ripetizioni autentiche; distinguere chiaramente transcript originale e vista ottimizzata. Senza tempi non inventare timestamp; senza audio disponibile non offrire il riascolto.
- **Persistenza:** `crates/domain/src/artifact.rs::TranscriptArtifact` ha già `revision`; `sqlite_artifact_repository.rs` conserva transcript cifrati e cancellazioni. Usare revisione e hash della fonte per scartare risultati obsoleti, insieme a versione del modello, prefissi e dimensione dei vettori. Una modifica del solo titolo non richiede ricodifica se la fonte dell'embedding resta identica; modifica del testo/timeline la richiede. Non serve un nuovo database vettoriale per il primo prototipo: indice in memoria e confronto locale bastano sul campione. Una cache persistente deve seguire la cifratura dell'archivio, essere ricostruibile e rispettare eliminazione, importazione e cambio workspace.
- **Download:** riutilizzare manifest, checksum, stati e gestione modelli di `commands/provisioning.rs` e `ModelManagerSheet`. È una capacità facoltativa, caricata quando richiesta; non entra nei modelli obbligatori di `initialSetup.ts` e non blocca la trascrizione. La chat oggi salva testo, senza citazioni strutturate: i risultati di ricerca proposti mostrerebbero estratti originali, ma citazioni cliccabili nella chat richiedono anche DTO, persistenza e UI dedicati.

### Peso e adattamento all'hardware

La configurazione iniziale proposta è **solo encoder testuale 270M, quantizzato, 256 dimensioni**, con LiteRT-LM e binding Rust all'API C fissata alla versione verificata. Il percorso senza Python ha funzionato nello smoke arm64; non è ancora integrato in Tauri. La [model card ufficiale](https://huggingface.co/google/embeddinggemma-2) documenta encoder separati e dimensioni riducibili: per queste funzioni non servono pesi audio o visione. Il supporto dichiarato a oltre 100 lingue resta una proprietà upstream, non una copertura dimostrata dai nostri test.

Sul Mac M3, la query CPU a caldo richiede circa 48 ms nel wrapper dello spike, abbastanza poco rispetto ai secondi della generazione. I pesi sono 164,6 MB; lo smoke nativo CPU registra 176,5 MB di footprint di processo. **Non sono il peso totale dell'app né la RAM end-to-end:** la dylib macOS universale estratta aggiunge 137,8 MB su disco, e il pacchetto finale per architettura deve essere misurato. GPU riduce il tempo di indicizzazione nel wrapper, ma il percorso nativo GPU resta da verificare.

Indicizzare dopo il salvataggio, soltanto le fonti nuove o modificate, con priorità inferiore all'ASR e cancellazione. Un solo engine quando necessario; scaricarlo quando inattivo o sotto pressione di memoria. CPU è il primo percorso da validare per contenere risorse; GPU è un'opzione da misurare per l'indicizzazione. Nessuna soglia automatica di RAM o promessa su vecchie CPU viene dedotta da un solo M3. La ricerca con modello assente deve restare utile; download e caricamento non devono essere richiesti per trascrivere.

Un indice float32 a 256 dimensioni richiede circa 10,24 MB per 10.000 passaggi, esclusi metadati e testo. È una stima del solo indice, non un test di scalabilità. Nessun motore generativo 4B sempre residente è necessario per la ricerca proposta; la chat continua a usare il provider scelto dall'utente.

### Potenzialità da tenere distinte

EmbeddingGemma 2 può aiutare a selezionare il materiale per domande, analisi e riassunti; il contenuto finale viene comunque prodotto da un altro modello o mostrato come estratto. Non corregge parole sentite male, non traduce e non produce da solo attività o decisioni. Riassunti e FAQ già generati non sono prove letterali primarie e devono restare distinti dal transcript.

Clustering, classificazione e ricerca audio sono possibilità future, non risultati di questo test. La documentazione distingue i prefissi per ricerca, domande, classificazione e clustering: non riutilizzare i vettori dello spike come se validassero tutti questi compiti. La ricerca semantica direttamente sull'audio richiederebbe encoder, pesi e corpus dedicati; anche un hit audio non dimostrerebbe una correzione letterale dell'ASR. Non aggiungere questi componenti al primo rilascio.

### Criteri per passare dal potenziale alla release

1. Riprodurre il recupero nativo sugli stessi testi, prefissi e dimensioni, controllando overflow e ranking CPU/GPU; verificare caricamento, cancellazione e chiusura senza Python.
2. Provare nell'app ricerca e citazioni dopo modifica, ottimizzazione, cancellazione, cambio workspace e importazione; mai mostrare una citazione da un indice obsoleto o da un documento escluso. Verificare ripetizioni, cifre e passaggi privi di timestamp/audio.
3. Verificare il contesto realmente inviato alla chat, inclusi tutti i budget: il supporto trovato deve arrivare al provider. Citazioni risolte dal testo salvato, non ricopiate dal generatore; dettaglio assente segnalato anche quando il tema è pertinente.
4. Misurare pacchetto firmato, avvio, memoria e latenza mentre l'ASR lavora, su archivi più grandi e Intel/Windows reali. Confermare il fallback quando modello o runtime non sono disponibili; estendere lingue e domande prima di dichiarare copertura generale.

**Decisione:** EmbeddingGemma 2 merita la prossima fase come ricerca locale facoltativa con estratti verificabili; l'arricchimento della chat segue la verifica del recupero nell'app. Non è dimostrato superiore in ogni metrica alla baseline Gemma 1 Q8, né rende automaticamente affidabile una risposta generata. Questo approfondimento definisce punti di inserimento e criteri di verifica: non introduce dipendenze, API o modifiche alla pipeline ASR.

Il confronto successivo sul [glossario per il correttore audio-aware](embedding-gemma-glossary-correction-2026-10-08.md) separa il recupero terminologico dalla correzione e include il controllo lessicale. È un esperimento distinto dal retrieval di questo report.

## Riproduzione e provenienza

Artefatti esterni: `/Volumes/UltraDisk/sbobino-embedding-spike-20261008/deepening`. `data/prepare_long_holdout.py` rigenera i corpus pubblici; `data/audit.md` conserva selezione, intervalli, etichette e hash. `protocol.json` registra le configurazioni predefinite e le correzioni pre-inferenza. `results/provenance.json` registra hash dei dati, modelli, harness, risultati e log; i vettori originali LiteRT sono conservati per il confronto con la verifica overflow.

Query freeze SHA-256: `d3151abea1c45165770fb4c963b344ac65d69ffb943a984ba2af5de34a23b56f`. Congelamento: `2026-10-08T06:01:01Z`. Modelli e versioni del primo report restano invariati; aggiunta [Gemma 1 QAT Q4](https://huggingface.co/ggml-org/embeddinggemma-300M-qat-q4_0-GGUF), revisione `8dd0ca2a66a8f14470acb0e2a71f801afbc5fb73`, SHA-256 `50d28e22432a148f6f8a86eab3700f92add5d1f54baf7790675a2a4dadbccf26`. Il generatore usa il GGUF Gemma 4 E4B Q4_0 già disponibile, 4.590.807.392 B, SHA-256 `a555b900214b477d8880e7832e0b8925e139b0159640036b09fe472b6f2097f2`. Nessun mmproj nel controllo generativo testuale.

Dalla radice della repository:

```sh
python3 sbobino_desktop/scripts/embedding_spike.py --self-test
python3 sbobino_desktop/scripts/embedding_qa_probe.py --self-test
SPIKE=/Volumes/UltraDisk/sbobino-embedding-spike-20261008/deepening
python3 sbobino_desktop/scripts/embedding_spike.py \
  --corpus "$SPIKE/data/corpus-parakeet.json" \
  --vectors "$SPIKE/results/g2-parakeet-vectors.json" \
  --dimension 256 --scope recording --output "$SPIKE/results/reproduced-score.json"
```

Il controllo generativo richiede il server locale Gemma 4 sulla porta 9382. `embedding_qa_probe.py` accetta i tre file di scoring con `--scores LABEL=METHOD=PATH`, salva atomicamente dopo ogni caso e può riprendere con `--resume` soltanto se configurazione e checksum coincidono. Ricontrolla i risultati salvati contro risposta grezza ed evidenza congelata. Il driver esterno `run_qa.py` registra avvio/fine e arresta il proprio server; il primo blocco interrotto senza checkpoint è escluso dai risultati finali e il suo log è conservato.

Lo smoke nativo ripetibile è `native/embedding_cabi_smoke.arm64 MODEL_PATH "domanda italiana"`; sorgente C++, header/framework fissati, binario e log sono nella stessa directory. Non è un artefatto di distribuzione dell'app.

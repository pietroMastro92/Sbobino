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

## Cosa conviene portare nella prossima fase

1. **Ricerca e consultazione prima della generazione.** Userei la ricerca lessicale come base senza pesi aggiuntivi e proverei EmbeddingGemma 2 come download facoltativo per parafrasi e query tra lingue. Può migliorare la scelta dei passaggi da fornire anche ai provider AI già disponibili, senza cambiare il transcript. I risultati devono mostrare il testo salvato e la finestra audio, non una citazione ricopiata dal generatore.
2. **Gemma 1 Q8 resta la baseline di qualità.** In questa configurazione vince sul primo risultato e sulla latenza delle query; Gemma 2 offre meno pesi, indicizzazione più rapida e meno footprint nel wrapper provato. Gemma 1 Q4 non mostra un vantaggio sufficiente rispetto a Q8: risparmia relativamente poco e perde precisione. Nessuna regola di scelta automatica per hardware è dedotta da un solo Mac.
3. **La chat richiede una verifica ulteriore.** Il ranking semantico non certifica che il passaggio contenga il dettaglio richiesto. La soglia cosine non risolve l'astensione, e il generatore può etichettare come risposta una citazione vera ma insufficiente. Non introdurrei il generatore 4B come componente sempre residente per questa funzione. Riassunti, decisioni e attività restano elaborati distinti dalla trascrizione letterale; non sono stati valutati da questo test.

Prima di integrare: un binding Rust minimo al runtime scelto, prova nativa completa con gli stessi documenti/prefissi, budget di contesto della chat, aggiornamento dell'indice dopo una modifica del testo, gestione della memoria/cancellazione durante ASR, firma e pacchetto distribuibile. Poi Intel e Windows reali, altre lingue e archivi più grandi. Il successo arm64 non dimostra queste proprietà. Python resta soltanto nello spike; nessuna nuova dipendenza dell'app, API pubblica o selezione dei motori ASR è cambiata.

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

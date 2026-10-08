# EmbeddingGemma: primo confronto locale per Sbobino

## Decisione

La ricerca semantica merita una prova nell'esperienza di consultazione, ma **questo test non giustifica scegliere Gemma 2 al posto di Gemma 1 né integrare subito un nuovo runtime**. Entrambi recuperano al primo posto tutti i 26 passaggi etichettati pertinenti, anche usando gli output reali di Parakeet. Gemma 1 è più veloce nella configurazione CPU provata; Gemma 2 ha download e footprint CPU inferiori e offre un percorso multimodale da valutare separatamente.

Il primo impiego consigliato è il recupero di passaggi per ricerca e chat, mantenendo citazioni e ricerca letterale. Non sono stati provati generazione di risposte, riassunti, correzione del transcript, classificazione, clustering o embedding audio. Il transcript non viene modificato. Nessuna API, dipendenza o pipeline dell'app è cambiata.

## Protocollo e limiti

- Hardware verificato: Apple M3, 16 GiB, macOS 27.0 ARM64. Nessuna misura Intel o Windows.
- Solo materiale pubblico: 200 clip FLEURS italiano test del precedente spike, deduplicate in 164 testi distinti. La selezione conserva gli ID delle clip duplicate.
- 30 query curate da un agente e congelate prima dell'inferenza: 10 parafrasi italiane, 6 termini/nomi/numeri, 6 query inglesi verso testi italiani, 4 query contenenti termini letterali e 4 senza risposta nel corpus.
- Un controllo separato, congelato prima dell'ispezione dei risultati dei modelli, contiene 8 query di 2–4 parole copiate letteralmente, ciascuna con un solo documento corrispondente. Le 30 domande naturali non sono query di sottostringa: il loro 0/26 non dimostra un difetto della ricerca letterale.
- Le etichette sono di **pertinenza del passaggio**, non una certificazione della disponibilità della risposta. La revisione indipendente ha trovato `q005` (perché non percepiamo il moto della Terra) e `q025` (definizione di "team invisibile") pertinenti al tema ma senza la spiegazione richiesta. Le etichette congelate non sono state cambiate dopo i risultati.
- Seconda variante: gli stessi documenti sono sostituiti dai transcript del run pubblico `fleurs-test200-parakeet-q8-resident.json`, scegliendo sempre la clip rappresentativa per ID. Stesse query e stesse etichette, checksum distinto. Non è una nuova trascrizione né un test su registrazioni lunghe.
- Baseline: sottostringa dello storico, port fedele di `artifacts.rs::score_chunk`, BM25 con parametri fissi e fusione RRF (`k=60`, pesi uguali). Il port della chat valuta la sua euristica su brani indipendenti: non esegue budget, riordino cronologico, selezione dentro una singola registrazione o generazione della risposta.
- Un modello residente alla volta. Quattro thread CPU. Una codifica dei documenti e tre ripetizioni delle query a caldo. Tutti i vettori sono finiti e le ripetizioni coincidono entro `1e-5`.
- Output richiesto: 768 dimensioni; valutazione anche a 128 e 256 dopo troncamento e nuova normalizzazione L2. La riduzione dell'indice non dimostra risparmi di tempo d'inferenza o RAM del modello.

È un test esplorativo piccolo, con argomenti riconoscibili e testi brevi. Non ha un holdout indipendente, distrattori difficili, riunioni lunghe, interrogazioni realistiche degli utenti o copertura di molte lingue. Le parafrasi e i sei casi inglese→italiano non provano qualità uniforme in oltre 100 lingue. I risultati perfetti ripetuti sono misure sullo stesso insieme di etichette.

## Recupero a 256 dimensioni

Hit@1/Hit@5 indica la presenza di almeno un documento etichettato pertinente nel primo risultato/nei primi cinque. Non indica che una risposta generata sarebbe corretta.

| Metodo | Testi di riferimento, Hit@1 / Hit@5 | Transcript Parakeet, Hit@1 / Hit@5 |
|---|---:|---:|
| Sottostringa, domande naturali | 0/26 / 0/26 | 0/26 / 0/26 |
| Euristica attuale della chat | 17/26 / 23/26 | 18/26 / 22/26 |
| BM25 | 21/26 / 22/26 | 21/26 / 22/26 |
| Gemma 1 Q8, llama.cpp CPU | 26/26 / 26/26 | 26/26 / 26/26 |
| Gemma 2 QAT, LiteRT CPU | 26/26 / 26/26 | 26/26 / 26/26 |
| Gemma 2 + BM25, RRF fisso | 22/26 / 23/26 | 23/26 / 23/26 |

Sul controllo letterale separato tutti i metodi recuperano 8/8 sui riferimenti. Sui transcript Parakeet la sottostringa scende a 7/8; BM25, euristica chat ed entrambi i modelli restano a 8/8. Pertanto quel recupero non è un beneficio esclusivo degli embedding.

Gemma 2 CPU con prefissi HF, prefissi LiteRT e senza prefissi raggiunge 26/26 anche a 128/768 dimensioni; Gemma 1 e Gemma 2 GPU hanno lo stesso esito sui riferimenti. Questo test non discrimina la qualità dei prefissi né delle dimensioni. Non è stato ottimizzato RRF dopo aver osservato il peggioramento.

Le quattro query negative ricevono comunque risultati semantici. Con Gemma 2 HF/256 i massimi cosine sono 0,607–0,687; con Gemma 1 0,270–0,408. Le scale non sono intercambiabili. Non è stata calibrata una soglia di astensione e non si deduce una soglia sicura da quattro esempi. I casi `q005`/`q025` mostrano inoltre che la pertinenza tematica non basta a verificare che il testo contenga una risposta.

## Prestazioni locali

Misure principali sui 164 riferimenti, prefissi HF, 90 richieste a caldo (30 query × 3). La latenza comprende codifica e validazione del vettore; per llama.cpp include il trasporto HTTP locale. Non comprende una chat generativa né la ricerca nell'archivio dell'app.

| Configurazione | Pesi su disco | Indicizzazione 164 testi | Query mediana / p95 | Picco footprint |
|---|---:|---:|---:|---:|
| Gemma 1 Q8, llama.cpp CPU, 1 slot | 333.590.944 B | 4,88 s | 12,7 / 19,4 ms | server 501.466.576 B; client Python separato |
| Gemma 2 QAT, LiteRT CPU | 164.626.432 B | 8,15 s | 45,9 / 57,1 ms | processo Python + runtime 191.562.712 B |
| Gemma 2 QAT, LiteRT GPU | 164.626.432 B | 4,28 s | 23,1 / 25,5 ms | processo Python + runtime 434.489.576 B |

I picchi RSS corrispondenti sono 762.920.960 B per il server Gemma 1, 350.715.904 B per Gemma 2 CPU e 415.875.072 B per Gemma 2 GPU. RSS e footprint sono metriche differenti: non vengono sommati, né viene aggiunta un'allocazione GPU alla memoria unificata. Il client del run Gemma 1 clean ha RSS 50.380.800 B e footprint 34.505.520 B secondo `/usr/bin/time`; la cifra del server non è l'intera pipeline. Il valore RSS registrato dal harness prima della chiusura del processo può essere inferiore al picco esterno finale. Configurazioni, quantizzazioni e wrapper diversi impediscono di attribuire tutta la differenza alla sola architettura del modello.

Gemma 1 usa `--parallel 1 --ctx-size 2048 --batch-size 512 --ubatch-size 512`, quattro thread e zero layer GPU. Il server registra l'ascolto circa 1,13 s dopo l'avvio. Il campo `load_seconds` del client HTTP non misura il caricamento del server e non viene usato come startup nativo.

Il primo smoke di Gemma 2 CPU ha richiesto 13,29 s di apertura e 11,01 s per la prima richiesta. Il secondo smoke con cache riusata ha aperto in 0,17 s. Nel run completo CPU apertura 0,46 s e warmup 0,34 s; GPU apertura 3,95 s e warmup 0,04 s. Il costo iniziale della cache è separato dalle latenze a caldo: non promettere avvio istantaneo. Non è stata misurata l'indicizzazione concorrente con Live/ASR, energia o stabilità su archivi grandi.

## Riproduzione e provenienza

Harness: `scripts/embedding_spike.py`. Controllo senza dipendenze: `python3 scripts/embedding_spike.py --self-test`. Il runner rifiuta ID/vettori non corrispondenti, checksum del corpus differente, vettori non finiti, nulli o troppo corti. Python e LiteRT restano nell'ambiente dello spike.

Artefatti: `/Volumes/UltraDisk/sbobino-embedding-spike-20261008/{data,models,runtime,results}`. Il manifest `results/provenance.json` registra hash di harness, corpus, vettori, punteggi e log. I dati congelati e i modelli rimangono esterni alla repository.

- Corpus principale: `de766542723ed8794de14e2431b5db23dceb3f71245e8060f4c55b600f57ced8`.
- Controlli letterali: `03ec68ed74867c01fb8fa1d580f0ab2b8c80c32a4f8dfd086395012a5ebc6ac5`.
- Gemma 2 [LiteRT text 270M](https://huggingface.co/litert-community/embeddinggemma-2-text-270m-litert-lm), revisione `9be6e8b90982095dc05c2bd162e4b954ee4dbac7`, SHA-256 `2d079ee2f6f066b1f368e8d7c819f55214eaef1d0513b312321901f30ab286fb`; Python 3.12.7, `litert-lm`/`litert-lm-api` 0.18.0.
- Gemma 1 [GGUF del progetto llama.cpp](https://huggingface.co/ggml-org/embeddinggemma-300M-GGUF), revisione `0f741b5a6585bd53aeb15cd1372c56f2a0f65e12`, SHA-256 `b5ce9d77a3fc4b3b39ccb5643c36777911cc4eb46a66962eadfa3f5f60490d63`; Homebrew llama.cpp 0.4.1, build 10964, commit `b29c606e2`. Il checkpoint Google di Gemma 1 richiede approvazione; il GGUF pubblico ha permesso il confronto senza autenticazione.

Esempio dalla directory `sbobino_desktop`, con `SPIKE=/Volumes/UltraDisk/sbobino-embedding-spike-20261008`:

```sh
"$SPIKE/runtime/.venv/bin/python" scripts/embedding_spike.py \
  --corpus "$SPIKE/data/corpus.json" --runtime litert --backend cpu \
  --model "$SPIKE/models/embeddinggemma-2-text-270m.litertlm" --prefix hf \
  --output "$SPIKE/results/reproduced-vectors.json"
python3 scripts/embedding_spike.py --corpus "$SPIKE/data/corpus.json" \
  --vectors "$SPIKE/results/reproduced-vectors.json" --dimension 256 \
  --output "$SPIKE/results/reproduced-scores.json"
```

I prefissi [Google/HF](https://huggingface.co/google/embeddinggemma-2) (`task: search result | query:` / `title: none | text:`) e quelli della [guida LiteRT](https://developers.google.com/edge/litert-lm/embedding_models) (`task: search query | text:` / `task: search result | text:`) differiscono. L'API installata passa il testo senza prefissi automatici; per questo sono state conservate le tre ablation esplicite, senza eleggerne una tramite questo test piccolo.

Prima di una decisione d'integrazione serve un holdout pubblico con registrazioni lunghe, distrattori simili, errori ASR su entità/numeri, domande pertinenti ma non rispondibili e più coppie linguistiche. Deve misurare anche citazioni corrette e astensione nella chat. Restano da verificare binding C++/Rust, artefatti distribuibili e Intel/Windows reali: il successo del wrapper Python non certifica il packaging nativo. Gemma 2 è un candidato leggero valido, non un aggiornamento automaticamente migliore: Gemma 1 resta un confronto obbligatorio e BM25 un'alternativa utile senza pesi aggiuntivi.

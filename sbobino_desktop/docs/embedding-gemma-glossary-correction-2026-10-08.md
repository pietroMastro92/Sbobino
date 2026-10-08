# EmbeddingGemma 2: glossario per il correttore audio-aware

## Esito del confronto sul parlato

**Non è dimostrato un beneficio del glossario rispetto al transcript ASR originale in questo pilot.** Su Whisper il recupero embedding migliora leggermente il correttore senza riferimenti e BM25; su Parakeet peggiora il punteggio. Il vantaggio sui nomi/termini annotati è nullo. Questi risultati non autorizzano la correzione automatica nell'app e non escludono benefici con documenti specialistici più pertinenti.

96/96 tentativi riusciti, zero errori e troncamenti. Ogni colonna corretta riguarda 16 finestre e 886 parole dei segmenti interni; valori bassi sono migliori:

| Sorgente ASR | Originale | Correttore solo audio | + BM25 | + EmbeddingGemma 2 |
|---|---:|---:|---:|---:|
| Parakeet Q8: errori interni | 81 (9,14%) | 81 (9,14%) | 83 (9,37%) | 83 (9,37%) |
| Whisper Q8: errori interni | 76 (8,58%) | 80 (9,03%) | 78 (8,80%) | 77 (8,69%) |

Su Parakeet EmbeddingGemma registra una finestra migliorata, due peggiorate e tredici neutre rispetto all'ASR. Su Whisper quattro migliorate, due peggiorate e dieci neutre. Sono classi **del punteggio automatico sui sottotitoli**, non modifiche già certificate dall'ascolto. Tutti i bracci corretti mantengono 8/9 entità annotate corrette per sorgente; anche gli originali hanno 8/9. Il controllo dei numeri nell'intera finestra resta invariato: 7 errori Parakeet e 6 Whisper, comprendendo le discrepanze di bordo.

Il WER dell'intera finestra racconta una storia diversa per Whisper: originale 215 errori, audio-only 219, BM25 214, embedding 213, su 1128 parole. I segmenti intersecati oltrepassano l'audio da 0,21 a 6,386 s (mediana 3,06 s): non usare questa apparente riduzione di due errori come prova di vantaggio letterale. La misura interna è quella primaria, con i limiti di allineamento descritti sotto.

Casi consultabili in `results/score.json`, con prima/dopo e finestra audio:

- **Industria, 26–54 s, Whisper:** `tessire` diventa `tessile` in tutti e tre i bracci. Un errore viene recuperato nel riferimento interno, ma il glossario non aggiunge beneficio.
- **Blockchain, 312–340 s, Whisper:** `appassionati` diventa `appassionate` nei due bracci con riferimenti, riducendo un errore del punteggio. La correzione non richiede un termine tecnico e non dimostra un contributo specifico degli embedding.
- **Blockchain, 884–912 s, Parakeet:** i due bracci con glossario aggiungono `in questo`, con due errori interni aggiuntivi; il correttore senza glossario conserva il testo. È una modifica da verificare nell'audio, non un termine recuperato.
- **Ottimismo, 26–54 s, Whisper:** tutti reintroducono la ripetizione `non ho avuto` e altre parole, con quattro errori interni aggiuntivi. I sottotitoli possono omettere ripetizioni autentiche: questo caso resta esplicitamente non classificato acusticamente, anche se peggiora il punteggio.

Mediane per chunk di 28 s: Parakeet audio-only 18,50 s, BM25 18,10 s, embedding 20,91 s; Whisper 17,38 / 15,32 / 19,87 s. Le sole 16 correzioni embedding impiegano 321,88 s su Parakeet e 305,30 s su Whisper, prima di aggiungere ASR e recupero. Sono una ripetizione residente, con cache e ordine ruotato; non rappresentano un benchmark della pipeline integrata o un confronto definitivo di velocità tra metodi.

Recupero CPU: 86,71 s per caricamento, codifica di 188 blocchi e 32 query; le query sommano 11,91 s, mediana 330 ms. Questo corpus e questo run non riproducono i 48 ms misurati sulle query brevi del test precedente: qui la query è il transcript di un chunk, con condizioni di sistema diverse. L'indice documentale può essere riutilizzato; non attribuire l'intero tempo a ogni correzione.

Il server correttore avvia in 12,72 s; il run principale dura 1781,70 s. Picco RSS di processo 4497,9 MB, senza sommare presunte allocazioni GPU alla memoria unificata. È il high-water del processo llama.cpp, non il footprint fisico o la memoria della pipeline completa. Embedding e correttore sono caricati in sequenza. Nessun confronto Intel, Windows o MLX in questo pilot.

## Controllo separato sul silenzio

Dopo il confronto sul parlato sono stati eseguiti sei tentativi sul controllo pubblico già presente nello spike: 28 s PCM16 con tutti i campioni a zero, SHA-256 `ab513bd9258f62d957ae2351f15e8c2cae5c1ca257ad3f026f818040821da945`. Transcript sorgenti realmente prodotti in precedenza: Parakeet vuoto; Whisper «Autore dei sottotitoli e revisione a cura di QTSS». Stesso corpus documentale, top-3 e parametri del correttore; due query aggiuntive, congelate separatamente. Non si sommano questi casi ai risultati mTEDx.

| Sorgente | Solo audio | + BM25 | + EmbeddingGemma 2 |
|---|---|---|---|
| Parakeet vuoto | Due virgolette: zero parole, artefatto di formato | Eco delle istruzioni di glossario: 64 parole | Eco delle istruzioni e materiale del glossario: 308 parole, troncato |
| Whisper, frase spuriosa | Conserva la frase, sostituendo `revisione` con `revisionato` | Conserva la frase, sostituendo `revisione` con `revisioni` | Stesso esito BM25 |

Cinque output su sei contengono parole non supportate dall'audio; uno di questi raggiunge il limite di generazione. Sono cinque tentativi con stato `ok` e uno `truncated`, non sei correzioni fedeli. Un riferimento vuoto non ha WER definito: si contano le parole emesse e si conservano risposte grezze, usage e motivo di arresto in `results/silence-corrections.json` e `silence-score.json`.

BM25 sulla query Parakeet vuota non recupera estratti: l'eco avviene già con le istruzioni aggiuntive e una lista vuota. Il problema non è quindi attribuibile esclusivamente agli embedding o ai termini recuperati; riguarda il correttore generativo, il prompt e il trattamento del silenzio. Lo scenario è diverso dal primo test ASR diretto, ma conferma che le istruzioni rigide da sole non garantiscono fedeltà. I delimitatori riducono la confusione dei dati, senza renderla impossibile.

## Decisione per l'app

**Non integrare questo correttore con glossario come passaggio automatico.** Nel piccolo campione non supera l'ASR originale, non recupera entità annotate aggiuntive e aggiunge decine di secondi per finestra; sul silenzio viola il contratto di fedeltà. Il vantaggio di tre errori rispetto al correttore audio-only su Whisper non giustifica distribuzione o uso generalizzato.

Resta sensato usare EmbeddingGemma 2 per recuperare riferimenti terminologici pertinenti e mostrarli durante la revisione, insieme al passaggio originale e all'audio. Questo utilizzo evita una riscrittura generativa automatica; la sua utilità come interfaccia di revisione non è stata misurata dal presente test. La ricerca locale e il recupero per la chat conservano l'evidenza del precedente benchmark.

Un successivo esperimento sulla correzione dovrebbe usare fonti pubbliche realmente associate alla lezione, con copertura terminologica verificata su sviluppo e selezione congelata su registrazioni nuove. Deve includere nomi/termini assenti dal documento, silenzio e audio ambiguo, revisione acustica delle modifiche e costo end-to-end. Soglie cosine, controllo ortografico o un filtro sul silenzio da soli non dimostrano la sicurezza delle correzioni; nessuna di queste protezioni è qui integrata o validata nell'app.

## Domanda e protocollo

Questo esperimento verifica se recuperare riferimenti terminologici esterni aiuta Gemma 4 E4B a correggere una trascrizione ascoltando il relativo audio. EmbeddingGemma 2 sceglie i riferimenti: non trascrive e non genera la correzione. È uno spike isolato, senza integrazione nell'app.

Tre bracci sullo stesso audio e transcript:

1. Gemma 4 audio-aware senza riferimenti.
2. Gemma 4 audio-aware con riferimenti recuperati tramite BM25.
3. Gemma 4 audio-aware con riferimenti recuperati tramite EmbeddingGemma 2.

Il confronto con BM25 distingue il beneficio del modello embedding dal semplice accesso ai documenti. Il transcript ASR originale è un'ulteriore baseline di qualità. Non si modificano prompt o parametri dopo aver visto i risultati.

## Dati e separazione delle fonti

- Quattro talk pubblici italiani mTEDx già presenti nello spike; quattro finestre complete da 28 s per talk. Selezione equidistante tra primo chunk dopo i crediti e ultimo chunk completo, senza consultare errori o trascrizioni manuali per scegliere le finestre. Intervalli non consecutivi: questo test non misura ricomposizione o stabilità su file lunghi.
- 16 finestre audio uniche, 448 s complessivi; per ciascuna, transcript Parakeet TDT v3 Q8 e Whisper large-v3-turbo Q8 già prodotti. 32 casi × 3 bracci = 96 tentativi di correzione.
- Un corpus misto condiviso di 188 blocchi da massimo 100 parole, senza overlap. Per ogni transcript, recupero top-3 sull'intero corpus, senza filtro per talk o soglia adattata al test.
- Fonti indipendenti: [glossario A–Z EUBOF, pagine 5–6](https://blockchain-observatory.ec.europa.eu/document/download/1063effa-59cc-4df4-aeee-d2cf94f69178_en?filename=Blockchain_For_Beginners_A_EUBOF_Guide.pdf), [leaflet manifattura e robotica, entrambe le pagine](https://monitor-industrial-ecosystems.ec.europa.eu/sites/default/files/2021-09/Leaflet%20Advanced%20Manufacturing%20and%20Robotics%20for%20ICT%20Manufacturing.pdf), [glossario ESA di osservazione terrestre](https://www.esa.int/Applications/Observing_the_Earth/Earth_observation_glossary), [voce NHGRI sul DNA](https://www.genome.gov/genetics-glossary/Deoxyribonucleic-Acid-DNA). Snapshot originali, estrazione, URL e checksum conservati. Non sono slide del talk né glossari costruiti con termini prelevati dal riferimento.
- Fonti prevalentemente inglesi e copertura imperfetta: osservazione terrestre non copre tutte le tecnologie spaziali; una voce sul DNA non copre tutto il talk sull'ottimismo. Gli estratti possono essere irrilevanti. La pertinenza è una proprietà da valutare, non una garanzia del protocollo.
- Selezione e riferimenti recuperati congelati prima della correzione: `cases.json` SHA-256 `cd9d7e3ee44ed4196fe382797ec7363a7829bcf3706c2e47d81d9d38620034f6`. Il processo di inferenza rifiuta campi contenenti reference/gold/expected e non legge il file del valutatore.

La verifica delle entità nei soli segmenti interni conta 18 opportunità caso/variante. I top-3 EmbeddingGemma contengono 4 occorrenze, tutte di `blockchain`; BM25 contiene 4 occorrenze di `blockchain` o `DNA`. Questo è un limite forte del corpus documentale e del recupero: il test misura soprattutto contesto generico e non un glossario ricco di candidati per nomi propri o parole rare. I conteggi sono in `results/glossary-coverage.json`; non si aggiungono termini dopo aver osservato il test.

## Runtime e controlli

EmbeddingGemma 2 text 270M LiteRT-LM 0.18.0, CPU quattro thread, prefissi HF search, 768 dimensioni ridotte e rinormalizzate a 256 per il ranking. Input oltre limite trattati come errore. BM25 riutilizza il valutatore esistente. Indicizzazione e query terminano prima del caricamento del correttore; non si sommano modelli residenti.

Correttore Gemma 4 E4B Q4_0, proiettore BF16, llama.cpp Homebrew 0.4.1 build 10964 `b29c606e2`, Metal, contesto 8192, un solo slot, thinking disattivato, temperatura 0, top-k 1, massimo 512 token. Contesti di conversazione indipendenti, ordine dei tre bracci ruotato per caso. La cache del runtime può comunque influenzare le latenze; una sola ripetizione non basta per confronti prestazionali definitivi.

Il prompt conserva le istruzioni rigide del primo spike. Gli estratti sono dati JSON delimitati, esplicitamente non autorevoli e non necessariamente pronunciati: una modifica è ammessa soltanto se supportata dall'audio, altrimenti il testo va conservato. Queste istruzioni non provano che il modello le rispetti.

Una verifica del prompt ha inoltre rimosso i valori cosine dagli estratti inviati al correttore: ID, testo e URL sono gli unici campi comuni ai due bracci con glossario. Il run preliminare che includeva il punteggio è archiviato ed escluso.

Un primo avvio con il default di quattro slot ha prodotto forte pressione di memoria ed è stato interrotto prima di ottenere una risposta riuscita; checkpoint e log sono archiviati e separati dal run principale. Il harness ora passa esplicitamente un solo slot attraverso il runner esistente. La configurazione a quattro slot non contribuisce ai risultati finali.

Il harness salva atomicamente ogni tentativo, inclusi errori e troncamenti. Un’interruzione lascia il braccio pendente e ritentabile. Resume vincolato a checksum di input, modelli, binario e sorgenti; verifica identità, prompt e coerenza della risposta salvata. Un run completo indica tutti i tentativi registrati, non necessariamente riusciti.

## Valutazione e limiti

Il valutatore separato riutilizza normalizzazione e WER/CER del primo spike, preservando le cifre. Confronta errori, opportunità di nomi/termini già annotati, numeri, finestre migliorate/peggiorate e modifiche lessicali; conserva testo prima/dopo e intervallo audio per la revisione. Errori e troncamenti sono conteggiati; nel punteggio con fallback simulato si conserva il transcript ASR originale.

Le trascrizioni manuali sono segmentate e vengono selezionate per intersezione con la finestra audio: possono includere parole oltre i confini. Il WER dell’intera finestra comprende questa discrepanza. La misura primaria allinea l’output con tutti i segmenti intersecati, ma conta sostituzioni e omissioni solo nei segmenti interamente contenuti nella finestra; le inserzioni contano solo tra due parole di segmenti contenuti. Le opportunità di nomi/termini sono limitate agli stessi segmenti interni. Due self-check verificano esclusione dei bordi e conservazione delle cifre. Questo riduce il problema dei confini, ma l’allineamento resta basato sui sottotitoli: anche il delta va verificato sui casi modificati. Una finestra con meno errori non dimostra che ogni sua modifica sia corretta. Non è una stima di precisione delle singole correzioni né una revisione acustica completata.

## Riproduzione

Artefatti: `/Volumes/UltraDisk/sbobino-embedding-spike-20261008/glossary`. `prepare.py` congela snapshot, blocchi, vettori e ranking senza leggere i riferimenti manuali; `score.py` è il valutatore separato. Python e librerie PDF/HTML restano nell'ambiente esterno dello spike.

Dalla radice della repository:

```sh
python3 sbobino_desktop/scripts/embedding_glossary_probe.py --self-test
SPIKE=/Volumes/UltraDisk/sbobino-embedding-spike-20261008/glossary
ASR=/Volumes/UltraDisk/sbobino-asr-spike-20260927
TMPDIR=/Volumes/UltraDisk/sbobino-tmp python3 sbobino_desktop/scripts/embedding_glossary_probe.py \
  --cases "$SPIKE/data/cases.json" --out "$SPIKE/results/corrections.json" \
  --binary /opt/homebrew/bin/llama-server \
  --model "$ASR/models/gemma-4-e4b-gguf/gemma-4-E4B-it-Q4_0.gguf" \
  --mmproj "$ASR/models/gemma-4-e4b-gguf/mmproj-gemma-4-E4B-it-BF16.gguf"
```

Il controllo usa `prepare_silence.py`, `data/silence-cases.json` e un run separato `results/silence-corrections.json`; `finish.py` esegue scoring e controlli solo dopo la chiusura del server principale. Il manifest `results/provenance.json` registra checksum di snapshot, dati, driver, harness e risultati, oltre ai tre modelli e al binario runtime.

Un risultato esistente richiede `--resume`; per una ripetizione indipendente usare un altro `--out`. Non rigenerare `cases.json` prima di riprendere un checkpoint congelato.

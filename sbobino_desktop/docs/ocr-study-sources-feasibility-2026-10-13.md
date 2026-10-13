# OCR documentale per la dispensa multifonte

Valutazione delle fonti collegate al post di Alex del 13 ottobre 2026 e della possibile integrazione di Unlimited-OCR nel flusso di studio di Sbobino. Questa nota riguarda l'acquisizione e la strutturazione di documenti; non dimostra ancora qualità su documenti italiani. Nessuna modifica alla pipeline dell'app in questa fase.

## Identità del modello

Il post di Alex non presenta un modello Gemma. Il testo del post dichiara che una versione aggiornata legge lo stesso paper di 14 pagine in 53 secondi invece di circa 70, a circa 263 token/s invece di circa 200, e mostra equazioni e tabelle durante la lettura. Il link abbreviato principale del post risolve a [FluidInference/unlimited-ocr-mlx-coreml](https://huggingface.co/FluidInference/unlimited-ocr-mlx-coreml); il secondo rimanda al video dimostrativo di [Brooks Whale X](https://x.com/BrooksWhaleX/status/2109080341642363349). La fonte primaria del post è l'[oEmbed ufficiale di X](https://publish.twitter.com/oembed?url=https%3A%2F%2Fx.com%2FAlex_tra_memory%2Fstatus%2F2109873622525358533) e il post originale è [qui](https://x.com/Alex_tra_memory/status/2109873622525358533).

Il modello è la conversione Apple Silicon di [Baidu Unlimited-OCR](https://huggingface.co/baidu/Unlimited-OCR), basato sull'architettura DeepSeek-OCR: encoder di pagina SAM-B + CLIP-L, decoder DeepSeekV2 MoE da circa 3B parametri con circa 0,5B attivi. Il paper ufficiale [Unlimited OCR Works](https://arxiv.org/abs/2606.23050) descrive Reference Sliding Window Attention (R-SWA), che mantiene tutti i token visivi e il prompt come contesto persistente e limita la finestra dei token già generati. FluidInference cambia formato e precisione per Apple; il [model card MLX/Core ML](https://huggingface.co/FluidInference/unlimited-ocr-mlx-coreml) dichiara che i pesi derivano dal modello Baidu e la conversione è MIT.

Non è EmbeddingGemma 2, non è un ASR e non è un modello generativo pensato per comporre la dispensa. È un VLM OCR/document parser: immagine o pagina PDF in ingresso, Markdown strutturato in uscita.

## Che cosa è documentato

Il model card FluidInference documenta tre varianti del decoder che condividono l'encoder Core ML:

- `mlx/decoder.safetensors`: decoder MLX a 8 bit;
- `mlx-turbo/decoder.safetensors`: esperti a 4 bit per blocchi di 32, resto a 8 bit, presentato come variante più veloce;
- `UnlimitedOcrDecoder.mlpackage`: decoder Core ML a 8 bit con cache K/V stateful.

Le dimensioni dichiarate dall'API del repository alla revisione `4568571eacd864e0a253c609b3c9d0e00450cebc` sono:

| Componente | byte | ruolo |
| --- | ---: | --- |
| `UnlimitedOcrPageEncoder.mlpackage/.../weight.bin` | 806.020.480 | encoder pagina fp16 |
| `mlx-turbo/decoder.safetensors` | 2.059.359.575 | decoder MLX turbo |
| `mlx/decoder.safetensors` | 3.119.076.803 | decoder MLX 8 bit |
| `UnlimitedOcrDecoder.mlpackage/.../weight.bin` | 2.942.897.664 | decoder Core ML 8 bit |
| `embeddings.f16` | 330.956.800 | tabella usata dal decoder Core ML |
| `tokenizer.json` | 9.979.544 | tokenizer |

Per il runtime MLX turbo servono almeno encoder, turbo decoder, configurazione, tokenizer e manifest: circa 2,88 GB decimali di pesi/file principali, prima di cache compilate e memoria di esecuzione. Il repository pubblica anche le varianti 8 bit e Core ML, perciò il suo storage complessivo non equivale al download minimo di un singolo runtime.

Il [codice Swift del branch `feat/unlimited-ocr` di FluidUse](https://github.com/FluidInference/FluidUse/tree/cfb5d7c9f50ca51e10a14b8ebc186040fb8b2b2d) aggiunge una destinazione `UnlimitedOcrMLX` che usa `mlx-swift`/`mlx-swift-lm`; il suo [manager MLX](https://raw.githubusercontent.com/FluidInference/FluidUse/cfb5d7c9f50ca51e10a14b8ebc186040fb8b2b2d/Sources/FluidUse/UnlimitedOcrMLX/UnlimitedOcrFastManager.swift) esegue l'encoder con Core ML e il decoder con MLX. Il manager ammette fino a 16 pagine in una generazione e può scegliere il turbo decoder. Il [main attuale di FluidUse](https://github.com/FluidInference/FluidUse/tree/main) espone invece il solo manager Core ML, con bucket fino a 8 pagine; il supporto MLX non va quindi considerato già disponibile come dipendenza stabile. Il branch non coincide con `main` e il suo `UnlimitedOcrModelStore` pinna una revisione precedente (`9d5f274c...`), mentre il model card corrente è alla revisione sopra indicata: prima di usare il codice occorre aggiornare e ricontrollare tutti gli SHA256.

Il [parser Swift del risultato](https://raw.githubusercontent.com/FluidInference/FluidUse/cfb5d7c9f50ca51e10a14b8ebc186040fb8b2b2d/Sources/FluidUse/UnlimitedOcr/UnlimitedOcrDocument.swift) conserva pagine, blocchi, etichette e rettangoli. L'output grezzo usa sezioni `<PAGE>` e blocchi `<|det|>label [x1, y1, x2, y2]<|/det|>content`, con coordinate di pagina 0–999; il parser le normalizza a rettangoli 0–1 e può produrre Markdown senza la marcatura di layout. Questo è utile per citare pagina e regione, ma non crea automaticamente citazioni bibliografiche né verifica che il contenuto sia corretto. L'encoder prepara la pagina in un canvas quadrato 1024 × 1024 con aspect-fit: per aprire la regione originale bisogna conservare il rapporto di scala e l'offset di padding e fare l'inversa della trasformazione; il rettangolo normalizzato non basta da solo. Gli eventi e le statistiche non espongono una confidence per blocco e una fine senza EOS va trattata come output potenzialmente troncato, non come documento completo.

Il model card dichiara:

- fino a 16 pagine in un prompt per il runtime MLX, con un'unica generazione e separatori `<PAGE>`;
- encoder di pagina a 1024 × 1024 e circa 273 token visivi per pagina;
- supporto a testo, layout, tabelle ed equazioni tramite Markdown/markup generato;
- offline su Apple Silicon;
- sul test dell'autore, circa 100 ms per pagina di encoding, 19–23 ms per token di decoding e circa 160 s per 6.727 token/8 pagine su M5 Pro 24 GB.

La pagina del modello segnala anche che l'encoder deve girare sulla GPU: sulla Neural Engine l'output risulta errato, con cosine similarity delle righe fino a 0,66. Quindi la presenza di Core ML non significa che l'intero flusso sfrutti correttamente la Neural Engine.

## Qualità, multilinguismo e limiti delle prove

Il paper Baidu riporta risultati su OmniDocBench, che valuta testo, formule, struttura delle tabelle e ordine di lettura. Sono risultati del modello Baidu/asset di riferimento, non una misura locale di `mlx-turbo` su documenti italiani. La parità di pochi output con il riferimento fp32 misura la conversione, non la correttezza rispetto alle pagine originali.

Il model card Baidu contiene il tag generico `multilingual`, ma non pubblica un elenco verificabile di lingue, una riga italiana, né una valutazione specifica sull'italiano. Non abbiamo quindi evidenza primaria sufficiente per dichiarare che il modello sia affidabile per dispense italiane, termini tecnici italiani, nomi propri o materiale bilingue. La copertura multilingue va misurata nel nostro corpus.

Il paper valuta document parsing; non documenta in modo separato la scrittura a mano, fotografie di appunti, scansioni inclinate, immagini sfocate o formule manoscritte. Le tabelle e le equazioni del post sono una prova dimostrativa di resa, non una misura di fedeltà. Il modello è generativo: anche un output Markdown ben formattato può sostituire una parola, inventare una cifra o alterare una formula. Il file immagine originale e l'output grezzo devono rimanere disponibili per la verifica.

La quantizzazione turbo richiede ulteriore cautela. La documentazione della conversione descrive l'8 bit come riferimento: una pagina e tre pagine risultano identiche al riferimento fp32 nel test pubblicato. Lo stesso documento segnala che il decoder 4 bit precedente, non distribuito, divergeva già al token 329; la revisione corrente distribuisce invece `mlx-turbo` e dichiara differenze contenute, entro circa un punto sul testo lungo, con possibili differenze piccole in formule e tabelle. Questo non equivale a parità dimostrata su italiano.

## Utilità per la dispensa multifonte

Il modello è interessante come primo livello di ingestione documentale, accanto a EmbeddingGemma 2:

1. Si conserva sempre il PDF originale, il suo hash e la versione della fonte.
2. Per un PDF con testo nativo si prova prima l'estrazione testuale deterministica. L'OCR si attiva per scansioni, pagine immagine o documenti in cui servono layout, formule e tabelle.
3. Unlimited-OCR produce un artefatto strutturato per pagina/blocco: testo, tipo di blocco, coordinate e Markdown grezzo. L'app conserva anche le immagini delle pagine e l'output non ripulito.
4. I blocchi vengono indicizzati e recuperati con EmbeddingGemma 2 (o con ricerca lessicale se il modello non è presente). EmbeddingGemma resta il componente di pertinenza semantica; Unlimited-OCR non decide quali fonti siano autorevoli e non scrive la sintesi finale.
5. Il provider compositore riceve soltanto i blocchi selezionati e le loro citazioni di pagina/regione. La spiegazione del docente, l'integrazione esterna, le formule e le parti non lette restano distinguibili secondo [il disegno multifonte già approvato](./studio-multifonte-design.md).
6. Il testo OCR con segnali incerti, le formule e le tabelle vengono marcati per revisione. Nessuna correzione silenziosa del transcript o della fonte deve diventare testo canonico.

Il valore principale per gli studenti sarebbe quindi rendere ricercabili e citabili dispense scansionate, paper scientifici, slide e tabelle; il modello non sostituisce la verifica delle affermazioni né il compositore della dispensa. I rettangoli di layout permettono, in prospettiva, di aprire la pagina nel punto esatto da cui deriva un passaggio.

## Spike necessario prima dell'integrazione

Sul Mac M3 16 GB bisogna riprodurre il caso del post con lo stesso documento, poi estendere il corpus a:

- dispense e paper italiani con testo stampato, termini tecnici, nomi e numeri;
- pagine scansionate, colonne, note a piè di pagina, figure con didascalie;
- almeno una serie di formule e tabelle con riferimento verificabile;
- documenti bilingui e pagine in più lingue, senza assumere il tag `multilingual` come prova;
- 1, 5, 14, 16 e più di 16 pagine, misurando la ricomposizione tra chiamate;
- materiale manoscritto e immagini imperfette, riportando esplicitamente i casi non supportati.

Confrontare estrazione PDF nativa, un OCR di riferimento già disponibile, Unlimited-OCR 8 bit/Core ML e `mlx-turbo`. Registrare CER/WER sul testo, edit distance delle formule, TEDS/TEDS-S delle tabelle, ordine di lettura, accuratezza dei bbox/citazioni, inserzioni non supportate, tempo per pagina, tempo totale e picco di memoria. Fare almeno tre letture dello stesso documento dopo warm-up e conservare output grezzi, file e revisioni.

La decisione di integrazione dovrebbe richiedere qualità italiana non peggiore del fallback, nessuna alterazione non segnalata di numeri/formule, citazioni riconducibili a pagina e regione, memoria compatibile con 16 GB e una strategia x86/Windows separata. Il runtime MLX/Core ML può essere un acceleratore Apple opzionale scaricato su richiesta; non va inserito come dipendenza obbligatoria dell'app multipiattaforma e non risolve il percorso x86.

## Raccomandazione per Sbobino e riscontro nel codice

Valutare Unlimited-OCR nello spike documentale come parser facoltativo per Apple Silicon. Il [disegno del 12 ottobre](./studio-multifonte-design.md) parte da PDF testuali e trascrizioni audio/video, segnalando gli elementi visivi non analizzati. L'OCR sarebbe un'estensione successiva di quella capacità: può recuperare una formula stampata o le celle di una tabella, ma non dimostra che un grafico o una dimostrazione visiva siano stati interpretati integralmente. La trascrizione del docente conserva il proprio ruolo di fonte; l'eventuale spiegazione tratta da un libro rimane un'integrazione attribuita.

Esempio di prodotto: la lezione menziona una formula, e lo studente importa una dispensa scansionata. Il parser estrae formula e spiegazione dalla pagina; EmbeddingGemma recupera quei blocchi rispetto al passaggio della lezione; il compositore propone l'integrazione con una citazione che apre la pagina originale. La resa OCR della formula resta verificabile sull'immagine. Il compositore non deve trasformare un segno matematico incerto in un fatto certo.

Percorso proposto, da confrontare prima dell'integrazione:

1. **PDF con testo leggibile:** estrazione del testo nativo per pagina; usare l'OCR soltanto dove il testo o la struttura risultano insufficienti. Su Mac, [PDFKit espone il testo della pagina](https://developer.apple.com/documentation/pdfkit/pdfpage/string).
2. **Scansioni di testo ordinario:** confrontare un OCR leggero della piattaforma; [Vision offre riconoscimento multilingue locale](https://developer.apple.com/documentation/vision/recognizing-text-in-images). Le lingue disponibili e la qualità vanno verificate sul sistema reale. Non presume una qualità equivalente su formule/tabelle.
3. **Pagine complesse:** modello documentale facoltativo scaricato su richiesta, con batch che coprano tutte le pagine, memoria misurata e caricamento limitato all'importazione. Evitare di mantenere contemporaneamente OCR, ASR e embedding residenti senza misurare il picco.
4. **Citazioni e copertura:** conservare hash/versione della fonte, indice della pagina originale, blocchi, output grezzo, motore/revisione/configurazione e trasformazione immagine. I numeri di pagina e i box generati devono essere validati; il conteggio dei `<PAGE>` non garantisce che tutte le pagine siano state lette. Se manca una pagina o l'output termina per limite, il risultato rimane parziale e la dispensa conserva questa lacuna.

Il coordinatore ha ispezionato FluidUse `main` al commit [`d278e17df85e39e8c21bb64a467da81c39576fcd`](https://github.com/FluidInference/FluidUse/tree/d278e17df85e39e8c21bb64a467da81c39576fcd). Il [manifest](https://github.com/FluidInference/FluidUse/blob/d278e17df85e39e8c21bb64a467da81c39576fcd/Package.swift) non contiene il target MLX del post: il percorso presente è `UnlimitedOcrManager` CoreML. Il [demo](https://github.com/FluidInference/FluidUse/blob/d278e17df85e39e8c21bb64a467da81c39576fcd/Sources/UnlimitedOcrDemo/ReaderModel.swift) usa `min(pdf.pageCount, maxPages)`; importarlo direttamente non coprirebbe un documento oltre il limite del batch. Il [model store](https://github.com/FluidInference/FluidUse/blob/d278e17df85e39e8c21bb64a467da81c39576fcd/Sources/FluidUse/UnlimitedOcr/UnlimitedOcrModelStore.swift) pinna `FluidInference/unlimited-ocr-coreml` alla revisione `51046f081ace85b77c8287ad7b56d504b91eb3d2`, diversa dal pacchetto MLX/Turbo del post. Il branch `feat/unlimited-ocr` osservato separatamente punta a `cfb5d7c9f50ca51e10a14b8ebc186040fb8b2b2d`; va congelato insieme ai pesi per riprodurre il demo.

La ricerca nei sorgenti Rust/React di Sbobino ha trovato l'esportazione PDF, non un'importazione OCR documentale già pronta. Una futura integrazione richiede quindi un percorso di acquisizione e provenienza, oltre al modello. Il ramo Apple non risolve da solo il runtime nativo per Intel/Windows/Linux. La scelta finale dipende dalla qualità per lingua, dai documenti scientifici e dal benchmark sul Mac M3 16 GB.

Snapshot dei metadata e checkout di sola lettura conservati in `/Volumes/UltraDisk/sbobino-ocr-audit-20261013/`. Nessun modello grande scaricato o eseguito, nessuna prestazione locale misurata e nessuna integrazione nell'app durante questa ricognizione.

# EG2-Transcribe: verifica preliminare per Sbobino

## Esito

L'annuncio riguarda un adattamento ASR di EmbeddingGemma 2, non l'API embedding standard già provata nello spike. È un candidato interessante per Apple Silicon, ma la configurazione pubblica presenta limiti concreti del vocabolario italiano e non dimostra qualità multipiattaforma o adatta alle lezioni. Nessuna inferenza, installazione dell'app o modifica della pipeline Sbobino eseguita in questa verifica.

## Fonti e metodo

- [Annuncio dell'autore](https://x.com/TslShahir/status/2109660368381395414) e [descrizione del reader](https://x.com/TslShahir/status/2109660369429954631), recuperati anche tramite l'endpoint pubblico oEmbed di X; i testi oEmbed sono troncati, gli screenshot forniti dall'utente conservano ulteriori dichiarazioni.
- [DMG 1.6](https://haku-models.tslfiles.org/eg2demo/EG2-Transcribe-1.6.dmg), 2.526.791 byte, SHA-256 `4ee020455bf3cd6d3184b6f8271af060cd37be0bacc3669cca9739d2a7805214`.
- [Manifest dei modelli v5](https://haku-models.tslfiles.org/eg2demo/v5/manifest.json), creato il 12 ottobre 2026. Metadata, NOTICE, configurazione reader, vocabolario e mappa di capitalizzazione scaricati e verificati rispetto ai checksum del manifest; pesi grandi non scaricati.

Ispezione statica del pacchetto: il coordinatore non ha potuto montare il DMG sul mountpoint UltraDisk; il worker lo ha montato temporaneamente read-only, copiato gli elementi ispezionati e poi scollegato. Binario, plist e risorse sono stati ricontrollati dal coordinatore, senza eseguire l'app. Prove conservate in `/Volumes/UltraDisk/sbobino-eg2-asr-20261012/static`.

## Architettura pubblicata

I metadata descrivono encoder audio che restituisce stati L11 `[1, 500, 1024]`, reader che li trasforma in vettori `[1, 500, 768]` e un'uscita `blank`, scorer e componenti separati per punteggiatura/capitalizzazione. La configurazione [reader](https://haku-models.tslfiles.org/eg2demo/v1/files/reader_G795_L11_big.json) riporta 86,45 milioni di parametri. Il vocabolario e la tabella di vettori delle parole fanno parte del pacchetto.

È quindi necessario il reader appositamente addestrato e l'intera pipeline; il solo confronto tra un embedding semantico aggregato dell'audio e parole testuali non riproduce questo ASR. La disponibilità dei modelli CoreML rende possibile studiare il percorso nativo; non abbiamo trovato il codice pubblico dell'app, di training o conversione del reader, né un'esportazione di questa pipeline per llama.cpp/MLX o CPU x86.

## Italiano e vocabolario

Il [vocabolario pubblicato](https://haku-models.tslfiles.org/eg2demo/v1/files/reader_words.json) contiene 229.457 elementi, 229.304 unici: tutti ASCII. Sono presenti anche alcune parole italiane, ma `lezione`, `trascrizione`, `matematica`, `sistema`, `università`, `perché` ed `è` non compaiono nella tabella. La mappa pubblica `punct_case.json` associa parole a classi numeriche di capitalizzazione; non documenta un meccanismo italiano di ripristino delle forme mancanti.

Questo è un limite del reader/vocabolario distribuito, non una prova che l'encoder di base non rappresenti l'italiano. La resa finale, eventuali composizioni/post-processing e il comportamento sui nomi devono essere misurati. Nessuna matrice delle lingue o valutazione italiana riproducibile trovata.

## Peso e compatibilità

Il manifest dichiara **714.975.724 byte**, circa 715 MB, per l'insieme dei modelli: il piccolo DMG non è il peso dell'installazione pronta a trascrivere. La tabella `word_unit_f16.bin` pesa 352.445.952 byte, coerenti con 229.457 × 768 × 2 byte; l'encoder pesa circa 227 MB, il reader circa 66 MB e lo scorer circa 59 MB. Questi sono pesi su disco, non misure di RAM.

Le stringhe dell'app dichiarano download iniziale, preparazione una tantum del Neural Engine e circa 2 GB aggiuntivi di spazio libero per questa preparazione. Sono messaggi del prodotto, non misure locali di picco o durata.

Il binario ispezionato è soltanto `arm64`, collega CoreML e dichiara macOS 14 nel plist. I metadata dei modelli v5 ispezionati richiedono invece macOS 15: la compatibilità effettiva con macOS 14 resta da verificare. Nessuna esecuzione Intel, Windows, Linux o iPhone; i target iOS nei metadata non dimostrano un port funzionante. I metadata della firma indicano un Developer ID dell'autore, ma la copia statica non costituisce una verifica completa del pacchetto installato o della notarizzazione.

## Prestazioni e qualità: dichiarazioni, non benchmark Sbobino

L'autore dichiara circa 13 s per un'ora di audio su M3 Air 8 GB (288× real time), WER circa 5% e addestramento in meno di due ore su 8×H100 usando speech descritto come apertamente licenziato. Non sono stati reperiti corpus del WER, split, lingue, normalizzazione o script riproducibili. Questi numeri non sono confrontabili con il nostro WER italiano su mTEDx.

La tabella dello screenshot confronta i primi due motori sullo stesso M3 Air, secondo l'autore; le righe FluidAudio usano altri Mac e altri audio, come dichiara la nota della tabella. Il numero 288× non dimostra un vantaggio rispetto al Parakeet/Whisper di Sbobino sullo stesso hardware, file e protocollo. Non sono misurati avvio freddo, memoria, confini delle finestre, silenzio, diarizzazione o fedeltà su lezioni realistiche.

Il post descrive il modello come invariato, mentre il [NOTICE distribuito](https://haku-models.tslfiles.org/eg2demo/v5/files/NOTICE) descrive fine-tuning dell'encoder audio e nuovi layer. Potrebbero riferirsi a versioni o fasi diverse; senza codice e confronto dei checkpoint non si può stabilire quale descrizione sia corretta. Il NOTICE identifica la base Apache-2.0 e modifiche dell'autore: prima di distribuire app o derivati serve chiarire i termini applicabili ai componenti concretamente utilizzati.

## Prossimo confronto utile

Trattare EG2-Transcribe come candidato ASR distinto e misurare gli stessi WAV pubblici contro le baseline: italiano breve e lungo, un controllo inglese, nomi/termini, accenti, numeri, silenzio e confini. Conservare output grezzi, WER/CER con cifre, errori per registrazione, tempi freddi/caldi e picco di memoria. Prima del corpus completo, poche clip italiane verificano se il vocabolario consente risultati sensati; un fallimento qui evita benchmark estesi poco informativi.

Per la dispensa multi-fonte, il ruolo di EmbeddingGemma resta quello concordato di recupero locale dei passaggi. Un ASR derivato può essere valutato separatamente; non certifica citazioni, correttezza delle integrazioni o qualità didattica.

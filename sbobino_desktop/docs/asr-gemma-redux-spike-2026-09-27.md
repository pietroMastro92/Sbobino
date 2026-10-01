# Spike ASR: Gemma 4 E4B e Parakeet Redux (Mac M3, 16 GB)

Stato al 2 ottobre 2026: confronti Mac completati. Sono conclusi tutti gli ASR diretti, i controlli 28/0 sui talk, lo stress di 90 minuti, i controlli Q4/F16, le 51 prove prestazionali e tutti i 24 run estesi di correzione/controllo (otto perturbazioni, otto mTEDx, otto AMI). Il batch è terminato con `BATCH_COMPLETE`; ID, finestre, riferimenti e baseline sono verificati. La revisione acustica delle modifiche e le prove x86 restano non eseguite. Nessuna integrazione nell'app.

## Protocollo e dati congelati

Tutti gli audio sono pubblici. Il corpus isolato è in `/Volumes/UltraDisk/sbobino-asr-spike-20260927/data/corpus-v1`: 30 clip FLEURS italiane di sviluppo, 200 di test, 60 perturbazioni congelate di 20 clip (rumore, 1,2×, 1 s di silenzio iniziale), un talk mTEDx di sviluppo e quattro di test, una riunione AMI far-field in inglese e una sequenza assemblata di 90 minuti solo per stabilità. Il rumore aggiunto ha SNR misurato 18,6–19,3 dB sulle 20 clip. La selezione usa seed 17 ed è congelata nel manifest. I riferimenti sono in un file separato, letto solo dal valutatore. I manifest contengono identificativi e SHA-256 di ogni WAV mono PCM16 a 16 kHz. La sequenza di stabilità non ha riferimento e non entra nel WER.

Manifest SHA-256: `e08985b5c44a9469dfcdaa0843e80026370bcc594d54f8ad275ea1b7331e65f9`. Riferimenti SHA-256: `63673cbfc6c7b60c3ac5c9c5a124ef3f86e1237f377a72706de79f1824e77e68`. La provenienza di binari, modelli, revisioni Hugging Face, archivi ed ambienti è in [`provenance.json`](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/provenance.json). Il confronto di nomi e termini usa un elenco annotato separato di 55 occorrenze in 28 clip, quindi è un controllo piccolo e non una misura esaustiva.

Prima delle prove sui talk è stato congelato un secondo elenco di 50 termini e nomi mTEDx (10 per talk), selezionati esclusivamente dai riferimenti, senza vedere output ASR: [`references-entities-long-v2.json`](/Volumes/UltraDisk/sbobino-asr-spike-20260927/data/corpus-v1/references-entities-long-v2.json), SHA-256 `50254e4ae916f872a26acc5e6b8bb253cb6ea2ed80c30e224727ccf049daa723`. Tutti e 50 sono localizzabili in un segmento temporizzato dei sottotitoli, con tempi approssimativi utili all'ascolto. Anche questo è un controllo lessicale esplorativo, non una verifica acustica.

Una verifica indipendente ha riletto checksum, formato WAV e copertura delle finestre per tutti i 297 campioni: 739 finestre da 28 s con overlap 2 s, nessun file alterato, nessuna finestra oltre 30 s e nessuna perdita dell'ultimo campione.

Per Gemma, ogni WAV è diviso in finestre di 28 s con overlap di 2 s; il controllo senza overlap è completo sul talk di sviluppo e sui quattro talk di test. Nessuna finestra supera il [limite audio di 30 s](https://ai.google.dev/gemma/docs/capabilities/audio). La ricomposizione allinea solo suffisso e prefisso delle due finestre confinanti, conserva ripetizioni ambigue e registra i casi non allineati. I timestamp sono quelli dei chunk, non delle parole generate. I due runtime Gemma sono `llama-server` con GGUF Q4_0 e proiettore BF16, e [MLX-VLM](https://github.com/Blaizzy/mlx-vlm/blob/main/mlx_vlm/models/gemma4/README.md) con modello a 4 bit. Redux usa [Photon](https://huggingface.co/moondream/parakeet-redux) su CPU e MPS. Python è confinato allo spike: nessuna API o pipeline dell'app è stata modificata.

I punteggi riportano WER e CER senza eliminare le cifre, sostituzioni/inserzioni/omissioni, punteggiatura separata e presenza dei nomi/termini annotati. Per le clip FLEURS si verifica la presenza del termine nell'intero transcript; per i talk con sottotitoli temporizzati si aggiunge la presenza nei chunk che intersecano l'intervallo di riferimento con tolleranza di 2 s. Sono indicatori lessicali, non una verifica acustica del nome, che richiede l'ascolto. Il valutatore già presente nell'app elimina le cifre nella normalizzazione; per questo lo scorer isolato dello spike conserva i numeri e non è direttamente intercambiabile con quello. WER non è una prova di fedeltà acustica: le modifiche sono conservate con gli intervalli audio per revisione. Le misure di processo includono avvio, tempo totale, RTF e picco RSS. L’avvio è misurato dopo il calcolo dei checksum dei pesi, che può riscaldare la cache del filesystem. Su memoria unificata, le allocazioni GPU MLX/PyTorch non vanno sommate al RSS. Per Redux MPS si registra anche la [memoria assegnata dal driver Metal](https://docs.pytorch.org/docs/main/generated/torch.mps.driver_allocated_memory.html) a fine chunk, che è solo un limite inferiore del possibile picco transitorio.

I confronti CPU usano le impostazioni dei runtime: il `parakeet-batch-json` installato non espone `--threads` (`parakeet_threads_supported=false` nei run), mentre Photon usa `cpu_threads=None`, cap Torch intra-op di 4 thread per il backend ternario e pool nativo automatico. Il `threads=4` generico del harness non imposta questi pool. Nei nuovi run Redux, `worker_ready` registra la politica richiesta e i conteggi Torch effettivi; il numero di worker nativi non è ancora misurato. Il vantaggio di velocità osservato riguarda quindi i runtime completi con queste impostazioni. Il wrapper nel `main` corrente supporta il cap di thread, ma il binario installato usato qui non accetta quel flag; il suo conteggio nativo effettivo resta non verificato.

La revisione acustica dei passaggi discordanti **non è stata eseguita**: l'assistente che conduce lo spike non ha accesso all'ascolto dei file in questa sessione. I runtime Gemma locali ricevono i WAV delle finestre verificate, insieme al transcript quando previsto dal protocollo. I WAV pubblici e gli intervalli di ogni modifica sono conservati per l’ascolto. Le conclusioni di qualità qui distinguono quindi accordo con il riferimento, eco delle istruzioni verificabili e fedeltà letterale ancora da confermare.

La revisione finale ha trovato che il vecchio generatore del rumore non impostava il seed FFmpeg: rigenerare i WAV non riproduceva gli stessi campioni. Il corpus corrente resta congelato e identico per tutti i confronti; non è stato rigenerato. Il generatore ora deriva e registra un seed stabile per ogni variante rumore, e un controllo eseguibile verifica due render identici e un seed diverso. Per riprodurre **questo** benchmark occorre riusare i WAV con i checksum congelati, non sostituirli con un nuovo corpus. Tutti i 296 riferimenti hanno checksum individuali verificati; il valutatore ora rifiuta testo modificato con checksum vecchio e segnala esplicitamente ID mancanti o aggiuntivi nei confronti parziali. I 19 test passano.

Whisper usa i default del runtime, incluso fallback di temperatura: il server imposta 0 iniziale e incremento 0,2, mentre la CLI mantiene i default del binario registrato. Le tre ripetizioni hanno transcript identici, ma non dimostrano determinismo quando il fallback viene attivato. I due runtime Gemma usano temperatura 0, contesti indipendenti e thinking disattivato. Il valutatore verifica l'integrità dei riferimenti e il pairing; le verifiche fisiche di audio e finestre avvengono nel runner e nell'audit del corpus, non sono sostituite dal solo checksum del manifest.

## Limiti dei riferimenti lunghi

Le trascrizioni mTEDx sono sottotitoli manuali, non verbatim revisionati: includono crediti iniziali non pronunciati, etichette editoriali di parlante, indicazioni come `[Applausi]` e code non annotate. Il valutatore decodifica le entità HTML, ma lascia tali discrepanze visibili. Il WER sui talk è esplorativo finché gli intervalli modificati non vengono ascoltati. La riunione AMI dura 1272,64 s e il runtime copre l'intero file, ma l'evaluatore condiviso considera sicura per la qualità soltanto la regione 52–652 s (600 s, 1290 parole), verificata dalla suite automatica e da una revisione indipendente; i punteggi AMI sono separati dall'italiano e calcolati solo su questa regione.

## Risultati

Le tabelle sono compilate dai JSON in `/Volumes/UltraDisk/sbobino-asr-spike-20260927/results`. I risultati a caldo sono distinti dalle prove che ricaricano il modello ad ogni clip. FLEURS test comprende 200 clip, 2846,46 s di audio e 4783 parole di riferimento; tutte le righe sotto hanno 200/200 successi. Un WER minore è migliore.

| Motore, FLEURS italiano test | WER | CER | S/D/I | Tempo totale | RTF inferenza | Picco RSS |
|---|---:|---:|---:|---:|---:|---:|
| Parakeet Q8, batch residente | 2,89% | 1,05% | 110/19/9 | 90,6 s | 0,031 | 1,25 GiB |
| Parakeet Q8 CPU, batch residente | 2,84% | 1,02% | 108/20/8 | 274,9 s | 0,096 | 1,16 GiB |
| Parakeet Q4, batch residente | 3,07% | 1,19% | 114/25/8 | 99,0 s | 0,034 | 1,35 GiB |
| Whisper large-v3-turbo Q8, CLI installata, caricamento per clip | 3,22% | 1,27% | 110/17/27 | 1298,1 s | 0,456 | 3,31 GiB |
| Whisper large-v3-turbo Q8, server Homebrew residente | 3,35% | 1,30% | 112/18/30 | 835,1 s | 0,292 | 1,07 GiB |
| Redux Photon CPU, residente | 3,83% | 1,35% | 134/28/21 | 125,2 s | 0,040 | 1,99 GiB |
| Redux Photon MPS, residente | 3,81% | 1,35% | 133/28/21 | 73,5 s | 0,023 | 1,74 GiB |
| Gemma 4 E4B Q4_0, llama.cpp residente | 4,18% | 1,57% | 140/36/24 | 1899,0 s | 0,657 | 3,24 GiB |
| Gemma 4 E4B 4 bit, MLX-VLM residente | 4,08% | 1,37% | 131/35/29 | n.d.¹ | 0,486¹ | n.d.¹ |

Numeri e punteggiatura, sugli stessi 200 transcript, sono misure separate dal WER:

| Motore | Errori token con cifre | Distanza della sequenza di punteggiatura |
|---|---:|---:|
| Parakeet Q8 MPS | 16 | 121 |
| Parakeet Q8 CPU | 16 | 123 |
| Parakeet Q4 MPS | 18 | 125 |
| Whisper CLI Q8 | 15 | 133 |
| Whisper server Q8 | 17 | 133 |
| Redux CPU | 20 | 145 |
| Redux MPS | 21 | 144 |
| Gemma llama.cpp | 16 | 131 |
| Gemma MLX | 17 | 127 |

Questi conteggi non sono accuratezza semantica dei numeri o correttezza della posizione delle virgole: il valutatore confronta token con cifre e sequenze di segni. Per esempio, cifre e numeri scritti in lettere possono esprimere lo stesso valore e risultare diversi. Q8 + MLX con audio mantiene 16 errori numerici, con distanza di punteggiatura 115 contro 121 della baseline e 126 del controllo testuale; il guadagno WER non dimostra quindi un recupero dei numeri.

Q8 residente e Q4 residente hanno prodotto rispettivamente 200/200 transcript identici ai propri run con caricamento per clip; il batch cambia solo il tempo misurato. Il caricamento Q8 è stato osservato in 0,66 s e Q4 in 0,46 s. Il server Whisper usa gli stessi pesi Q8 ma un eseguibile diverso: 131/200 transcript coincidono come stringa e 190/200 dopo la sola normalizzazione di parole; il suo tempo resta una stima separata dalla CLI installata. Con il medesimo riferimento, Redux peggiora il WER di 0,94 punti rispetto a Q8: migliora 20 registrazioni, ne peggiora 42 e pareggia 138. Il bootstrap appaiato per registrazione (10.000 ricampionamenti, seed 20260927) dà un intervallo al 95% di 0,38–1,53 punti a sfavore di Redux. Questo riguarda FLEURS italiano, non i talk o x86. Sull'elenco separato di 11 nomi/termini presenti nelle 200 clip Redux ne riproduce 9 e Parakeet Q8/Q4 7; il denominatore è troppo piccolo per una decisione.

Redux MPS ha prodotto 182 errori di parola contro 183 su CPU; 183/200 transcript sono identici. Sulle 17 clip diverse, MPS migliora 5 registrazioni e ne peggiora 4 nel WER, mentre le altre 8 cambiano solo senza differenza nel numero di errori. L'inferenza a caldo MPS richiede 66,1 s sull'audio FLEURS contro 114,8 s su CPU; tempi totali 73,5 s contro 125,2 s. Il picco RSS MPS è 1,74 GiB e l'allocazione Metal osservata a fine chunk è al massimo 1,29 GiB: quest'ultima si sovrappone alla memoria unificata e non va sommata. Entrambi riproducono 9/11 termini annotati. Rispetto a Parakeet Q8 residente, Redux MPS peggiora il WER di 0,92 punti (bootstrap appaiato 95%: +0,35–+1,51; 21 registrazioni migliori, 41 peggiori, 138 pari). La prova MPS sul Mac non è una stima delle prestazioni x86.

A parità di CPU sul Mac, Redux completa FLEURS in 125,2 s contro 274,9 s di Parakeet Q8 (2,20 volte più rapido), ma usa più RSS (1,99 contro 1,16 GiB) e peggiora il WER di 0,98 punti (3,83% contro 2,84%). Il bootstrap appaiato per clip dà +0,42–+1,56 punti al 95% per Redux CPU meno Parakeet Q8 CPU: 20 clip migliorano, 42 peggiorano, 138 restano pari. Le due esecuzioni Parakeet Q8 su CPU e MPS producono lo stesso testo in 190/200 clip; la piccola differenza nel WER non va scambiata per effetto sistematico dell'hardware. I [risultati CPU per registrazione](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/fleurs-test200-parakeet-q8-cpu.score.json) sono conservati separatamente.

I tempi tra CLI Whisper, server Whisper e batch Parakeet non hanno la stessa politica di caricamento del modello. La CLI Whisper installata ricarica per ogni clip; il server è un binario Homebrew usato solo come riferimento di modello residente. Le tre ripetizioni riportate sotto distinguono avvio, inferenza a caldo e tempo totale sullo stesso sottoinsieme.

Gemma llama.cpp ha completato 200/200 clip senza output troncati. Il suo WER è 1,30 punti sopra Parakeet Q8 e 0,96 sopra Whisper CLI; il bootstrap appaiato contro Q8 (10.000 ricampionamenti, seed 20260927) dà 0,51–2,13 punti al 95%, con 36 registrazioni migliori, 51 peggiori e 113 pari. Sugli 11 nomi/termini annotati ne riproduce 10, ancora un campione troppo piccolo per una conclusione. La prova completa è stata molto più lenta della prova di sviluppo: RTF inferenza 0,657 contro 0,231 sulle 30 clip dev. La causa non è stata accertata; la velocità sostenuta richiede ripetizioni e controllo delle condizioni macchina.

Due esempi da riascoltare prima di classificarli acusticamente: nel [WAV 0–7,62 s](/Volumes/UltraDisk/sbobino-asr-spike-20260927/data/corpus-v1/audio/fleurs-test-1885-000195-c93d8da1a4.wav), il riferimento contiene «tra le 22:00 e le 23:00 MDT», mentre Gemma produce «Talve il 2223»; nel [WAV 0–18,66 s](/Volumes/UltraDisk/sbobino-asr-spike-20260927/data/corpus-v1/audio/fleurs-test-1872-000565-0442c03442.wav), Gemma aggiunge «un raduno specifico nella città» prima di «un raduno pacifico nella città». Questi sono errori rispetto al riferimento e possibili segnali di generazione non letterale, da confermare all'ascolto.

¹ Il run MLX è stato interrotto dopo 173 risposte e completato con un secondo processo per le 27 mancanti. I due blocchi hanno lo stesso modello, manifest, device e parametri; la [fusione verificata](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/fleurs-test200-gemma-mlx-4bit-combined.json) conserva gli SHA-256 dei blocchi e l'ordine esatto dei 200 ID. Il WER è completo; il tempo totale e il picco RSS del primo processo non sono disponibili. L'RTF 0,486 è la media di 0,541 sul primo blocco e 0,159 sul secondo dopo una lunga pausa: non va usato come velocità sostenuta. MLX riporta 5,41 GiB di allocazione GPU massima nel run, sovrapposta alla memoria unificata. Il blocco finale di 27 clip ha picco RSS 1,06 GiB per il processo modello e 0,03 GiB per il runner; non è il picco dell'intera prova da 200 clip. Anche MLX ha riprodotto 10 degli 11 nomi/termini annotati.

La differenza WER appaiata MLX meno Parakeet Q8 è +1,19 punti (bootstrap 95%: +0,44–+1,98; 37 registrazioni migliori, 54 peggiori, 109 pari). MLX meno llama.cpp è −0,10 punti (−0,70–+0,45), quindi questi due runtime non mostrano una differenza di fedeltà chiara su FLEURS test.

Nel conteggio separato, gli errori sui token contenenti cifre sono 16 per Parakeet Q8, 15 per Whisper CLI, 20 per Redux CPU, 16 per Gemma llama.cpp e 17 per Gemma MLX; gli errori nella sequenza dei segni di punteggiatura sono rispettivamente 121, 133, 145, 131 e 127. Sono distanze di edit rispetto al riferimento, non percentuali né una verifica acustica.

Nelle 30 clip FLEURS di sviluppo (697 parole), la correzione automatica ha dato questi conteggi di parole errate. Le colonne audio e solo testo usano lo stesso transcript di partenza e le stesse istruzioni; nella colonna solo testo manca l'audio.

| Transcript di partenza | Errori iniziali | Gemma MLX audio / solo testo | Gemma llama.cpp audio / solo testo |
|---|---:|---:|---:|
| Parakeet Q8 | 18 | 13 / 15 | 11 / 18 |
| Whisper Q8 | 11 | 9 / 8 | 8 / 10 |

Il risultato di sviluppo suggerisce che l'audio possa recuperare alcuni errori, ma non in modo uniforme: con Whisper+MLX il controllo solo testo è migliore dell'audio nel totale. Entrambi i runtime audio hanno anche cambiato “Mahayano” in “Mahayana” nel [WAV 0–12,72 s](/Volumes/UltraDisk/sbobino-asr-spike-20260927/data/corpus-v1/audio/fleurs-dev-1595-000056-09dcaefd6b.wav), peggiorando il confronto con il riferimento; la plausibilità del nuovo nome non basta a dimostrare fedeltà all'audio. Le differenze e gli intervalli sono nei file `*.audio-correct.score.json`.

## Correzione FLEURS test (200 clip)

Parakeet Q8 parte da 138 errori di parola (WER 2,89%). Gemma MLX con l'audio della stessa finestra scende a 102 errori (WER 2,13%, CER 0,83%): 28 registrazioni migliorate, 2 peggiorate e 170 con lo stesso numero di errori di parola. Il bootstrap appaiato dà −0,75 punti WER, intervallo 95% −1,09–−0,44. Il correttore impiega 947,0 s, con 9,3 s di avvio e 931,4 s di inferenza; la pipeline Parakeet + Gemma richiede circa 1037,6 s sull'intero insieme (RTF totale circa 0,365). Si registrano 72 modifiche di chunk, 39 delle quali lessicali, e la presenza dei termini annotati passa da 7 a 9 su 11 (le due occorrenze recuperate sono «Plitvice»). Il [report per registrazione](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/fleurs-test200-q8-mlx-audio-correct.score.json) contiene il WAV e l'intervallo di ogni modifica. Il controllo Gemma MLX **senza audio**, sugli stessi 200 transcript, arriva a 123 errori (WER 2,57%) e richiede 686,2 s. L'audio ne elimina altri 21 netti: 19 registrazioni migliorano rispetto al solo testo, 4 peggiorano e 177 pareggiano. La differenza audio meno solo testo è −0,44 punti WER (bootstrap appaiato 95%: −0,74–−0,17); il solo testo migliora comunque Parakeet di −0,31 punti (−0,52–−0,12). La presenza dei termini annotati resta 7/11 senza audio e sale a 9/11 con audio. Questo misura un beneficio dell'ascolto rispetto al controllo, senza ancora provare la fedeltà acustica di ogni modifica. Il CER passa da 1,05% nella baseline a 1,04% nel solo testo e 0,83% con audio; gli errori dei segni di punteggiatura sono rispettivamente 121, 126 e 115, mentre il conteggio separato degli errori sui token numerici resta 16 in tutti e tre i casi.

Gemma llama.cpp con audio ha completato 199/200 clip partendo da Parakeet Q8: sulle 199 riuscite totalizza 101 errori di parola (WER 2,12%, CER 0,83%). Sulle stesse 199 clip Parakeet aveva 137 errori: 36 parole di miglioramento netto appaiato, 32 registrazioni migliorate e 4 peggiorate. Il controllo solo testo completa 200/200 clip e ha 131 errori (WER 2,74%); sulle 199 clip comuni, l'audio migliora 25 registrazioni e ne peggiora 3 rispetto al solo testo. Il miglioramento appaiato audio meno solo testo è −0,59 punti WER (bootstrap 95% circa −0,90–−0,32). L'audio ha richiesto 2163,3 s, contro 943,0 s del controllo testuale e 947,0 s della correzione audio MLX da Parakeet, con picco RSS del processo llama.cpp di 4,23 GiB. I tempi di run separati non sostituiscono le tre ripetizioni a condizioni macchina comparabili.

Partendo invece da Whisper Q8, Gemma llama.cpp con audio completa 199/200 clip e scende da 153 a 124 errori sulle 199 clip comuni: miglioramento netto di 29 parole, 25 registrazioni migliori e 5 peggiori. Il WER sulle sole clip riuscite è 2,60%. Il controllo solo testo completa 200/200 clip con 147 errori (WER 3,07%); sulle 199 clip comuni l'audio migliora 18 registrazioni e ne peggiora 4 rispetto al solo testo. La differenza appaiata audio meno solo testo è −0,44 punti WER (bootstrap 95% circa −0,73–−0,19). La correzione audio ha richiesto 1651,2 s, il solo testo 919,6 s; il picco RSS del processo llama.cpp è 5,68 GiB.

Con Whisper come sorgente, Gemma MLX con audio completa 200/200 clip e riduce gli errori da 154 (WER 3,22%) a 130 (2,72%); il controllo solo testo ha 144 errori (3,01%). L'audio migliora 16 registrazioni e ne peggiora 7 rispetto al solo testo; il delta WER appaiato è −0,29 punti (bootstrap 95%: −0,56–−0,06). Rispetto a Whisper iniziale il delta è −0,50 punti (−0,79–−0,25). I nomi/termini annotati restano 10/11 sia nella baseline sia nel controllo testuale sia con audio: qui il vantaggio è distribuito su altre parole. Il run audio MLX si è interrotto dopo 92 clip ed è stato completato con 108 clip in un secondo processo; la [fusione verificata](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/fleurs-test200-whisper-mlx-audio-correct-combined.json) conserva gli SHA dei due blocchi e abbina tutti i 200 ID alle finestre di Whisper. Perciò qualità e modifica del transcript sono complete, ma tempo totale e picco RSS dell'intero run non sono misurabili; 5,47 GiB è la massima allocazione GPU osservata nei chunk. Il controllo solo testo MLX ha richiesto 470,5 s. Il [report appaiato](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/fleurs-test200-whisper-mlx-audio-correct-combined.score.json) conserva tutte le differenze e gli intervalli audio.

I due run audio llama.cpp falliscono **sulla stessa clip**, quindi sono due fallimenti di run ma un solo audio problematico: nel [WAV 0–9,00 s](/Volumes/UltraDisk/sbobino-asr-spike-20260927/data/corpus-v1/audio/fleurs-test-1966-000035-9f1c6b8d7d.wav) entrambi i transcript di partenza riportano «Martelli», mentre il riferimento ha «Martelly»; Gemma ripete «protest» fino al limite di 512 token. Gli output grezzi sono `truncated` e non entrano nel WER delle clip riuscite. Un fallback che conservi il transcript originale darebbe 102 errori da Parakeet e 125 da Whisper sulle 200 clip, ma questa è una simulazione, non il risultato dei due run. Ho provato separatamente il parametro [DRY di llama.cpp](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md) a 0,8, senza alterare il run originale: sulla clip del loop evita il troncamento ma produce 6 errori di parola invece dell'unico errore del transcript Parakeet; su una seconda clip con la ripetizione autentica «se se» conserva quelle due parole ma introduce 4 errori rispetto a 0 di Parakeet e del llama.cpp originale. Il [diagnostico grezzo](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/fleurs-diagnostic-llama-dry08-q8.json) mostra che DRY non è affidabile in questi due casi; non basta a stabilire una regola generale sulla fedeltà letterale. Il valutatore registra una simulazione conservativa separata: su un output troncato mantiene il transcript ASR originale e continua a contare il fallimento del correttore.

Esempi in cui il controllo solo testo non recupera il termine ma la versione con audio coincide con il riferimento: «Pokawa» → «Pohutukawa» nel [WAV 0–10,68 s](/Volumes/UltraDisk/sbobino-asr-spike-20260927/data/corpus-v1/audio/fleurs-test-1965-000086-bc87fa90dd.wav), «Pletvic» → «Plitvice» nel [WAV 0–12,48 s](/Volumes/UltraDisk/sbobino-asr-spike-20260927/data/corpus-v1/audio/fleurs-test-1831-000657-de92a8280c.wav), e più parti del nome «Tupua Tamasese Lealofi» nel [WAV 0–18,66 s](/Volumes/UltraDisk/sbobino-asr-spike-20260927/data/corpus-v1/audio/fleurs-test-1872-000565-0442c03442.wav). Tra i quattro casi in cui l'audio peggiora il WER rispetto al solo testo c'è «sonoro» → «suono» nel [WAV 0–22,02 s](/Volumes/UltraDisk/sbobino-asr-spike-20260927/data/corpus-v1/audio/fleurs-test-1789-000457-9d13ae32cf.wav). Tutti questi giudizi sono relativi al riferimento; la revisione acustica resta necessaria.

## Talk mTEDx completi: risultati preliminari

Parakeet Q8 ha completato i cinque talk (4866,7 s di audio) in 110,8 s complessivi, inferenza 109,8 s, RTF a caldo 0,023 e picco RSS 1,88 GiB. Il WER aggregato contro i sottotitoli manuali è 12,18%; 40/50 nomi e termini annotati sono presenti anche nei chunk vicini al loro intervallo di riferimento. Questi numeri non sono ancora una classifica di fedeltà acustica: i sottotitoli contengono elementi editoriali e richiedono ascolto degli intervalli discordanti.

| Talk | Durata | WER Q8 / Whisper | Errori Q8 / Whisper | Termini Q8 / Whisper |
|---|---:|---:|---:|---:|
| [Sviluppo, IA/Turing](/Volumes/UltraDisk/sbobino-asr-spike-20260927/data/corpus-v1/audio/mtedx-dev-o7krwo3pvz8-75133c7118.wav) | 886,4 s | 7,13% / 8,93% | 115 / 144 | 9/10 / 10/10 |
| [Test, blockchain](/Volumes/UltraDisk/sbobino-asr-spike-20260927/data/corpus-v1/audio/mtedx-test-binubkb-mn0-82d1d5dfd9.wav) | 928,9 s | 10,49% / 10,62% | 237 / 240 | 8/10 / 9/10 |
| [Test, industria 4.0](/Volumes/UltraDisk/sbobino-asr-spike-20260927/data/corpus-v1/audio/mtedx-test-qlzosmhzg4c-137b4282c9.wav) | 902,1 s | 8,68% / 9,05% | 164 / 171 | 8/10 / 9/10 |
| [Test, spazio](/Volumes/UltraDisk/sbobino-asr-spike-20260927/data/corpus-v1/audio/mtedx-test-7tqiezn5ube-22e54b0c0e.wav) | 1104,1 s | 18,59% / 19,37% | 544 / 567 | 7/10 / 6/10 |
| [Test, genetica/5G](/Volumes/UltraDisk/sbobino-asr-spike-20260927/data/corpus-v1/audio/mtedx-test-zlj5srro7p4-a70788d854.wav) | 1045,2 s | 12,11% / 11,65% | 265 / 255 | 8/10 / 9/10 |

Whisper ha completato gli stessi cinque talk con WER aggregato 12,66% contro 12,18% di Parakeet Q8, 1377 contro 1325 errori di parola e 43/50 contro 40/50 termini vicini al proprio intervallo. La CLI Whisper installata ha impiegato 1554,9 s e ricarica il modello ad ogni finestra; questo tempo confronta politiche di caricamento diverse. Il picco RSS è 4,47 GiB. Whisper peggiora il WER in quattro talk e migliora nel quinto; il confronto dei termini è inverso in tre talk e favorevole a Parakeet in quello sullo spazio. La differenza aggregata Whisper meno Parakeet è +0,48 punti WER, ma il bootstrap appaiato per soli cinque talk ha intervallo 95% −0,06–+1,13: non mostra un vantaggio di qualità chiaro. Il [report Whisper per talk](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/mtedx-o2-whisper.score.json) mantiene i punteggi separati.

Redux Photon completa i cinque talk su CPU e MPS. Il WER sui sottotitoli è 13,08% su CPU (1423 errori) e 13,27% su MPS (1443), contro 12,18% di Parakeet Q8 su MPS; entrambi i device Redux riproducono 38/50 termini negli intervalli vicini, contro 40/50 di Q8 e 43/50 di Whisper. Redux impiega 246,5 s su CPU e 152,3 s su MPS, mentre Parakeet Q8 su MPS ne impiega 110,8. Picco RSS Redux CPU 1,16 GiB, MPS 0,75 GiB; l'allocazione Metal osservata a fine chunk raggiunge 1,26 GiB, sovrapposta alla memoria unificata. Il bootstrap per cinque talk dà Redux CPU meno Q8 MPS +0,90 punti WER (95% +0,03–+1,70) e Redux MPS meno Q8 MPS +1,08 (+0,13–+1,96); questi intervalli confrontano sottotitoli imperfetti, non parlato revisionato acusticamente. I [report CPU](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/mtedx-o2-redux-cpu.score.json) e [MPS](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/mtedx-o2-redux-mps.score.json) mantengono i risultati per talk.

Parakeet Q8 su CPU completa gli stessi cinque talk in 548,9 s, contro 246,5 s di Redux CPU: Redux è 2,23 volte più veloce sul Mac, con picchi RSS simili (1,16 contro 1,13 GiB). Q8 CPU ottiene 1318 errori (WER 12,12%) e 40/50 termini vicini al riferimento, contro 1423 errori (13,08%) e 38/50 di Redux CPU. La differenza Redux meno Q8 CPU è +0,97 punti WER; il bootstrap appaiato per talk dà +0,11–+1,76 punti al 95%. Redux migliora due talk e ne peggiora tre: il confronto aggregato è dominato anche dalla diversa lunghezza dei talk. Il [report Q8 CPU per talk](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/mtedx-o2-parakeet-q8-cpu.score.json) conserva ogni riferimento e risultato. La prova x86 reale resta successiva.


Parakeet **Q4** completa i cinque talk con 1323 errori su 10878 parole (WER 12,16%, CER 8,51%), contro 1325 di Q8: la differenza di due parole non indica un vantaggio robusto. Sui quattro test Q4 ha 1195 errori (12,90%) contro 1210 di Q8 (13,06%), mentre sullo sviluppo ne ha 128 contro 115. Riproduce gli stessi 40/50 termini. Il tempo totale è 150,9 s, inferenza 149,9 s, avvio 0,26 s e RSS modello 1,39 GiB. Il controllo **F16 sul solo sviluppo** dà 118 errori (WER 7,32%, CER 4,31%), 9/10 termini, 25,2 s complessivi e RSS 2,81 GiB: non spiega il divario di Gemma rispetto a Q8 su questo talk. Questi sono controlli diagnostici, non un confronto prestazionale ripetuto tra talk. I [punteggi Q4](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/mtedx-o2-parakeet-q4.score.json) e [F16](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/mtedx-dev-o2-parakeet-f16.score.json) restano separati.

Gemma MLX con overlap di 2 s completa il talk tecnico di sviluppo da 886,4 s in 207,5 s (196,3 s di inferenza, avvio 10,3 s), 35 chunk tutti riusciti. Contro i sottotitoli ha 178 errori su 1612 parole (WER 11,04%, CER 7,67%), rispetto ai 115 errori di Q8, 144 di Whisper e 151 di Redux CPU sullo stesso talk. Riproduce 10/10 termini annotati vicino al loro intervallo, ma segnala 20 confini con overlap non allineato o ripetizione ambigua. Nel [chunk finale a 884–886,4 s](/Volumes/UltraDisk/sbobino-asr-spike-20260927/data/corpus-v1/audio/mtedx-dev-o7krwo3pvz8-75133c7118.wav) produce «Trascrizione non disponibile.»; Parakeet e Redux restituiscono testo vuoto, Whisper «Grazie.». Va ascoltato prima di classificarlo come allucinazione. Il picco RSS del processo è 1,13 GiB e l'allocazione GPU massima osservata è 5,38 GiB sulla memoria unificata: non vanno sommati. Il [run grezzo](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/mtedx-dev-o2-gemma-mlx.json) e il [punteggio](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/mtedx-dev-o2-gemma-mlx.score.json) conservano i 35 transcript e i confini per revisione. Questa singola registrazione non basta a giudicare il runtime sui quattro talk di test.

Gemma llama.cpp sullo stesso talk e con le stesse finestre impiega 355,7 s (339,7 s di inferenza, avvio 14,6 s) e produce 203 errori (WER 12,59%, CER 8,20%); MLX è 1,71 volte più rapido nel tempo totale e riduce di 25 errori il confronto con i sottotitoli. Entrambi i runtime trovano localmente 10/10 termini annotati, ma llama.cpp segnala 22 confini non allineati o ambigui. Il suo picco RSS è 3,81 GiB. Sul chunk finale restituisce «Trascrizione: [Musica]», una differenza da riascoltare con gli altri output. [Run grezzo](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/mtedx-dev-o2-gemma-llama.json) e [punteggio](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/mtedx-dev-o2-gemma-llama.score.json) restano separati; questa velocità non sostituisce le tre ripetizioni prestazionali richieste.

Il controllo MLX senza overlap sullo stesso talk produce 150 errori (WER 9,31%, CER 4,66%) in 240,4 s, contro 178 errori (11,04%) in 207,5 s con overlap di 2 s. Usa 32 chunk invece di 35, riproduce ancora 10/10 termini e chiude con una frase completa e «Grazie.» invece del segnaposto del breve chunk finale con overlap. Al confine intorno a 78–84 s la frase «un metodo usato fin dall'antichità» compare una sola volta in entrambe le ricomposizioni MLX; quel caso non spiega da solo il divario. Il risultato [grezzo](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/mtedx-dev-o0-gemma-mlx.json) e lo [score](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/mtedx-dev-o0-gemma-mlx.score.json) non dimostrano che l'assenza di overlap sia generalmente migliore: cambiano insieme le finestre e la lunghezza dell'ultimo chunk, quindi servono gli altri talk e l'ascolto dei confini.

Anche llama.cpp senza overlap migliora su questo talk: 188 errori (WER 11,66%, CER 5,60%) in 364,4 s, contro 203 (12,59%) in 355,7 s con overlap. Riproduce 10/10 termini e l'ultimo chunk contiene la frase conclusiva e «Grazie.» anziché il breve output «Trascrizione: [Musica]». I 32 [chunk grezzi](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/mtedx-dev-o0-gemma-llama.json) e lo [score](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/mtedx-dev-o0-gemma-llama.score.json) confermano un effetto della segmentazione su entrambi i runtime, senza isolare ancora se il miglioramento venga dal confine finale o dagli altri spostamenti delle finestre.

### Quattro talk mTEDx di test: Gemma diretto

Il run Gemma MLX diretto con finestre da 28 s e overlap di 2 s ha completato con esito ok tutti e quattro i talk di test e tutti i 155 chunk. Sui soli quattro ID comuni (9266 parole di riferimento), MLX totalizza 1693 errori (WER 18,27%), contro 1210 di Parakeet Q8 MPS (13,06%) e 1233 di Whisper (13,31%). I termini presenti nel chunk vicino all'intervallo di riferimento sono 35/40 per MLX, 31/40 per Q8 e 33/40 per Whisper.

| Talk | WER Q8 MPS | WER Whisper | MLX 28/2 | llama.cpp 28/2 | MLX 28/0 | llama.cpp 28/0 |
|---|---:|---:|---:|---:|---:|---:|
| [Test, blockchain](/Volumes/UltraDisk/sbobino-asr-spike-20260927/data/corpus-v1/audio/mtedx-test-binubkb-mn0-82d1d5dfd9.wav) | 10,49% | 10,62% | 18,01% | 17,21% | 14,29% | 13,89% |
| [Test, industria 4.0](/Volumes/UltraDisk/sbobino-asr-spike-20260927/data/corpus-v1/audio/mtedx-test-qlzosmhzg4c-137b4282c9.wav) | 8,68% | 9,05% | 10,16% | 12,28% | 7,99% | 8,15% |
| [Test, spazio](/Volumes/UltraDisk/sbobino-asr-spike-20260927/data/corpus-v1/audio/mtedx-test-7tqiezn5ube-22e54b0c0e.wav) | 18,59% | 19,37% | 24,67% | 24,80% | 21,42% | 20,98% |
| [Test, genetica/5G](/Volumes/UltraDisk/sbobino-asr-spike-20260927/data/corpus-v1/audio/mtedx-test-zlj5srro7p4-a70788d854.wav) | 12,11% | 11,65% | 16,99% | 18,18% | 14,62% | 14,44% |

Per l'intero run MLX il tempo trascorso è 1187,8167 s, di cui 1178,5063 s di inferenza e 7,6635 s di avvio; il picco RSS del processo è 2,34175 GiB e l'allocazione GPU massima 5,38245 GiB. Quest'ultima si sovrappone alla memoria unificata e non va sommata al RSS. Le [risposte grezze](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/mtedx-test-o2-gemma-mlx.json) e il [punteggio](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/mtedx-test-o2-gemma-mlx.score.json) conservano i quattro ID, i 155 chunk e gli intervalli.

Il punteggio MLX contiene 1107 inserzioni e 83 segnalazioni di confine (`boundary_findings`) sui quattro talk; Q8 ne ha 61 e Whisper 64. Sono segnali di rischio nel confronto con sottotitoli manuali, insieme alle discrepanze editoriali già descritte, e non una prova acustica: gli intervalli modificati e i confini vanno ascoltati. Anche llama.cpp completa tutti i quattro talk e i 155 chunk senza troncamenti, con 1745 errori sulle stesse 9266 parole (WER 18,83%, CER 13,54%), 34/40 termini localizzati, 1053 inserzioni e 98 segnalazioni di confine. Impiega 2070,7 s complessivi, 2048,8 s di inferenza e 17,0 s di avvio, con picco RSS modello 4,23 GiB. Il [run](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/mtedx-test-o2-gemma-llama.json) e lo [score](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/mtedx-test-o2-gemma-llama.score.json) conservano anche un’eco del prompt sul tail 1040–1045,16 s del talk genetica/5G, inclusa nel WER. Entrambi i controlli senza overlap sui test sono completi.


Il controllo **MLX senza overlap (28/0)** completa gli stessi quattro talk, con 1421 errori sulle 9266 parole: WER **15,34%**, CER 9,46%, S/D/I **494/170/757** e 34/40 termini localizzati. Rispetto a 28/2 elimina 350 inserzioni ma aggiunge 59 sostituzioni e 19 omissioni, per 272 errori netti in meno. Il WER resta sopra Q8 (13,06%) e Whisper (13,31%); sul solo talk industria il valore 7,99% è inferiore a Q8 8,68%. Gli altri tre sono 14,29% (blockchain), 21,42% (spazio) e 14,62% (genetica). Il [run 28/0](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/mtedx-test-o0-gemma-mlx.json) e lo [score](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/mtedx-test-o0-gemma-mlx.score.json) conservano tutti gli intervalli, compresa un'eco del prompt nel tail blockchain 924–928,90 s, inclusa nelle metriche. Le finestre diverse spostano anche i confini: il miglioramento è coerente con un problema di overlap/ricomposizione, ma non ne isola da solo la causa. L'assenza di `boundary_findings` in 28/0 è prevista dal ricompositore e non prova assenza di errori ai confini.

MLX 28/0 impiega 1342,5 s complessivi, 1329,1 s di inferenza e 10,8 s di avvio, con RSS modello 0,92 GiB e GPU osservata 5,38 GiB, da non sommare. È più lento del run 28/2 nonostante meno finestre: i due run separati non dimostrano un vantaggio prestazionale del chunking. Le tre ripetizioni su clip identiche sono ora complete e riportate separatamente per misurare la variabilità dei runtime.


Anche **llama.cpp 28/0** completa 145/145 chunk, con 1398 errori sulle stesse 9266 parole: WER **15,09%**, CER 9,48%, S/D/I **530/223/645**, 34/40 termini. Rispetto a 28/2 riduce di 408 le inserzioni ma aggiunge 57 sostituzioni e quattro omissioni: 347 errori netti in meno. Mantiene un’eco del prompt sul tail blockchain 924–928,90 s, inclusa nel punteggio. È migliore di Q8 e Whisper sul solo talk industria; nel totale resta peggiore. Impiega 1714,1 s (1707,4 s inferenza, avvio 4,1 s), con RSS modello 5,70 GiB. I [risultati 28/0](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/mtedx-test-o0-gemma-llama.score.json) preservano questi output.

La latenza dei chunk sui quattro test, p50/p95/massimo, è 7,58/9,65/12,24 s per MLX 28/2, 9,30/10,96/13,16 s per MLX 28/0, 13,32/17,36/18,98 s per llama.cpp 28/2 e 11,98/14,89/17,31 s per llama.cpp 28/0. Sono distribuzioni di singoli run completi, distinte dalle tre ripetizioni su clip identiche. Tutti i quattro run diretti hanno `status=ok`, senza marker di troncamento.


## Correzione sul talk tecnico di sviluppo

Partendo dai 35 chunk Parakeet Q8, la correzione audio-aware MLX completa il talk ma passa da 115 a 138 errori di parola sui sottotitoli (WER 7,13% → 8,56%). Impiega 312,7 s (304,7 s di inferenza), modifica 27 chunk, 18 nelle parole, e recupera localmente «Bletchley Park» da «Bleachy Park», portando i termini annotati da 9/10 a 10/10. Nel [primo chunk, 0–28 s](/Volumes/UltraDisk/sbobino-asr-spike-20260927/data/corpus-v1/audio/mtedx-dev-o7krwo3pvz8-75133c7118.wav) lascia la frase originale che inizia «Nel duemilici mi è stato conferito un premio» e **aggiunge di seguito una seconda versione quasi identica** che inizia «Nel 2016 mi è stato conferito un premio». Il riferimento contiene la frase una sola volta. È un errore di duplicazione rispetto al riferimento e un segnale concreto che il prompt restrittivo non basta a garantire modifiche conservative; la verifica acustica è ancora necessaria per classificare ogni altra modifica. Il [punteggio appaiato](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/mtedx-dev-q8-mlx-audio-correct.score.json) conserva i 27 intervalli prima/dopo.

Il controllo MLX **senza audio** sugli stessi chunk e con lo stesso transcript iniziale produce 130 errori (WER 8,06%, CER 5,05%) in 165,7 s, mentre la versione con audio ne produce 138 (CER 5,17%) in 312,7 s. Entrambe peggiorano Parakeet Q8; l'audio aggiunge otto errori netti rispetto al solo testo, anziché dimostrare un recupero. Anche il controllo testuale trova localmente 10/10 termini e corregge «Bleachy Park» in «Bletchley Park», quindi quel nome non prova un beneficio dell'ascolto in questa registrazione. La duplicazione del primo chunk compare soltanto nella versione con audio. Il [punteggio del controllo](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/mtedx-dev-q8-mlx-text-only.score.json) rende verificabile il confronto. Il tempo complessivo ASR + Gemma non è sommato automaticamente qui perché il run sorgente Parakeet comprende altri quattro talk; per questa registrazione sono disponibili i tempi di inferenza appaiati dei due stadi.

Gemma llama.cpp con audio sullo stesso transcript peggiora ulteriormente: 164 errori (WER 10,17%, CER 8,17%) contro i 115 iniziali, in 479,9 s e con picco RSS 5,57 GiB. Modifica 13 chunk, 9 nelle parole, e trova localmente 10/10 termini grazie anche a «Bleachy» → «Bletchley». Non duplica il primo chunk come MLX, ma nel [brevissimo chunk finale 884–886,4 s](/Volumes/UltraDisk/sbobino-asr-spike-20260927/data/corpus-v1/audio/mtedx-dev-o7krwo3pvz8-75133c7118.wav), il cui transcript Parakeet è vuoto, restituisce integralmente le istruzioni che iniziano «Correggi soltanto errori chiaramente smentiti dall'audio». È un'eco del prompt, non una trascrizione del parlato, e dimostra che il vincolo testuale non impedisce un output inventato sul tail corto. Il [run grezzo](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/mtedx-dev-q8-llama-audio-correct.json) e il [report delle modifiche](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/mtedx-dev-q8-llama-audio-correct.score.json) conservano l'errore.

Il controllo llama.cpp **solo testo** produce 128 errori (WER 7,94%, CER 4,87%) in 234,1 s, contro 164 con audio; entrambi recuperano 10/10 termini, ma sul chunk finale vuoto anche il controllo genera una risposta metatestuale («Non è stato fornito alcun audio o transcript...»). Una diagnosi separata che esclude **solo il chunk finale 884–886,4 s** dal transcript ricomposto, lasciando invariato il riferimento e tutti gli altri chunk, dà: Parakeet 115 errori, MLX audio 138, MLX solo testo 118, llama.cpp audio 109, llama.cpp solo testo 113. Quindi l'eco del prompt spiega il peggioramento aggregato di llama.cpp audio; nei chunk precedenti il run recupera sei errori netti rispetto a Parakeet e quattro rispetto al controllo testuale. Non è il risultato primario e non dimostra ancora una regola sicura per riconoscere audio senza parlato: il tail ha energia non nulla e il transcript Parakeet vuoto non prova silenzio. Nessun output grezzo è stato modificato.

La ricomposizione del run Parakeet segnala 60 confini con overlap non allineato, 10 ripetizioni ambigue e 3 chunk vuoti. Un caso concreto è nel [talk di sviluppo a 76–83 s](/Volumes/UltraDisk/sbobino-asr-spike-20260927/data/corpus-v1/audio/mtedx-dev-o7krwo3pvz8-75133c7118.wav): il chunk sinistro termina con «un metodo usato fin da», il destro inizia con «Un metodo usato fin dall'antichità» e la ricomposizione conserva entrambe le versioni. Il riferimento contiene una sola occorrenza. L'allineamento esatto lo segnala come non allineato, ma non può scegliere quale variante corrisponda all'audio; questo confine resta da ascoltare e correggere prima di dichiarare la pipeline lunga pronta. I transcript grezzi e il [report per talk](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/mtedx-o2-parakeet-q8.score.json) conservano gli intervalli per confrontare le altre implementazioni e il controllo Gemma senza overlap.


## Correzione Q8 + MLX sui cinque talk completi

Il run completo con audio ha 5/5 successi e **1386 errori su 10878 parole (WER 12,74%, CER 8,93%)**, contro 1325 di Q8 (12,18%) e 1357 del controllo solo testo (12,47%). Peggiora quattro talk rispetto a Q8 e ne migliora uno: 61 errori netti aggiuntivi. Rispetto al controllo testuale peggiora tre talk e ne migliora due, per 29 errori netti in più. Sui soli **quattro test** i conteggi sono 1210 Q8, 1227 solo testo e 1248 audio, con WER rispettivamente 13,06%, 13,24% e 13,47%: il peggioramento non dipende soltanto dal talk di sviluppo.

| Talk | WER Q8 | WER solo testo | WER con audio |
|---|---:|---:|---:|
| Sviluppo, IA/Turing | 7,13% | 8,06% | 8,56% |
| Test, blockchain | 10,49% | 10,18% | 11,55% |
| Test, industria 4.0 | 8,68% | 8,68% | 8,20% |
| Test, spazio | 18,59% | 18,82% | 18,93% |
| Test, genetica/5G | 12,11% | 12,88% | 12,70% |

La presenza localizzata dei termini passa da **40/50** in Q8 a **42/50** nel solo testo e **45/50** con audio. “Industry 4.0”, “meccatronica” e “Frederick Sanger” compaiono con audio nei chunk vicini al riferimento, mentre mancano sia in Q8 sia nel controllo solo testo. È un recupero lessicale interessante, da confermare all’ascolto, che non compensa l’aumento complessivo di errori: l’audio ha S/D/I **392/170/824**, contro **429/185/711** della baseline. Non risultano troncamenti o eco esatte del prompt; questo non esclude altri output non fedeli o duplicazioni.

Il correttore impiega 1368,2 s complessivi (1357,5 s inferenza, avvio 9,5 s), con RSS modello 0,92 GiB e GPU runtime 5,49 GiB, sovrapposti. La pipeline **Q8 + Gemma + ricomposizione** impiega 1479,0 s sugli stessi cinque file, contro 110,8 s della baseline. Il controllo testuale impiega 1072,2 s; pipeline Q8 + solo testo 1183,0 s. I [risultati con audio](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/mtedx-all-q8-mlx-audio-correct.score.json) e il [controllo testuale](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/mtedx-all-q8-mlx-text-only.score.json) conservano tutte le modifiche e gli intervalli. Nei primi due talk, i transcript e ogni chunk grezzo coincidono con il checkpoint interrotto preservato; il nuovo run ha tempi e memoria completi.


Anche i due run **Q8 + llama.cpp** sui cinque talk sono completi: con audio hanno 1460 errori (WER **13,42%**), contro 1364 del controllo solo testo (**12,54%**) e 1325 della baseline (**12,18%**). L’audio peggiora quattro talk e ne migliora uno, aggiungendo 135 errori netti rispetto a Q8; i termini localizzati sono 44/50 con audio e 41/50 senza. Il correttore audio richiede 2381,0 s, quello testuale 1235,9 s. Non risultano troncamenti in questi due run. Il [confronto completo llama.cpp](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/mtedx-all-q8-llama-audio-correct.score.json) rafforza la decisione di non promuovere la correzione automatica dei talk, con la verifica acustica ancora necessaria; il confronto completo da Whisper è riportato sotto.

## Correzione Whisper + MLX sui cinque talk completi

I due run MLX da Whisper completano tutti e cinque i talk e i 190 chunk, senza troncamenti o eco esatte del prompt. La baseline ha **1377 errori (WER 12,66%)**, il controllo solo testo **1368 (12,58%)**, la correzione con audio **1413 (12,99%, CER 9,10%)**: l'audio aggiunge 36 errori rispetto a Whisper e 45 rispetto al controllo testuale. Migliora un talk e ne peggiora quattro rispetto a entrambe le alternative. Sui quattro test: Whisper **1233/9266 (13,31%)**, solo testo **1224 (13,21%)**, audio **1264 (13,64%)**.

| Talk | Errori Whisper | Solo testo MLX | Con audio MLX |
|---|---:|---:|---:|
| Sviluppo, IA/Turing | 144 | 144 | 149 |
| Test, blockchain | 240 | 240 | 250 |
| Test, industria 4.0 | 171 | 169 | 161 |
| Test, spazio | 567 | 561 | 586 |
| Test, genetica/5G | 255 | 254 | 267 |

I termini localizzati passano da 43/50, sia nella baseline sia nel controllo, a 44/50 con audio: l'unico nuovo match è “Industry 4.0”. S/D/I con audio **424/145/844**; 30 errori numerici, invariati rispetto alla baseline e al controllo. La distanza di punteggiatura scende da 553 a 538, ma non compensa l'aumento degli errori nelle parole. Il [confronto appaiato](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/mtedx-all-whisper-mlx-audio-correct.score.json) conserva tutti i prima/dopo e gli intervalli audio.

Gemma con audio impiega **1895,8 s** (1886,7 s inferenza, avvio 7,5 s), RSS modello 1,55 GiB e allocazione GPU 5,49 GiB sovrapposti. Il solo testo impiega 1855,0 s; questi due run separati non dimostrano che l'audio abbia lo stesso costo. La pipeline Whisper + audio + ricomposizione impiega **3450,7 s**, contro 1554,9 s della CLI Whisper con ricaricamento per finestra; pipeline col solo testo 3409,9 s. Questi tempi non sono una misura della pipeline integrata dell'app. I due run llama.cpp da Whisper sono riportati nella sezione successiva.

## Correzione Whisper + llama.cpp sui cinque talk completi

Anche llama.cpp completa tutti i cinque talk e i 190 chunk nei due modi, senza troncamenti o eco esatte del prompt. Con audio ha **1378 errori (WER 12,67%, CER 8,98%)**, contro **1377 della baseline (12,66%)** e **1376 del solo testo (12,65%)**. Il cambiamento aggregato è sostanzialmente nullo: l'audio migliora due talk e ne peggiora tre rispetto a entrambe le alternative. I termini localizzati restano **43/50**, con gli stessi match della baseline e del controllo: non emerge un recupero aggiuntivo dei termini annotati in questo run.

| Talk | Errori Whisper | Solo testo llama.cpp | Con audio llama.cpp |
|---|---:|---:|---:|
| Sviluppo, IA/Turing | 144 | 144 | 142 |
| Test, blockchain | 240 | 240 | 241 |
| Test, industria 4.0 | 171 | 170 | 158 |
| Test, spazio | 567 | 568 | 570 |
| Test, genetica/5G | 255 | 254 | 267 |

Sui quattro test: **1233 errori Whisper, 1232 solo testo, 1236 con audio**, su 9266 parole. Con audio S/D/I **415/178/785**, errori numerici 29 contro 30 della baseline e del controllo, distanza di punteggiatura 555 contro 553 della baseline e 552 del controllo. Il [confronto appaiato](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/mtedx-all-whisper-llama-audio-correct.score.json) conserva le differenze, senza trasformare il miglioramento del singolo talk industria in un vantaggio complessivo.

Il correttore audio impiega **2488,3 s** (2483,6 s inferenza, avvio 2,1 s), RSS modello **6,10 GiB**; il controllo testuale **1407,3 s**, RSS 5,64 GiB. La pipeline Whisper + audio + ricomposizione impiega **4043,1 s**, contro 1554,9 s della baseline CLI con ricaricamento per finestra; col solo testo 2962,1 s. Tutti gli otto run del correttore sui talk sono completi. I risultati AMI sono riportati sotto; la revisione acustica resta da eseguire.

## Quadro appaiato sui quattro talk italiani di test

Gli stessi quattro talk contengono 9266 parole di riferimento. Il talk di sviluppo è escluso da questa tabella. I tempi e la memoria delle sezioni precedenti riguardano invece il gruppo completo di cinque talk; non vengono attribuiti artificialmente a questo sottoinsieme.

| ASR → runtime correttore | ASR: errori / WER | Solo testo: errori / WER | Con audio: errori / WER | Termini localizzati ASR → testo → audio |
|---|---:|---:|---:|---:|
| Parakeet Q8 → MLX | 1210 / 13,06% | 1227 / 13,24% | 1248 / 13,47% | 31 → 32 → 35 su 40 |
| Parakeet Q8 → llama.cpp | 1210 / 13,06% | 1236 / 13,34% | 1296 / 13,99% | 31 → 31 → 34 su 40 |
| Whisper → MLX | 1233 / 13,31% | 1224 / 13,21% | 1264 / 13,64% | 33 → 33 → 34 su 40 |
| Whisper → llama.cpp | 1233 / 13,31% | 1232 / 13,30% | 1236 / 13,34% | 33 → 33 → 33 su 40 |

Nessuna delle quattro correzioni con audio migliora il WER aggregato rispetto all'ASR o al proprio controllo testuale su questi talk. Il recupero lessicale di alcuni termini coesiste con più errori complessivi. Il riferimento resta un sottotitolo manuale non revisionato acusticamente: la tabella giustifica prudenza sull'integrazione, senza certificare la correttezza di ogni edit.

## Perturbazioni delle 20 clip di sviluppo

Tutti e sette i candidati hanno completato le 60 varianti senza errori di esecuzione o output troncati. Ogni colonna comprende gli stessi 20 audio e 480 parole di riferimento; tra parentesi è il WER della colonna.

| Motore | Pulito (20/480) | Rumore (20/480) | Velocità 1,2× (20/480) | Silenzio iniziale 1 s (20/480) | WER totale (60/1440) | Tempo totale | Picco RSS |
|---|---:|---:|---:|---:|---:|---:|---:|
| Parakeet Q8 MPS | 14 (2,92%) | 14 (2,92%) | 13 (2,71%) | 15 (3,13%) | 2,92% | 21,3 s | 1,29 GiB |
| Parakeet Q4 MPS | 16 (3,33%) | 15 (3,13%) | 15 (3,13%) | 16 (3,33%) | 3,19% | 28,2 s | 1,22 GiB |
| Redux CPU | 14 (2,92%) | 14 (2,92%) | 15 (3,13%) | 13 (2,71%) | 2,92% | 97,0 s | 1,41 GiB |
| Redux MPS | 14 (2,92%) | 14 (2,92%) | 15 (3,13%) | 15 (3,13%) | 3,06% | 54,5 s | 0,75 GiB |
| Whisper large-v3-turbo Q8, CLI MPS | 9 (1,88%) | 11 (2,29%) | 8 (1,67%) | 8 (1,67%) | 1,88% | 301,6 s | 4,44 GiB |
| Gemma 4 E4B 4 bit, MLX-VLM | 7 (1,46%) | 8 (1,67%) | 8 (1,67%) | 9 (1,88%) | 1,74% | 216,0 s | 1,18 GiB |
| Gemma 4 E4B Q4_0, llama.cpp | 10 (2,08%) | 13 (2,71%) | 14 (2,92%) | 13 (2,71%) | 2,78% | 434,1 s | 2,87 GiB |

llama.cpp produce 40 errori sulle varianti, contro 10 sulle 20 clip pulite abbinate; il suo WER passa dal 2,08% pulito al 2,78% aggregato. Lo [score delle perturbazioni](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/fleurs-perturb-gemma-llama.score.json) e lo [score pulito](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/fleurs-dev30-gemma-llama-q4.score.json) conservano il confronto per ID.

Il WER sulle 60 varianti è 2,92% per Q8 e Redux CPU, 3,19% per Q4, 3,06% per Redux MPS, 1,88% per Whisper e 1,74% per Gemma MLX. I conteggi quasi identici ai puliti non indicano un vantaggio robusto su questo piccolo controllo; il rumore con SNR circa 19 dB e la velocità 1,2× sono condizioni moderate. Per MLX l'allocazione GPU massima è 5,41 GiB, sovrapposta alla memoria unificata e non da sommare al RSS. Il pulito di Whisper e Gemma MLX usa esattamente i 20 base ID comuni alle tre varianti, abbinati ai [rispettivi score dev30 Whisper](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/fleurs-dev30-whisper-q8-mps.score.json) e [Gemma MLX](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/fleurs-dev30-gemma-mlx-4bit.score.json) (non tutte le 30 clip): `fleurs-dev-1510-000150-a3b6ac653b`, `fleurs-dev-1517-000131-ed5db42b56`, `fleurs-dev-1522-000141-4b6df95a94`, `fleurs-dev-1547-000061-aea0b76ebf`, `fleurs-dev-1552-000021-4684e8593a`, `fleurs-dev-1557-000130-b9cf93ba9c`, `fleurs-dev-1561-000263-eb5a357df4`, `fleurs-dev-1565-000032-e5d02b55fb`, `fleurs-dev-1565-000243-acf6813318`, `fleurs-dev-1574-000187-e453c67eba`, `fleurs-dev-1580-000089-a98dfa8708`, `fleurs-dev-1589-000103-f562fb073b`, `fleurs-dev-1595-000056-09dcaefd6b`, `fleurs-dev-1603-000271-f05d6d2943`, `fleurs-dev-1607-000351-a2e6a84d64`, `fleurs-dev-1614-000220-8c96adcdce`, `fleurs-dev-1631-000360-8d98f0b44c`, `fleurs-dev-1632-000198-9ead71be61`, `fleurs-dev-1639-000083-b66f128d13`, `fleurs-dev-1640-000114-f02227ef7a`. I [risultati Q8](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/fleurs-perturb-parakeet-q8.score.json), [Q4](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/fleurs-perturb-parakeet-q4.score.json), [Redux CPU](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/fleurs-perturb-redux-cpu.score.json), [Redux MPS](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/fleurs-perturb-redux-mps.score.json), [Whisper](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/fleurs-perturb-whisper.score.json) e [Gemma MLX](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/fleurs-perturb-gemma-mlx.score.json) conservano ogni variante e il relativo WAV; i tempi delle tre ripetizioni sono riportati separatamente.


## Correzione delle 60 clip perturbate

Tutti gli otto run completano 60/60 clip e 1440 parole di riferimento, senza troncamenti o eco del prompt. Le tre varianti derivano dalle stesse 20 clip: non sono 60 registrazioni indipendenti. “Migliori/peggiori” indica il conteggio WER per clip contro l’ASR iniziale, non la verifica acustica delle singole modifiche.

| Baseline → correttore | Errori baseline | Solo testo: errori / tempo | Audio: errori / WER / tempo | Audio: clip migliori / peggiori | ASR + audio, tempo totale |
|---|---:|---:|---:|---:|---:|
| Q8 → MLX | 42 | 36 / 156,4 s | 31 / 2,15% / 270,1 s | 8 / 3 | 291,4 s |
| Q8 → llama.cpp | 42 | 42 / 198,3 s | 25 / 1,74% / 413,1 s | 14 / 3 | 434,4 s |
| Whisper → MLX | 27 | 20 / 169,9 s | 24 / 1,67% / 254,8 s | 7 / 6 | 556,3 s |
| Whisper → llama.cpp | 27 | 25 / 183,5 s | 18 / 1,25% / 417,5 s | 8 / 1 | 719,1 s |

Rispetto al controllo solo testo, l’audio migliora/peggiora/pareggia rispettivamente 7/3/50 clip con Q8+MLX, 14/3/43 con Q8+llama.cpp, 2/6/52 con Whisper+MLX e 6/1/53 con Whisper+llama.cpp. Quindi il vantaggio dell’audio resta dipendente da sorgente e runtime: Whisper+MLX è peggiore del controllo testuale anche su queste perturbazioni. I [confronti Q8+MLX](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/fleurs-perturb-q8-mlx-audio-correct.score.json), [Q8+llama.cpp](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/fleurs-perturb-q8-llama-audio-correct.score.json), [Whisper+MLX](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/fleurs-perturb-whisper-mlx-audio-correct.score.json) e [Whisper+llama.cpp](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/fleurs-perturb-whisper-llama-audio-correct.score.json) conservano gli intervalli prima/dopo e i controlli.

Il cambio “Mahayano” → “Mahayana” torna sulle varianti del [WAV di sviluppo](/Volumes/UltraDisk/sbobino-asr-spike-20260927/data/corpus-v1/audio/fleurs-dev-1595-000056-09dcaefd6b.wav) e peggiora il riferimento. Q8+llama.cpp cambia inoltre “indagini scientifica” in “indagini scientifiche” sulla [variante rumorosa 0–15,54 s](/Volumes/UltraDisk/sbobino-asr-spike-20260927/data/corpus-v1/audio/fleurs-dev-1565-000032-e5d02b55fb-perturb-noise.wav), passando da due a tre errori: è un esempio da ascoltare di possibile regolarizzazione grammaticale, incompatibile con il prompt se il parlante ha pronunciato la forma originale.

## AMI: controllo inglese far-field

I cinque candidati baseline hanno completato tutti i 49 chunk della riunione ES2002a senza errori di esecuzione. Il WER riguarda esclusivamente 52–652 s e 1290 parole; il tempo riguarda l'intero WAV di 1272,64 s. I parlanti sovrapposti e l'ordine della trascrizione manuale limitano l'interpretazione del WER: questo controllo non entra nelle medie italiane.

| Motore | Errori di parola | WER sul tratto valutabile | Tempo file completo | RSS processo modello | GPU osservata* |
|---|---:|---:|---:|---:|---:|
| Parakeet Q8 MPS | 374 | 28,99% | 42,7 s | 1,85 GiB | — |
| Parakeet Q4 MPS | 373 | 28,91% | 43,9 s | 1,38 GiB | — |
| Redux CPU | 444 | 34,42% | 55,8 s | 1,15 GiB | — |
| Redux MPS | 440 | 34,11% | 29,8 s | 0,74 GiB | 1,25 GiB |
| Whisper large-v3-turbo Q8, CLI MPS | 452 | 35,04% | 228,1 s | 3,30 GiB | — |
| Gemma 4 E4B 4 bit, MLX-VLM | 585 | 45,35% | 239,8 s | 0,93 GiB | 5,38 GiB |
| Gemma 4 E4B Q4_0, llama.cpp | — | Run troncato | 419,6 s | 4,11 GiB | — |

Gli [score Q8](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/ami-o2-parakeet-q8.score.json), [Q4](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/ami-o2-parakeet-q4.score.json), [Redux CPU](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/ami-o2-redux-cpu.score.json), [Redux MPS](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/ami-o2-redux-mps.score.json) e [Whisper](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/ami-o2-whisper.score.json) conservano intervallo valutato, transcript e latenza dei chunk. Whisper impiega 227,9 s di inferenza, RTF 0,179; l'avvio non è separabile nella CLI che ricarica il modello a ogni finestra. Gemma MLX ([run](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/ami-o2-gemma-mlx.json), [score](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/ami-o2-gemma-mlx.score.json)) impiega 10,0 s di avvio e 228,3 s di inferenza, RTF 0,179. Tutti i suoi 49 chunk coprono il file, con 34 overlap non allineati e 3 ripetizioni ambigue. Il valutatore conserva un'eco del prompt nell'intervallo 624–652 s, inclusa nel punteggio.

Anche [llama.cpp](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/ami-o2-gemma-llama.json) produce tutti i 49 chunk, ma la finestra 26–54 s emette 512 token ripetendo «Yeah, yeah, yeah» e termina con `finish_reason=length`. Il run è `truncated`: lo [score](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/ami-o2-gemma-llama.score.json) esclude l'intera registrazione dalla qualità primaria, senza presentare un transcript incompleto come valido. L'avvio dura 13,2 s e l'inferenza 405,4 s; il picco RSS del runner è 0,061 GiB, distinto dai 4,11 GiB del processo modello. Gli output grezzi restano disponibili per la diagnosi.

\* Le allocazioni GPU osservate si sovrappongono alla memoria unificata e non vanno sommate al RSS.

## Correzione AMI: tutti gli otto confronti completati

Ogni run completa l'intero file e tutte le 49 finestre, senza troncamenti o eco esatte del prompt. Qualità: solo 52–652 s, 1290 parole. Tempi: intero WAV da 1272,64 s. Non sono risultati italiani e non rappresentano una valutazione multilingue.

| ASR → correttore | ASR: errori / WER | Solo testo: errori / WER | Con audio: errori / WER | ASR + audio, tempo totale |
|---|---:|---:|---:|---:|
| Parakeet Q8 → MLX | 374 / 28.99% | 393 / 30.47% | 394 / 30.54% | 568.1 s |
| Parakeet Q8 → llama.cpp | 374 / 28.99% | 389 / 30.16% | 394 / 30.54% | 585.4 s |
| Whisper → MLX | 452 / 35.04% | 452 / 35.04% | 479 / 37.13% | 597.4 s |
| Whisper → llama.cpp | 452 / 35.04% | 467 / 36.20% | 448 / 34.73% | 782.0 s |

Gemma peggiora Q8 di 20 errori con entrambi i runtime. Da Whisper, MLX aggiunge 27 errori; llama.cpp ne recupera quattro rispetto alla baseline e 19 rispetto al controllo testuale. Quest'ultimo è un piccolo miglioramento su una sola riunione, con pipeline 782,0 s contro 228,1 s del solo Whisper: non prova un vantaggio generalizzabile. Tutti gli otto run hanno otto errori numerici; AMI non ha un elenco di termini annotati.

| Sorgente / runtime / modo | Avvio | Inferenza | Totale Gemma | RSS modello | GPU MLX* |
|---|---:|---:|---:|---:|---:|
| q8-mlx-testo | 8.1 s | 294.0 s | 304.7 s | 3.53 GiB | 5.15 GiB |
| q8-mlx-audio | 13.0 s | 510.9 s | 525.4 s | 1.17 GiB | 5.49 GiB |
| q8-llama-testo | 3.6 s | 283.2 s | 287.2 s | 5.60 GiB | — |
| q8-llama-audio | 2.1 s | 539.8 s | 542.7 s | 5.78 GiB | — |
| whisper-mlx-testo | 8.4 s | 198.9 s | 207.6 s | 3.96 GiB | 5.14 GiB |
| whisper-mlx-audio | 6.4 s | 362.4 s | 369.3 s | 2.41 GiB | 5.48 GiB |
| whisper-llama-testo | 2.1 s | 252.7 s | 255.1 s | 5.79 GiB | — |
| whisper-llama-audio | 2.0 s | 551.0 s | 553.9 s | 5.71 GiB | — |

* Allocazioni sovrapposte alla memoria unificata: non sommare GPU e RSS. Gli score [Q8 + MLX](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/ami-q8-mlx-audio-correct.score.json), [Q8 + llama.cpp](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/ami-q8-llama-audio-correct.score.json), [Whisper + MLX](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/ami-whisper-mlx-audio-correct.score.json) e [Whisper + llama.cpp](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/ami-whisper-llama-audio-correct.score.json) mantengono S/D/I, CER, punteggiatura, modifiche e intervalli prima/dopo.

## Stabilità: sequenza senza riferimento di 90 minuti

La sequenza `stability-sequence-5400s` dura 5400 s e non ha un riferimento. Tutti i sette motori sono completi. Tutti gli score riportano `success_count=0`, `quality_sample_count=0`, `unscored_count=1` e `unscored_sample_ids=["stability-sequence-5400s"]`. Nella singola riga `samples[]`, `status=unscored`, `reason=reference_missing` e `output_status=ok`: è uno scoring atteso, non un fallimento runtime. La singola registrazione di ciascun run grezzo ha `status=ok`.

| Motore | Tempo totale | Avvio | Inferenza | RTF | RSS processo modello | RSS runner | GPU osservata* | Chunk / copertura | Findings di confine |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Parakeet Q8 MPS | 150,2294 s | 0,3670 s | 149,2890 s | 0,027646 | 1,668 GiB | 0,1945 GiB | — | 208 / 0–5400 s | 92 |
| Parakeet Q4 MPS | 148,7433 s | 0,2621 s | 147,9314 s | 0,027395 | 1,384 GiB | 0,1938 GiB | — | 208 / 0–5400 s | 97 |
| Redux CPU | 187,6912 s | 4,0434 s | 182,1542 s | 0,033732 | 1,206 GiB | 0,2022 GiB | — | 208 / 0–5400 s | 89 |
| Redux MPS | 109,2579 s | 3,5911 s | 104,1539 s | 0,019288 | 0,744 GiB | 0,1943 GiB | 1,261 GiB | 208 / 0–5400 s | 92 |
| Whisper large-v3-turbo Q8, CLI MPS | 951,9238 s | n.d. | 951,2203 s | 0,176152 | 3,306 GiB | 0,1933 GiB | — | 208 / 0–5400 s | 91 |
| Gemma MLX 4 bit | 1578,0383 s | 7,8803 s | 1568,3335 s | 0,290432 | 1,380 GiB | 0,195 GiB | 5,382 GiB | 208 / 0–5400 s | 120 |
| Gemma llama.cpp Q4_0 | 2165,6490 s | 3,574 s | 2159,354 s | 0,399880 | 5,830 GiB | 0,188 GiB | — | 208 / 0–5400 s | 133 |

Ogni run usa finestre da 28 s con passo 26 s: il primo chunk parte da 0 s, l'ultimo termina a 5400 s, tutti i 208 intervalli sono continui e nessun chunk ha testo vuoto o marker di troncamento. I [run Q8](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/stability-o2-parakeet-q8.json) e [score Q8](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/stability-o2-parakeet-q8.score.json), [Q4](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/stability-o2-parakeet-q4.json) e [score Q4](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/stability-o2-parakeet-q4.score.json), [Redux CPU](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/stability-o2-redux-cpu.json) e [score CPU](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/stability-o2-redux-cpu.score.json), [Redux MPS](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/stability-o2-redux-mps.json) e [score MPS](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/stability-o2-redux-mps.score.json), [Whisper](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/stability-o2-whisper.json) e [score Whisper](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/stability-o2-whisper.score.json) riportano tutti il manifest SHA-256 `e08985b5c44a9469dfcdaa0843e80026370bcc594d54f8ad275ea1b7331e65f9`, verificato sul file `corpus-v1/manifest.json`.

\* L'allocazione Metal è un'osservazione separata del driver e si sovrappone alla memoria unificata: non va sommata al RSS.


I due run Gemma completano tutti i 208 chunk con `finish_reason=stop`, nessun chunk vuoto, nessun troncamento e zero eco del prompt rilevate. MLX segnala 113 overlap non allineati e sette ripetizioni ambigue, llama.cpp 128 e cinque. Latenza p50/p95/massimo: MLX 7,43/9,41/10,40 s; llama.cpp 10,26/12,77/13,65 s. I [run MLX](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/stability-o2-gemma-mlx.json) e [llama.cpp](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/stability-o2-gemma-llama.json) documentano copertura, completamento e picchi di memoria. Non è stata acquisita una curva RSS per chunk: i picchi high-water non provano assenza di crescita nel tempo. La memoria GPU MLX è allocazione runtime, mentre quella Redux è osservata dal driver; entrambe si sovrappongono alla memoria unificata.

## Silenzio puro

Il controllo sul silenzio puro è completo per i sei motori principali: WAV mono PCM16 a 16 kHz, 28 s, tutti i campioni a zero, SHA-256 `ab513bd9258f62d957ae2351f15e8c2cae5c1ca257ad3f026f818040821da945`. Parakeet Q8, Redux CPU e Redux MPS restituiscono testo vuoto. Whisper produce «Autore dei sottotitoli e revisione a cura di QTSS». Gemma MLX e llama.cpp emettono entrambi integralmente il prompt diretto che inizia «Trascrivi letteralmente l'audio in italiano». Il riferimento è vuoto: il WER resta non definito, mentre il valutatore segnala un output lessicale inventato per Whisper e per entrambi i Gemma. I [risultati Gemma MLX](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/control-silence-gemma-mlx.json), [llama.cpp](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/control-silence-gemma-llama.json) e [Whisper](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/control-silence-whisper.json) conservano le risposte complete. La verifica indipendente del codice e dei template conferma che il WAV arriva al modello e che le risposte sono token realmente generati: entrambi i runtime contano 62 token di completamento, senza concatenare il prompt alla risposta. Il valutatore ora registra `prompt_echo_intervals` e `prompt_echo_chunk_count` senza rimuovere il testo dalle metriche; i 19 controlli automatici attuali passano. La formulazione del prompt resta una possibile variabile da provare separatamente sullo sviluppo.


## Tre ripetizioni prestazionali

Il sottoinsieme è congelato per misurare velocità e riproducibilità: **cinque clip e appena 108 parole**, non un test di accuratezza generale. I WER di questa tabella non sostituiscono i risultati FLEURS test200 o mTEDx.

Protocollo: tre ripetizioni (`perf-r1`…`perf-r3`) sugli stessi cinque ID FLEURS, durata audio complessiva **72,54 s**. Ogni cella numerica è `mediana [min–max]` tra le tre ripetizioni. Inferenza è la somma dei `samples[].inference_seconds`; RTF = inferenza / 72,54 s. RSS modello e runner restano separati; i valori GPU sono riportati separatamente e non sommati al RSS.

### Configurazioni dirette

| Configurazione | WER / CER | Avvio s | Inferenza s | Elapsed s | RTF inferenza | RSS modello GiB | RSS runner GiB | GPU MLX GiB | GPU MPS driver GiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Gemma Q4 llama.cpp | 2,78% / 0,65% | 3,059 [2,566–9,163] | 27,832 [26,236–28,949] | 31,087 [29,001–38,436] | 0,384 [0,362–0,399] | 5,68 [5,62–5,70] | 0,04 [0,04–0,04] | — | — |
| Gemma 4-bit MLX | 0,00% / 0,00% | 7,163 [6,916–7,476] | 16,354 [14,333–19,792] | 23,827 [22,101–27,398] | 0,225 [0,198–0,273] | 1,79 [1,34–2,06] | 0,03 [0,03–0,03] | 5,35 [5,35–5,35] | — |
| Parakeet Q4 MPS | 2,78% / 0,65% | 0,210 [0,158–0,261] | 1,430 [1,425–1,446] | 1,691 [1,602–1,722] | 0,020 [0,020–0,020] | 1,37 [1,30–1,37] | 0,03 [0,03–0,03] | — | — |
| Parakeet Q8 CPU | 2,78% / 0,65% | 0,106 [0,106–0,156] | 6,570 [6,483–7,083] | 6,700 [6,609–7,273] | 0,091 [0,089–0,098] | 1,10 [1,10–1,11] | 0,03 [0,03–0,03] | — | — |
| Parakeet Q8 MPS | 2,78% / 0,65% | 0,370 [0,262–0,415] | 1,757 [1,602–1,929] | 2,171 [1,903–2,394] | 0,024 [0,022–0,027] | 1,82 [1,77–1,87] | 0,03 [0,03–0,03] | — | — |
| Redux CPU | 0,00% / 0,00% | 3,908 [3,865–4,103] | 2,112 [2,108–2,222] | 6,903 [6,818–6,993] | 0,029 [0,029–0,031] | 1,28 [1,28–1,28] | 0,03 [0,03–0,03] | — | — |
| Redux MPS | 0,00% / 0,00% | 2,810 [2,645–3,016] | 1,288 [1,285–1,304] | 4,683 [4,658–5,074] | 0,018 [0,018–0,018] | 0,74 [0,74–0,74] | 0,03 [0,03–0,03] | — | 1,26 [1,26–1,26] |
| Whisper server Q8 MPS | 0,00% / 0,00% | 0,527 [0,526–1,039] | 11,008 [10,578–11,057] | 11,619 [11,576–11,655] | 0,152 [0,146–0,152] | 1,12 [1,12–1,12] | 0,04 [0,04–0,04] | — | — |
| Whisper CLI Q8 MPS | 0,00% / 0,00% | — | 20,167 [19,678–23,225] | 20,191 [19,703–23,252] | 0,278 [0,271–0,320] | 3,30 [3,30–4,49] | 0,03 [0,03–0,03] | — | — |

### Correzione: 2 sorgenti × 2 runtime × 2 modi

La colonna `ASR+Gemma elapsed` viene da `comparison.asr_plus_gemma_elapsed_seconds` nello score; il WER/CER è quello dello score della correzione.

| Sorgente | Runtime | Modo | WER / CER | Avvio s | Inferenza s | Elapsed s | RTF inferenza | ASR+Gemma elapsed s | RSS modello GiB | RSS runner GiB | GPU MLX GiB | GPU MPS driver GiB |
|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Parakeet Q8 MPS | llama.cpp | audio | 1,85% / 0,32% | 2,057 [2,045–2,553] | 27,743 [26,485–29,518] | 29,996 [28,725–32,280] | 0,382 [0,365–0,407] | 31,899 [31,119–34,452] | 5,78 [5,72–5,80] | 0,04 [0,04–0,04] | — | — |
| Parakeet Q8 MPS | llama.cpp | solo testo | 2,78% / 0,65% | 2,063 [2,056–2,065] | 14,431 [14,398–16,074] | 16,667 [16,656–18,411] | 0,199 [0,198–0,222] | 18,838 [18,559–20,805] | 5,62 [5,56–5,62] | 0,04 [0,04–0,04] | — | — |
| Parakeet Q8 MPS | MLX | audio | 2,78% / 0,65% | 6,260 [5,959–6,287] | 16,621 [16,048–17,395] | 23,200 [22,557–23,706] | 0,229 [0,221–0,240] | 25,371 [24,951–25,609] | 2,55 [2,47–2,61] | 0,03 [0,03–0,03] | 5,37 [5,37–5,37] | — |
| Parakeet Q8 MPS | MLX | solo testo | 3,70% / 0,97% | 6,541 [6,415–8,208] | 11,965 [9,505–13,183] | 18,625 [16,280–22,416] | 0,165 [0,131–0,182] | 21,019 [18,451–24,319] | 2,32 [1,76–2,41] | 0,03 [0,03–0,03] | 5,04 [5,04–5,04] | — |
| Whisper Q8 | llama.cpp | audio | 0,00% / 0,00% | 2,049 [2,034–3,578] | 27,919 [26,521–28,931] | 30,172 [28,770–32,704] | 0,385 [0,366–0,399] | 52,022 [49,874–52,895] | 5,74 [5,65–5,77] | 0,04 [0,04–0,04] | — | — |
| Whisper Q8 | llama.cpp | solo testo | 0,00% / 0,00% | 2,041 [2,034–2,562] | 14,627 [14,029–15,384] | 17,412 [16,249–17,604] | 0,202 [0,193–0,212] | 37,604 [35,952–40,856] | 5,59 [5,56–5,61] | 0,04 [0,04–0,04] | — | — |
| Whisper Q8 | MLX | audio | 0,00% / 0,00% | 6,255 [6,226–6,377] | 17,223 [16,667–19,593] | 23,968 [23,166–26,073] | 0,237 [0,230–0,270] | 46,264 [42,869–47,221] | 2,55 [2,36–2,58] | 0,03 [0,03–0,03] | 5,37 [5,37–5,37] | — |
| Whisper Q8 | MLX | solo testo | 0,00% / 0,00% | 7,654 [7,505–7,862] | 10,008 [9,793–11,066] | 18,109 [17,551–19,019] | 0,138 [0,135–0,153] | 37,811 [37,742–42,271] | 1,71 [1,18–2,13] | 0,03 [0,03–0,03] | 5,04 [5,04–5,04] | — |

### Controlli

- File analizzati: **27 diretti + 24 correzioni = 51** run JSON, con **51** `*.score.json` corrispondenti.
- ID e ordine: **OK** su tutti i 51 run; gli stessi cinque ID coprono 72,54 s.
- Configurazioni: **OK** tra r1/r2/r3; per le correzioni differiscono solo `source_run` e `source_run_sha256`, come atteso.
- Stati: **OK**; 51/51 run hanno cinque righe `status=ok`; 51/51 score hanno 5/5 successi, 5/5 quality sample e 0 non valutati.
- Pairing correzioni: **OK**; i 24 score usano `source_pairing=sha256`, 5 registrazioni appaiate e zero fallimenti/non valutati.
- Riproducibilità: **OK**; i testi finali sono esattamente uguali tra le tre ripetizioni per tutti i 17 gruppi (9 diretti + 8 correzioni).
- Redux: **OK** su 6 run (`worker_ready.ready=true`); conteggi Torch effettivi sempre `torch_num_threads=4`, `torch_num_interop_threads=8`, `cpu_threads_requested=null`. Il numero di worker nativi non è esposto da questi JSON.


MLX ha una mediana di inferenza diretta 16,35 s contro 27,83 s di llama.cpp (circa **1,70× più rapido a caldo**); nel tempo totale il vantaggio scende a 1,30× perché l’avvio MLX è più lungo. Le correzioni audio mostrano lo stesso ordine, includendo anche la baseline nei totali appaiati. La memoria GPU MLX (circa 5,35–5,37 GiB qui) si sovrappone al RSS, perciò il solo RSS non dimostra un risparmio di memoria rispetto a llama.cpp.

I trattini nelle colonne GPU indicano telemetria assente, non memoria zero.

Redux CPU impiega 2,11 s di inferenza contro 6,57 s di Q8 CPU (circa **3,11× a caldo**), ma sui cinque file il tempo totale è 6,90 contro 6,70 s: il suo avvio più lungo assorbe il vantaggio. Redux MPS ha 1,29 s di inferenza contro 1,76 s di Q8 MPS, ma 4,68 contro 2,17 s totali. Il risultato dipende dunque anche dalla durata e dalla residenza del modello. Restano confronti con i default dei due runtime, non con pool nativi di thread equivalenti.

La variabilità osservata include avvio llama.cpp diretto 2,57–9,16 s e RSS Whisper CLI 3,30–4,49 GiB. Tre ripetizioni descrivono questi run, senza stimare la variabilità di lungo periodo. Gli [artefatti per ripetizione](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results) mantengono tempi e picchi grezzi; l’allocazione driver Redux è un’osservazione a fine chunk e non il suo massimo transitorio.

## Passaggi pronti per revisione acustica

Cinque estratti dei WAV pubblici, senza ricampionamento o modifica del segnale, sono in [review-audio](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/review-audio/manifest.json). Il manifest registra origine, intervallo, checksum dell'estratto e `acoustic_review_completed=false`. Il numero di campioni è verificato rispetto agli estremi originali.

| Passaggio | Estratto da ascoltare | Confronto |
|---|---|---|
| IA/Turing, 0–28 s | [Frase iniziale](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/review-audio/mtedx-dev-o7krwo3pvz8-75133c7118-0-28.wav) | Q8 contro MLX: seconda versione della stessa frase aggiunta |
| IA/Turing, 76–84 s | [Confine](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/review-audio/mtedx-dev-o7krwo3pvz8-75133c7118-76-84.wav) | Ripetizione conservata dalla ricomposizione testuale dello spike |
| IA/Turing, 884–886,422 s | [Coda breve](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/review-audio/mtedx-dev-o7krwo3pvz8-75133c7118-884-886.422.wav) | Transcript Q8 vuoto, prompt restituito da llama.cpp |
| Genetica, 26–54 s | [Ripetizioni](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/review-audio/mtedx-test-zlj5srro7p4-a70788d854-26-54.wav) | «sono sono», «non ho avuto non ho avuto» nelle revisioni |
| Genetica, 1040–1045,16 s | [Coda breve](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/review-audio/mtedx-test-zlj5srro7p4-a70788d854-1040-1045.16.wav) | Prompt restituito da llama.cpp |

I JSON appaiati conservano il transcript originale, il controllo solo testo e la versione con audio per ogni finestra. Gli estratti rendono più rapida la verifica; crearli non equivale ad averli ascoltati. Anche i casi positivi sui nomi FLEURS, collegati sopra, richiedono lo stesso controllo letterale.

## Implicazioni per un'app leggera e multilingue

La copertura dichiarata dai modelli va distinta dalla qualità misurata. [Parakeet TDT v3](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3) e [Redux](https://huggingface.co/moondream/parakeet-redux) documentano 25 lingue europee; [Whisper turbo](https://huggingface.co/openai/whisper-large-v3-turbo) offre una copertura più ampia, con prestazioni variabili per lingua. Questo spike verifica italiano e una sola riunione inglese: non valida tutte le lingue, né l'accuratezza uniforme di uno dei motori. I conteggi di lingue generali di Gemma non costituiscono una matrice di accuratezza ASR.

Sul Mac M3, mantenere Parakeet Q8 come scelta per l'italiano e Whisper come candidato per la copertura linguistica più ampia è la direzione operativa: la qualità va validata lingua per lingua. Q4 ha pesi più piccoli; il RSS osservato è inferiore in alcuni run lunghi, ma superiore a Q8 nel test FLEURS residente (1,35 contro 1,25 GiB). Non emerge un guadagno sistematico di qualità, velocità o memoria in tutti i carichi. La lingua selezionata deve arrivare al motore; dove un backend rileva automaticamente la lingua, il parametro del wrapper non dimostra da solo un vincolo effettivo. La qualità su parlato misto richiede verifiche per segmento.

La scelta su altri dispositivi deve usare capacità reali del runtime, memoria disponibile e misure sul dispositivo, conservando una strada CPU e caricando un modello alla volta. Nessuna soglia RAM universale o equivalenza Intel/ARM è stata validata. Redux CPU è promettente per velocità, ma non è ancora un sostituto generale: occorrono x86 reale e una soluzione di distribuzione compatibile col vincolo nativo dell'app. Il modello compatto non implica da solo un runtime leggero: il benchmark include Photon/PyTorch, non soltanto i pesi ternari.

Le directory locali dei modelli Gemma occupano 4,824 GiB (MLX) e 5,199 GiB (GGUF con proiettore), contro 0,876 GiB del solo GGUF Parakeet Q8, 0,629 GiB Q4 e 0,814 GiB dei pesi Whisper turbo Q8. Sono dimensioni dei file misurate, non dimensioni dell'app distribuita né RAM richiesta. Gemma deve quindi restare un download sperimentale separato, se mai verrà proposto. MLX-VLM è più veloce tra le configurazioni Gemma provate su questo Mac, ma la versione provata usa Python; non è stata realizzata o validata una distribuzione nativa dell'app con MLX.

La potenzialità più interessante di Gemma è una seconda lettura di brevi passaggi incerti, con proposte visibili e transcript ASR originale conservato. È un'ipotesi di prodotto da verificare, non una funzione pronta: lo spike corregge tutti i chunk e non ha provato un selettore dei passaggi, una soglia di confidenza affidabile o l'accettazione automatica dei singoli edit. Nomi recuperati e WER migliore su clip brevi non autorizzano modifiche non verificate a numeri, grammatica, ripetizioni o nomi propri. Una revisione selettiva potrebbe limitare costo e superficie degli errori, ma non dimostra che il modello sappia riconoscere quando si sbaglia. Gemma fornisce qui tempi del chunk: alle parole modificate non sono attribuibili automaticamente i timestamp di parola dell'ASR originale.

Il `main` corrente dell'app ha già batching residente e ricomposizione Parakeet con parole/timestamp (`parakeet_cpp.rs`, confronto ai confini). Lo spike usa un allineamento testuale conservativo diverso: le duplicazioni osservate qui non provano un difetto identico nell'app. Il corpus e i controlli sui confini possono essere riusati per verificare la pipeline reale prima di modificare quella ricomposizione.

## Decisioni per il prossimo aggiornamento dell'app

**Gemma ASR: non integrare come terzo motore in questa fase.** Su FLEURS italiano Gemma è meno accurato di Q8, e sui quattro talk il WER resta superiore alle baseline anche senza overlap. AMI aggiunge un peggioramento MLX e un fallimento operativo llama.cpp. Il piccolo vantaggio sulle perturbazioni non compensa costi, peso e comportamento sul silenzio. Lo stress supera 90 minuti in entrambi i runtime, ma completamento non significa qualità né assenza di crescita della memoria.

**Gemma correttore: non attivare automaticamente su tutta la trascrizione.** Il beneficio audio-aware sulle clip brevi è misurabile anche rispetto al solo testo: Q8 + MLX passa da 2,89% a 2,13% WER su 200 clip, contro 2,57% del controllo. Llama usa 199 pair riusciti perché fallisce una clip con un loop. Sui quattro talk di test tutte le combinazioni audio sono peggiori della baseline e del proprio controllo testuale; da Whisper + llama.cpp il cambiamento è quasi nullo. In AMI solo Whisper + llama.cpp migliora lievemente. Sui cinque talk Q8 + MLX costa 1479,0 s contro 110,8 s dell'ASR (13,35 volte), con più errori. Alcuni termini rari vengono recuperati, ma questo non autorizza correzioni di grammatica, numeri o nomi per plausibilità.

Il potenziale da approfondire è **una revisione opzionale di brevi passaggi incerti**, con audio, differenze visibili e originale conservato. Il selettore dei passaggi e l'accettazione degli edit non sono stati provati. Per renderla una funzione occorrono un held-out nuovo, ascolto dei casi discordanti, verifica che gli edit dannosi non aumentino e un budget concreto di latenza/memoria. Tra le configurazioni Gemma provate su questo Mac, MLX è la strada sperimentale più veloce; non è ancora una soluzione di distribuzione nativa dell'app.

**Redux: non sostituire estensivamente Parakeet attuale.** Photon CPU è circa 2,2 volte più rapido di Q8 CPU su FLEURS e sui cinque talk del Mac, ma peggiora il WER su entrambi; anche AMI è peggiore di Q8. Le misure non dimostrano che Photon/PyTorch sia più leggero come app distribuita. È il candidato prioritario per una successiva prova su x86 reale, con verifica di qualità, memoria e distribuzione nativa.

La direzione dell'aggiornamento è mantenere **Parakeet Q8 per l'italiano sul Mac testato e Whisper come candidato per le lingue più ampie**, validando qualità per lingua. Usare modelli scaricabili, un solo modello residente, accelerazione realmente disponibile e fallback CPU. Q4 può ridurre i pesi e talvolta il RSS, ma non è una vittoria sistematica. Nessuna soglia hardware universale o promessa di accuratezza in tutte le lingue è supportata da questo spike. Corpus e controlli sui confini sono riutilizzabili per verificare la pipeline nativa reale; non è stata modificata in questo lavoro.

## Chiusura e riproducibilità

Tutti i 24 run estesi sono completi e verificati, senza ID mancanti o aggiuntivi; il batch sequenziale ha terminato con codice 0. I 19 controlli automatici passano. Restano esplicitamente fuori dalla conclusione: revisione acustica, matrice di qualità per altre lingue, hardware x86 e integrazione/distribuzione nell'app. La conclusione operativa è sufficiente per scegliere cosa non promuovere ora, senza trasformare accordo col riferimento in una certificazione di fedeltà letterale.

Il checkpoint interrotto `mtedx-all-q8-mlx-audio-correct.interrupted-ea5f38176e55.json`, SHA-256 `ea5f38176e559a8899fc9891071ba4427bcbcda1704f3630aafd42eb465307b1`, è conservato ed escluso dai risultati primari: il run completo ripete tutti e cinque i talk. I driver richiedono baseline riuscite e conservano i fallimenti dei candidati come risultati. La [matrice finale verificata](/Volumes/UltraDisk/sbobino-asr-spike-20260927/results/extended-corrections-final-matrix.json) e i [driver/versioni/checksum](/Volumes/UltraDisk/sbobino-asr-spike-20260927/tools) restano separati dalla pipeline dell'app.

# Dispensa di studio da lezioni e fonti selezionate

Disegno di prodotto concordato il 12 ottobre 2026 con `grill-with-docs` (`grilling` e `domain-modeling`). L'utente ha confermato tutte le scelte Q1–Q18. Questa fase definisce il prodotto e documenta le decisioni: non implementa la funzione nell'app.

## Obiettivo e pubblico

Aiutare uno studente universitario a comprendere la lezione e preparare l'esame, ottenendo una dispensa con integrazioni verificabili. Il primo pilot lavora su una lezione dentro un corso; fonti e concetti possono essere condivisi da più lezioni.

Materia, livello e programma rendono esplicito il contesto. Il programma ufficiale stabilisce il perimetro dell'esame, quando disponibile. In sua assenza le lezioni importate definiscono soltanto un perimetro provvisorio; nessuna promessa di preparazione completa all'esame.

La dispensa è distinta dalla trascrizione. La spiegazione del docente rimane riconoscibile; le fonti esterne la possono integrare senza riscrivere ciò che è stato detto. Gli approfondimenti fuori programma restano distinti dal materiale necessario all'esame.

## Flusso dello studente

1. Organizza il corso e indica materia, livello e programma, se disponibile.
2. Trascrive o importa una lezione e può consultare il testo e la registrazione disponibili.
3. Aggiunge materiali pertinenti: inizialmente PDF testuali e trascrizioni di audio/video. Figure, formule e dimostrazioni visive non analizzate sono segnalate; non si dichiara completamente integrata una lezione che dipende da quei materiali.
4. Su richiesta cerca integrazioni in rete. L'app mostra inizialmente 3–5 fonti candidate motivate; lo studente seleziona quelle da utilizzare.
5. Compone una bozza per argomenti, con spiegazione principale, integrazioni attribuite, esempi supportati, approfondimenti, disaccordi e lacune riconoscibili.
6. Controlla i passaggi citati, modifica il testo e conferma la versione da studiare. Non è obbligatoria un'approvazione per ogni paragrafo; la conferma dello studente non certifica automaticamente la correttezza dei contenuti.
7. Studia nell'app oppure esporta PDF/DOCX. Fonti, citazioni, disaccordi e limiti accompagnano anche l'esportazione.

## Fonti, citazioni e disaccordi

Prima materiali del docente, risorse universitarie e testi con autore identificabile. Materia, livello e programma devono motivare la pertinenza; popolarità o similarità non certificano autorevolezza o correttezza.

La ricerca web è avviata dallo studente e usa argomenti/parole chiave visibili. L'intera trascrizione non viene inviata automaticamente al motore di ricerca. Una fonte candidata non entra nella composizione prima della selezione; la presenza nei risultati non garantisce che il suo contenuto sia accessibile.

Ogni integrazione rimanda al passaggio che la sostiene, con pagina o intervallo audio/video quando disponibili. La sola bibliografia non basta. Le affermazioni senza supporto restano domande o lacune, non spiegazioni presentate come certe. Il reperimento di una citazione non è di per sé una verifica che il passaggio sostenga davvero l'affermazione.

Quando una fonte contraddice il docente si conservano entrambe le posizioni e si cerca se dipendano da ipotesi, contesto o aggiornamenti diversi. Nessuna sostituzione silenziosa della spiegazione del docente; un disaccordo non risolto resta visibile.

## Versioni e disponibilità

Ogni citazione è riferita alla versione effettivamente utilizzata della fonte. Una modifica della trascrizione o delle fonti conserva la dispensa precedente, segnala le parti potenzialmente superate e propone un aggiornamento confrontabile. Le modifiche dello studente non vengono sovrascritte silenziosamente.

Se un link scompare, si può consultare la versione già importata quando disponibile; altrimenti la citazione è segnalata come non consultabile. Una fonte eliminata esplicitamente non viene recuperata di nascosto. Le dispense già create non sono riscritte automaticamente: lo studente può aggiornarle o eliminare anche i contributi derivati. Eliminare una fonte non elimina automaticamente gli estratti già presenti nelle dispense salvate.

Decisione architetturale: [ADR 0001 — citazioni versionate](../../docs/adr/0001-citazioni-versionate-dispense.md). Nessuna conservazione illimitata dei materiali o scelta di database è implicita.

## Lingue

La lingua della lezione è l'impostazione iniziale della dispensa ed è modificabile dallo studente. I termini specialistici originali accompagnano la spiegazione tradotta; le citazioni permettono di leggere il passaggio originale distinguendolo dalla traduzione. L'efficacia su ogni coppia di lingue richiede verifica, non deriva automaticamente dal supporto multilingue del modello.

## Ruolo di EmbeddingGemma 2 e dei provider AI

EmbeddingGemma 2 recupera passaggi semanticamente pertinenti tra la lezione e le fonti selezionate; non è un motore di ricerca web né scrive autonomamente la dispensa. La scoperta online e l'acquisizione delle fonti sono operazioni separate dal recupero locale dei passaggi. Un altro modello, tramite il provider locale o remoto già scelto dall'utente, compone la bozza con il materiale recuperato.

Il recupero è locale. I materiali inviati al provider di composizione devono essere espliciti. EmbeddingGemma è facoltativo; nessun modello generativo pesante è obbligatorio. Senza embedding si usa ricerca testuale; senza provider generativo disponibile non si compone una nuova dispensa. Lezioni, fonti già importate e dispense salvate restano consultabili anche senza rete. Le funzioni disponibili dipendono dai materiali e dalle capacità effettivamente presenti: non si finge una ricerca web o una generazione riuscita.

La funzione non corregge automaticamente la trascrizione e non modifica la selezione dei motori ASR. Fonte primaria sulle capacità del modello: [model card Google](https://ai.google.dev/gemma/docs/embeddinggemma/model_card_2?hl=en). I benchmark locali disponibili dimostrano risultati di recupero testuale nel campione provato; non certificano sintesi multimodale, qualità didattica o correttezza delle integrazioni.

## Primo prototipo e verifica

Un caso completo, prima dell'integrazione estensiva: una lezione universitaria, relativo programma e poche fonti pertinenti. Per il test tecnico usare materiali pubblici, conservare riferimenti a passaggi/pagine reali e fissare corpus e configurazioni prima del confronto.

Confrontare il materiale generato dalla sola trascrizione con la dispensa integrata. Per isolare il contributo di EmbeddingGemma, confrontare anche recupero lessicale e semantico sulle stesse fonti, mantenendo invariato il compositore. Una persona competente verifica supporto delle affermazioni, correttezza delle citazioni, integrazioni utili, copertura del perimetro e disaccordi o lacune omessi. L'eventuale beneficio su comprensione e tempo di revisione richiede poi una prova con studenti; lunghezza e gradevolezza non bastano.

Includere almeno una fonte irrilevante, una in disaccordo, una modifica del transcript dopo la composizione e un materiale visivo essenziale non analizzato. Verificare esportazioni, citazioni non più consultabili, conservazione delle revisioni umane e comportamento senza rete/modelli. Misurare tempi e risorse su hardware reale; nessuna promessa multipiattaforma deriva dal solo Mac provato nello spike precedente.

Questo è il perimetro di verifica concordato, non una validazione già eseguita o una decisione di release.

## Collegamento con il codice esistente

- `GLOSSARY.md` resta il glossario unico; contiene termini di trascrizione e studio, senza dettagli implementativi.
- `generate_artifact_pack` e `study_pack_v1` producono già un elaborato da un solo artifact. Il nuovo disegno non coincide con questa capacità e non la dichiara già multi-fonte.
- L'epic `docs/github-backlog/issues/06-student-study-output-pack.md` esclude la sintesi multi-documento dalla V1. Questa idea è un'estensione successiva e non cambia retroattivamente quella V1.
- Le citazioni esterne versionate e le associazioni corso/lezione/fonte richiedono un piano tecnico. Schema di persistenza, adapter di ricerca web e dettagli della UI restano scelte implementative da verificare nel checkout.

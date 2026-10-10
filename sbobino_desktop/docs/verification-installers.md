# Installer di verifica nativi

Il workflow `Verification installers` produce DMG separati per macOS ARM/Intel e NSIS per Windows x86_64. Non crea tag, release, updater firmati o promozioni. Ogni artefatto GitHub contiene installer, log e `verification-manifest.json` con SHA completo del codice, hash di frontend/app/runtime/installer e architettura effettiva. Il nome installer usa lo SHA breve.

Questi pacchetti hanno versione interna 2.0.34 ma contengono il codice del commit indicato nel manifest. I runtime gestiti restano quelli immutabili di v2.0.34. Questa build scarica e verifica il solo archivio speech, registrandone provenienza e hash separatamente; Pyannote e modelli non vengono scaricati o certificati da questa build. Non confonderli con la release pubblica, che contiene un altro snapshot dell’app. I modelli saranno scaricati dal flusso di setup esistente; non sono certificati dalla sola build.

macOS: firma ad hoc verificata anche dopo montaggio del DMG; nessuna notarizzazione. Windows: nessuna firma di produzione richiesta. Gatekeeper e SmartScreen possono bloccare questi pacchetti di test. Gli artefatti updater sono disabilitati; il controllo manuale degli aggiornamenti dell’app resta quello esistente.

## Verifica manuale

Usare un utente OS o una VM di test. Il pacchetto mantiene l’identità `com.sbobino.desktop`: cambiare soltanto HOME o rinominare l’app non isola cronologia, credenziali e WebKit. Non usare `local_cleanroom_smoke.sh` sul profilo personale per questa verifica. Su macOS tenere modelli, cache e log voluminosi su UltraDisk.

1. Scaricare l’artefatto della propria architettura. Verificare l’hash SHA-256 dell’installer contro `artifact_sha256` nel manifest e annotare sistema, hardware e SHA completo.
2. Installare sul profilo isolato. Conservare percorso e hash del processo effettivamente avviato; il pacchetto incorpora `verification-build.json` nelle risorse.
3. Provare setup/download, errore, annullamento e rollback; Rinomina con salvataggio e annullamento; Live → Stop & Save → History → riapertura → export con dati sintetici. Conservare audio esportato e trascrizione.
4. Provare Pyannote reale, diarizzazione e tre veri riavvii dell’app offline. Annotare separatamente credenziali/accessi ai modelli mancanti.
5. Verificare Tab/Shift+Tab, Escape, ripristino del focus, finestre piccole e VoiceOver/Narrator. Salvare schermate nuove identificate da scenario/data/SHA e un contact sheet.
6. Restituire log grezzi, manifest, esiti e riferimenti audio revisionati. Distinguere PASS, FAIL e BLOCKED. Installazione o avvio riusciti non certificano questi flussi.

Le prove prestazionali richiedono stessa macchina, audio e impostazioni tra baseline e candidato: una prova a freddo, tre a caldo e una lunga. Il WER/CER si calcola soltanto rispetto a riferimenti revisionati.

Su Windows `app_sha256` del manifest di build misura l’eseguibile compilato ripristinato da Tauri dopo il bundling. Tauri CLI 2.10 inserisce nel payload NSIS il marcatore `NSS` al posto di `UNK`: lo smoke registra separatamente l’hash reale installato e controlla che ripristinando soltanto quei tre byte si ottenga esattamente l’hash compilato. Qualsiasi altra differenza o firma inattesa fa fallire il controllo. `verified-installed-binary.json` conserva l’identità effettiva. Gli originali dei due smoke falliti restano disponibili.

## Variante UI isolata

Il workflow `verification-packages.yml` accetta `isolated_ui=true` solo per la verifica interattiva: configura `com.sbobino.verification` e WebKit non persistente, conservando codice e dimensioni della finestra. Il manifest registra l'overlay effettivo e distingue questa variante dai pacchetti destinati alla distribuzione. Non installarla sopra Sbobino personale: copiare l'app in una directory di test su UltraDisk.

Prima dell'avvio creare il profilo `~/Library/Application Support/com.sbobino.verification` collegato a una directory nuova su UltraDisk, verificando che non esista già. Avviare il binario direttamente con `SBOBINO_ALLOW_INSECURE_LOCAL_SECRETS=1`: il backend esistente conserva soltanto segreti sintetici nel profilo di test e non consulta il Keychain personale. Non inserire credenziali personali in questa variante. Cambiare soltanto HOME resta insufficiente. Rimuovere il collegamento di test soltanto dopo la chiusura del processo e la conservazione delle prove.

Questi controlli verificano i flussi dell'identico codice applicativo, ma non sostituiscono firma, notarizzazione, migrazione o installazione del pacchetto con identità di produzione. Pyannote runtime e modello sono già distribuiti negli asset pubblici: una prova offline con il pacchetto integro non richiede un token Hugging Face.

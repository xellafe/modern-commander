# Fase 3b "Archivi": design

Data: 2026-10-28 · Stato: rivista dal builder check, da approvare · Riferimenti: spec generale `2026-10-16-modern-commander-design.md` (§6.2, §6.6, §7.2, §9 punto 3), design della fase 3a `2026-10-22-fase-3a-rete-design.md` (provider di cartella), follow-up della fase 2 (archivi nell'ispettore, rischio 7z)

## Perché

La fase 3 dello spec generale ("archivi e rete") è stata divisa in due: la 3a "Rete" è conclusa, questa è la 3b. Si aggancia al provider di cartella della 3a.

L'utente usa ogni giorno ZIP e 7z, raramente TAR.GZ. L'uso più comune: crea in locale uno ZIP per un rilascio, lo carica via SFTP sui server Lepida, e lì uno script suo lo gestisce. Ogni tanto deve correggere uno ZIP già fatto (un file dimenticato, una configurazione da sostituire). Aprire gli archivi direttamente sul server serve per controllare cosa c'è dentro quando qualcosa va storto, e su altri server.

## Obiettivi

1. **Archivi come cartelle.** ZIP, 7z, TAR e TAR.GZ si aprono con Invio o Ctrl+PgDn e si navigano come una cartella: frecce, ricerca rapida, filtro, marcatura, colonne, ordinamento. Backspace dalla radice dell'archivio torna alla cartella che lo contiene, con il cursore sull'archivio.
2. **Ovunque stia l'archivio:** locale, condivisione di rete, SFTP, FTP/FTPS, HTTP.
3. **Estrazione** = copia fuori dall'archivio: F5 verso un altro gruppo (locale o remoto) con la coda, i conflitti, l'avanzamento e l'annullamento di oggi. **Alt+F9** estrae tutto l'archivio sotto il cursore.
4. **Creazione di ZIP (Alt+F5)** dai file selezionati, nella cartella dell'altro gruppo, anche se quel gruppo è una scheda remota.
5. **Modifica di ZIP**, in locale e sul remoto: aggiunta (F5/F6 verso la scheda dell'archivio), eliminazione (F8), nuova cartella (F7), rinomina (Shift+F6), F4 su una voce con salvataggio che sostituisce quella voce.
6. **F3 e ispettore** sulle voci di un archivio.
7. **7z senza rischio di chiusura dell'app** per un archivio costruito ad arte (vedi "Protezione 7z").

## Non obiettivi

- **RAR**: escluso dall'utente nella 3a.
- **Creazione di 7z, TAR, TAR.GZ e modifica di 7z/TAR/TAR.GZ**: sola lettura. Per creare un 7z l'utente usa il menu contestuale di Windows. *(Decisione dell'utente per il 7z; TAR e TAR.GZ in sola lettura per lo scarso uso.)*
- **Archivi dentro archivi** (uno ZIP dentro uno ZIP): non si aprono come cartella; si estraggono e si aprono. *(Scelta del progettista.)*
- **Archivi cifrati** (ZIP con password, 7z con AES): una voce cifrata dà `UNSUPPORTED`, l'elenco resta navigabile dove l'intestazione è in chiaro. *(Scelta del progettista: non serve ai rilasci.)*
- **Ricerca dentro gli archivi**: fase 4 (Alt+F7).
- **Drag & drop OLE verso Explorer** da una scheda archivio e menu contestuale nativo: come per le schede remote della 3a, c'è il menu dell'app; il drag tra gruppi dell'app funziona con la semantica di F5.
- **Elevazione UAC** per estrarre o modificare un archivio in una cartella protetta: non c'è, come per i trasferimenti tra provider della 3a. Si ha `ACCESS_DENIED`.
- **Watcher** sulla scheda archivio: si aggiorna con Ctrl+R e dopo le operazioni fatte dall'app.

## Anti-obiettivi (falliscono anche se "funzionano")

- Un archivio modificato lasciato a metà o troncato, oppure l'originale perso se la modifica fallisce o viene annullata.
- Un'estrazione che scrive fuori dalla cartella di destinazione (voci con `..`, percorsi assoluti, lettere di unità) o che rinomina in silenzio un nome non valido su Windows.
- L'app chiusa (OOM con `panic = "abort"`) da un archivio costruito ad arte, aperto, elencato o estratto.
- Un archivio da centinaia di MB sul server scaricato per intero solo per vederne l'elenco o estrarne un file (vale per ZIP e 7z; il TAR.GZ remoto si scarica, vedi sotto).
- La UI bloccata mentre si legge l'indice di un archivio grande o remoto.
- Regressioni sul locale e sul remoto: coda, conflitti, elevazione, prestazioni e test delle fasi 1, 2 e 3a restano come sono.

## Approcci considerati

- **A. Archivio come provider di cartella (scelto dall'utente).** `Loc` guadagna una terza forma, "dentro l'archivio X al percorso Y", dove X è a sua volta una `Loc` locale o remota. Un `ArchiveProvider` implementa il `Provider` della 3a e legge l'archivio attraverso il provider di X.
  - Pro: estrazione = copia tra provider che esiste già (`vfs/transfer.rs`), quindi conflitti, coda, avanzamento e annullamento senza lavoro nuovo. Sul remoto si leggono a blocchi solo l'indice e le voci che servono.
  - Contro: tocca `Loc` e ogni `match` su di essa (coda, conflitti, plan). La modifica degli ZIP non entra nel `writer` per singolo file del provider: va fatta come job a sé che riscrive l'archivio una volta per lotto.
- **B. 7-Zip esterno (`7z.exe`).** Copre tutti i formati e isola il parsing in un altro processo. Scartato: dipendenza da installare, licenza LGPL, e sul remoto non legge a blocchi (servirebbe scaricare l'archivio intero).
- **C. Comandi separati per gli archivi, fuori dalla coda.** Scartato per lo stesso motivo dell'approccio equivalente nella 3a: duplica conflitti e coda.

## Cosa è deciso (il come)

### Posizioni

- **Nuova forma di `Loc`:** archivio + percorso interno. Il contenitore è una `Loc` locale o remota, mai un'altra posizione d'archivio (niente annidamento). Il percorso interno è POSIX, normalizzato come quello remoto (`/` iniziale, niente `/` doppi o finali, `..` risolti senza uscire dalla radice dell'archivio).
- **Formato dall'estensione del contenitore**, come `archive::format_of` di oggi (`.zip`, `.7z`, `.tar`, `.tar.gz`, `.tgz`). Un file con estensione giusta ma contenuto non valido dà `CORRUPT_ARCHIVE` all'apertura.
- **Forma testuale.** Il frontend, la hotlist e la cronologia usano posizioni come stringhe: serve una forma testuale dell'archivio che si analizzi senza ambiguità anche quando il contenitore è remoto e quando nomi di file contengono caratteri insoliti. *(Forma esatta lasciata al builder, con questo vincolo e con test di andata e ritorno.)*
- **Barra del percorso:** i segmenti del contenitore, poi il nome dell'archivio con un'icona d'archivio, poi i segmenti interni, tutti cliccabili.
- **Hotlist, Preferiti, cronologia (Alt+←/→, Alt+↓):** accettano posizioni d'archivio. *(Scelta del progettista: viene gratis dalla forma testuale.)*

### Provider d'archivio (backend)

- **Lettura del contenitore** sempre attraverso il provider del contenitore (`LocalProvider` o remoto), con un adattatore `Read + Seek` sopra `read_at`. Sul remoto usa una sessione `Transfer` per estrazioni e modifiche, `Browse` per l'elenco.
- **Indice in cache.** L'indice letto (elenco di voci con offset/metodo) resta in memoria per il contenitore, legato a dimensione e data di modifica del contenitore: se cambiano, si rilegge. Una voce si legge senza rianalizzare l'archivio. Limite al numero di indici in cache e alla loro memoria. *(Valori al builder.)*
- **Cartelle implicite.** Molti ZIP non hanno voci per le cartelle: le cartelle intermedie si ricavano dai percorsi delle voci.
- **Metadati:** nome, cartella sì/no, dimensione non compressa, data di modifica (`Caps.precise_mtime` = falso per gli ZIP senza data estesa: le date DOS sono a 2 s e in ora locale), niente permessi POSIX.
  - Oggi `dos_to_ms` (`archive/list.rs`) legge le date DOS come UTC: va corretto in ora locale, altrimenti i file estratti e "più recente" sbagliano di 1–2 ore. Se c'è il campo esteso con la data Unix (UT, `0x5455`) o NTFS (`0x000a`), si usa quello. *(La colonna "Compresso" o il rapporto di compressione non servono; si aggiunge se richiesto.)*
- **Separatore `\` nei nomi ZIP** (strumenti Windows vecchi): si tratta come separatore di cartelle, come fanno 7-Zip ed Explorer e come fa già `archive/list.rs`. La validazione dei nomi avviene dopo questa conversione. *(Scelta del progettista.)*
- **`Caps.writable`** vero solo per gli ZIP il cui contenitore è scrivibile; `ranged_read` vero per tutti (vedi sotto per il costo).
- **Contenitore senza letture a salti** (server HTTP che rifiuta `Range`, `ranged_read` falso): l'archivio si scarica nel temporaneo come il TAR.GZ remoto. *(Scelta del progettista.)*
- **`read_at`** su una voce decomprime dall'inizio della voce fino all'offset: va bene per l'ispettore (prima parte del file), non per scorrere un file grande. Per questo F3 estrae prima in un temporaneo (sotto).
- **Formati di lettura:**
  - **ZIP**: Store e Deflate obbligatori, Deflate64, BZip2, Zstd, LZMA se il crate `zip` li offre senza dipendenze C problematiche (licenze come da spec generale). ZIP64. Nomi in UTF-8 o, senza il flag UTF-8, in CP437. Oggi `zip` e `sevenz-rust2` sono compilati con `default-features = false` (l'ispettore legge solo l'indice): le funzioni di decompressione e compressione vanno attivate.
  - **7z**: con il crate in uso (`sevenz-rust2`) e la protezione qui sotto. Archivi solidi supportati.
- **Estrazione di molte voci da archivi a flusso** (7z solido, TAR, TAR.GZ). Leggere ogni voce dall'inizio del blocco darebbe un costo quadratico: Alt+F9 su un 7z solido da centinaia di MB diventerebbe inutilizzabile. Quindi:
  - la pianificazione del job ordina le voci di uno stesso archivio nell'ordine in cui stanno nell'archivio;
  - il provider tiene aperto il decoder dell'ultima voce letta e, se la voce chiesta dopo sta più avanti nello stesso blocco, prosegue da lì invece di ricominciare.

  Così l'estrazione resta la copia tra provider a voce singola (conflitti, avanzamento e annullamento invariati) e un archivio si decomprime una volta sola. *(Scelta del progettista; il meccanismo esatto è del builder, il requisito è: un Alt+F9 decomprime ogni blocco al più una volta.)*
  - **TAR** e **TAR.GZ**: si riusa il parser manuale di `archive/tar_walk.rs` (nato per l'ispettore per non avere OOM), con le correzioni elencate nel follow-up della fase 2 che toccano la correttezza dell'estrazione: `MultiGzDecoder`, TAR troncato dentro i dati di una voce = `CORRUPT_ARCHIVE`, `L`/PAX `path` vuoto = errore.
  - **TAR remoto:** si legge a salti saltando i dati delle voci (gli header da 512 byte bastano per l'elenco).
  - **TAR.GZ remoto:** il gzip non si legge a salti, quindi si scarica per intero in un temporaneo dell'app (quello della 3a, che si svuota alla chiusura e all'avvio) con avanzamento e annullamento, e si aprono da lì. *(Scelta del progettista: l'utente li usa di rado.)*
- **Collegamenti simbolici nelle voci** (TAR, ZIP con attributi Unix): non si estraggono; si saltano con un avviso nell'elenco degli errori del job. *(Scelta del progettista, coerente con i collegamenti a cartella sul remoto della 3a.)*

### Protezione 7z

Il rischio è descritto nel follow-up della fase 2, ma in parte è superato: in `sevenz-rust2` 0.24.1 la lettura dell'intestazione codificata è già pigra e i conteggi sono limitati. Resta il dizionario dei decoder (LZMA/LZMA2) senza limite di memoria. Prima di creare un decoder (per l'intestazione codificata o per un blocco di dati):

- si leggono le proprietà del coder;
- si rifiuta con un errore tradotto (nuovo codice, es. `ARCHIVE_TOO_DEMANDING`, o `UNSUPPORTED` con dettaglio) se il dizionario supera **256 MiB**.

*(Scelta del progettista, con l'accordo dell'utente sulla direzione. Soglia: i 7z normali usano dizionari fino a 64 MiB, "Ultra" di 7-Zip fino a 192 MiB; un rifiuto a torto è meglio della chiusura dell'app. Alternativa più robusta, scartata per costo: parsing in un processo figlio con limite di memoria, sul modello dell'helper elevato.)*

Con la protezione, il 7z nell'ispettore può tornare a elencarsi da solo come ZIP e TAR. *(Da decidere dopo aver verificato che il controllo copre tutti i percorsi che allocano; altrimenti resta il pulsante "Mostra contenuto".)*

### Estrazione

- **F5 / drag** da una scheda archivio a qualunque altra scheda: è la copia tra provider della 3a (`vfs/transfer.rs`), con temporaneo `.mcpart` e rinomina, conflitti, avanzamento e annullamento. La data di modifica della voce si riporta sul file estratto.
- **F6 da un archivio** (sposta = estrai e poi elimina dall'archivio): solo da ZIP scrivibili; altrove disattivato con un avviso tradotto. L'eliminazione dall'archivio avviene una volta sola alla fine, e solo per le voci estratte con successo.
- **Alt+F9** su un archivio (in una scheda normale o remota) apre il dialogo di copia con destinazione l'altro gruppo e, come proposta, una sottocartella col nome dell'archivio senza estensione. *(Scelta del progettista, come in 7-Zip "Estrai in <nome>\"; il dialogo permette di toglierla.)*
- **Sicurezza dei nomi.** Ogni nome di voce si valida prima di scrivere: niente `..` che risale, percorsi assoluti, lettere di unità, componenti vuoti; un nome non valido su Windows (`:`, `*`, `?`, nomi riservati, spazio o punto finale) dà un errore per quella voce, nessuna rinomina silenziosa. Si riusa `paths::invalid_windows_name` come nella 3a.
- **Voci duplicate** nello stesso archivio (stesso percorso due volte, capita negli ZIP): vince l'ultima, come fanno 7-Zip ed Explorer. *(Scelta del progettista.)*

### Creazione di ZIP (Alt+F5)

- **Dialogo:** nome dell'archivio (default: nome dell'elemento se ce n'è uno solo, altrimenti nome della cartella corrente, più `.zip`), destinazione = cartella dell'altro gruppo, modificabile come nel dialogo di copia. Nessuna opzione di livello di compressione o di formato in v1. *(Scelta del progettista: Deflate di default; si aggiunge "senza compressione" se richiesto.)*
- **Contenuto:** gli elementi selezionati con percorsi relativi alla cartella corrente, cartelle ricorsive comprese le vuote, nomi in UTF-8 con il flag UTF-8, ZIP64 automatico oltre i limiti dello ZIP classico, data di modifica di ogni file.
- **Destinazione esistente:** conflitto con Sovrascrivi, Rinomina, Annulla. *(Scelta del progettista: "aggiungi all'archivio esistente" c'è già con F5 verso la scheda dell'archivio.)*
- **Esecuzione:** job della coda con avanzamento per file e totale, pausa e annullamento.
  - destinazione locale: si scrive `<nome>.<8 cifre>.mcpart` accanto alla destinazione e lo si rinomina alla fine;
  - destinazione remota: si compone lo ZIP nel temporaneo locale dell'app e lo si carica, nello stesso job, con la stessa sequenza della modifica di ZIP (temporaneo remoto e rinomina; su server senza sostituzione atomica, eliminazione e rinomina dopo il caricamento completo). *(Scelta del progettista, estende la decisione dell'utente al caso Sovrascrivi.)*
  - annullamento o errore: si eliminano i temporanei, la destinazione resta com'era.
- **Sorgenti** locali o remote (ZIP di una cartella sul server): si leggono attraverso il loro provider. *(Viene gratis dall'approccio A.)*
- I comandi Alt+F5 (`cm_PackFiles`), Alt+F9 (`cm_UnpackFiles`) e Ctrl+PgDn (apri archivio) non esistono ancora: vanno registrati nel registro dei comandi e in `DEFAULT_KEYMAP` (`src/commands/tcKeymap.ts`), con i nomi e i tasti di fabbrica di TC.

### Modifica di ZIP

- **Operazioni:** aggiunta (F5/F6 o drag verso la scheda dell'archivio, anche da un altro archivio o dal remoto), eliminazione (F8, sempre definitiva: non c'è cestino dentro un archivio, il dialogo lo dice), nuova cartella (F7, voce cartella esplicita), rinomina (Shift+F6, anche di una cartella con tutto il contenuto), F4 su una voce.
- **Un job per lotto.** Tutte le modifiche di un'operazione si applicano riscrivendo l'archivio una volta:
  - si scrive un nuovo ZIP temporaneo: le voci invariate si copiano già compresse, senza ricomprimerle; quelle nuove o sostituite si comprimono;
  - contenitore locale: temporaneo `.mcpart` accanto all'archivio, poi rinomina sull'originale;
  - contenitore remoto: temporaneo nella cartella temporanea locale, poi caricamento su un temporaneo remoto `<nome>.<8 cifre>.mcpart` accanto all'archivio, poi rinomina sull'originale;
  - **server senza sostituzione atomica** (`Caps.atomic_replace` falso: FTP, SFTP senza `posix-rename`): dopo il caricamento completo del temporaneo remoto si elimina l'originale e si rinomina il temporaneo. Niente scrittura diretta sull'archivio e niente conferma. Se la rinomina fallisce dopo l'eliminazione, l'errore del job indica il nome del temporaneo, che contiene l'archivio completo. *(Decisione dell'utente: per uno ZIP di rilascio un upload interrotto non deve lasciare l'archivio troncato. Vale solo per la modifica degli ZIP; F4 e le copie della 3a restano come sono.)* Se il temporaneo remoto non si può creare (cartella non scrivibile), la modifica fallisce con `ACCESS_DENIED` e l'originale resta intatto;
  - errore o annullamento: si eliminano i temporanei, l'originale resta intatto.
- **Conflitti** dentro l'archivio con la logica di oggi (sovrascrivi, salta, rinomina, più recente, più grande, per tutti), decisi prima di riscrivere.
- **Concorrenza.** Un solo job di modifica per archivio alla volta (la chiave di scheduling della coda include il contenitore). Se il contenitore è cambiato fuori dall'app dall'apertura (dimensione o data diverse), il job si ferma con il conflitto "modificato da altri" invece di sovrascrivere. *(Scelta del progettista, sulla linea di F4 nella 3a.)*
- **F4 su una voce:** si estrae nel temporaneo, si apre nell'editor interno (limite di dimensione di oggi); Ctrl+S sostituisce quella voce con un job di modifica come sopra. Su archivi in sola lettura (7z, TAR) l'editor apre la voce in sola lettura. L'editor esterno non si usa per le voci d'archivio, come per il remoto nella 3a.

### Viewer e ispettore

- **F3 su una voce:** si estrae nel temporaneo dell'app con avanzamento e annullamento, poi si apre come file locale con tutte le modalità del viewer. *(Scelta del progettista: `read_at` dentro una voce compressa costa quanto decomprimere fino all'offset; con un file temporaneo il viewer resta quello di oggi.)*
- **Ispettore su una voce:** metadati e anteprima testuale dei primi KB (con `read_at` dall'inizio della voce). Niente hash.
- **Ispettore su un archivio remoto:** elenco del contenuto per ZIP e 7z (letture a blocchi); per TAR.GZ remoto niente elenco automatico (richiederebbe il download).

### Interfaccia

- **Scheda archivio:** si comporta come una scheda normale. Icona d'archivio nella scheda e nella barra del percorso.
- **Menu col tasto destro:** menu dell'app come per le schede remote: Vedi, Modifica, Copia (estrai), Sposta, Rinomina, Elimina, Nuova cartella, Aggiorna, Copia percorso; nelle schede in sola lettura le voci di scrittura sono disattivate.
- **Comandi in sola lettura** (7z, TAR, TAR.GZ, contenitore non scrivibile, contenitore HTTP): F6, F7, F8, Shift+F6, F4 in scrittura disattivati con un avviso tradotto, come per HTTP nella 3a.
- **Errori tradotti:** `CORRUPT_ARCHIVE` esistente, `UNSUPPORTED` (metodo di compressione, cifratura), il nuovo errore della protezione 7z, nome di voce non valido.

## Lasciato al builder

- Forma testuale della posizione d'archivio (vincoli sopra) e forma esatta della nuova variante di `Loc`.
- Come il job di modifica dello ZIP si inserisce nella coda (nuovo `kind` del job o executor dedicato) e come si raggruppano in un solo job le operazioni F5/F6/F7/F8/Shift+F6 verso lo stesso archivio.
- Adattatore `Read + Seek` sopra `read_at`, dimensione dei blocchi e cache delle letture sul remoto.
- Limiti della cache degli indici.
- Funzioni del crate `zip` effettivamente disponibili nella versione installata (copia raw delle voci, metodi, ZIP64, CP437), verificate sulla sua documentazione; dipendenze nuove solo con `cargo add` e licenze permissive.
- Lettura delle proprietà dei coder 7z per la protezione (parser proprio o API del crate, se ne espone una prima di allocare).
- Strategia di test. Minimo: test automatici in Rust per la forma testuale di `Loc` (andata e ritorno), la validazione dei nomi in estrazione, la riscrittura dello ZIP (originale intatto su errore e annullamento, voci invariate identiche byte per byte), la protezione 7z con archivi costruiti ad arte, la creazione con destinazione remota sul provider in memoria (`vfs/mem.rs`).

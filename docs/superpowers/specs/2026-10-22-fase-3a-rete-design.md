# Fase 3a "Rete": design

Data: 2026-10-22 · Stato: rivista dal builder check, da approvare · Riferimenti: spec generale `2026-10-16-modern-commander-design.md` (§6.2, §6.7, §7.2, §9 punto 3), follow-up della fase 2 (CSP)

## Perché

L'utente lavora ogni giorno su condivisioni di rete raggiungibili in VPN (`\\vm645srv\...`). Ogni tanto si collega a server remoti per leggere log applicativi e per caricare o scaricare file, per i rilasci e non solo. Oggi per questo usa WinSCP, che non gli permette di modificare un file sul server: scarica, modifica e ricarica. Alcuni log applicativi li legge da un browser, tramite l'elenco delle cartelle di un server web protetto da utente e password.

La fase 3 dello spec generale prevede "archivi e rete". L'utente usa molto di più la rete, quindi la fase 3 si divide in:
- **3a "Rete"**, questo documento;
- **3b "Archivi"**, da progettare dopo, che si aggancia alla stessa base.

## Obiettivi

1. **Condivisioni Windows.** Restano come oggi, con percorsi UNC. In più, `\\server` da solo mostra l'elenco delle condivisioni del server, come fa Explorer.
2. **SFTP**, con utente e password. Si possono:
   - navigare le cartelle;
   - leggere i file a blocchi con F3, anche log da GB;
   - modificare con F4 e salvare direttamente sul server;
   - copiare e spostare con F5/F6 nei due sensi tra una scheda locale e una remota, e tra due schede remote;
   - creare cartelle (F7), eliminare (F8) e rinominare.
3. **FTP e FTPS:** le stesse funzioni di SFTP.
4. **Elenco cartelle dei server web (HTTP/HTTPS), in sola lettura:** navigare, F3 a blocchi, F5 verso il locale. Autenticazione HTTP con utente e password.
5. **Gestore delle connessioni:**
   - si apre con Ctrl+F, come in TC;
   - le connessioni sono salvate con nome, protocollo, host, porta, utente e cartella iniziale;
   - le password stanno solo nel **Windows Credential Manager**.
6. **Impronta dei server SSH:** alla prima connessione si chiede di confermarla e poi si ricorda. Se l'impronta cambia, la connessione si blocca con un avviso chiaro.
7. **Importazione dei siti di WinSCP**, senza password, con anteprima di quello che verrà importato.
8. **Content Security Policy** nella WebView, al posto di `csp: null`. È il follow-up di sicurezza della fase 2, che diventa necessario ora che arrivano contenuti remoti.

## Non obiettivi

- **Già escluse, con issue aperte:**
  - chiavi SSH e agente (Pageant, OpenSSH): [#1](https://github.com/xellafe/modern-commander/issues/1);
  - WebDAV: [#2](https://github.com/xellafe/modern-commander/issues/2).
- **Escluse e scartate dall'utente:** SCP (lo copre SFTP), RAR, scrittura sulle sorgenti HTTP.
- **Archivi come cartelle:** fase 3b.
- **Watcher sulle sorgenti remote:** si aggiorna con Ctrl+R e dopo le operazioni fatte dall'app.
- **Su una scheda remota non ci sono le integrazioni della shell di Windows:**
  - menu contestuale nativo e Proprietà;
  - drag & drop OLE verso Explorer;
  - apertura col programma associato;
  - miniature della shell.

  Il drag tra gruppi dell'app invece funziona, con la stessa semantica di F5. *(Scelta del progettista: lo spec generale prevede il drag verso Explorer con download in una cartella temporanea; si rimanda per tenere la 3a contenuta.)*
- **Altre funzioni che sulle schede remote non ci sono:**
  - modifica dei permessi (chmod) e del proprietario;
  - terminale SSH: Ctrl+ò su una scheda remota apre il terminale locale come oggi, nella cartella dell'ultima scheda locale attiva;
  - hash, vista miniature e salto rapido Ctrl+P con scansione.
- **Ripresa di un trasferimento interrotto** dal punto in cui si era fermato: v1 ricomincia il file da capo.

## Anti-obiettivi (falliscono anche se "funzionano")

- Una password scritta in un file di configurazione, in un log o in un messaggio d'errore, oppure passata in chiaro al frontend dopo il salvataggio.
- Un file sul server lasciato a metà, o un salvataggio che sovrascrive in silenzio una modifica fatta da altri.
- La UI bloccata da un server lento o irraggiungibile: ogni operazione di rete è asincrona, annullabile e ha un timeout.
- Un log da GB scaricato per intero solo per guardarne la fine.
- Regressioni sul locale. La coda, i conflitti, l'elevazione UAC e le prestazioni della fase 1 e della fase 2 restano come sono, con i loro test.
- La connessione a un server SSH la cui impronta è cambiata senza che l'utente l'abbia accettato esplicitamente.

## Approcci considerati

- **A. Provider di cartella unico (scelto dall'utente).** Un'interfaccia nel backend, con un'implementazione per sorgente:
  - elenco, stat, lettura e scrittura a blocchi, rinomina, eliminazione, creazione di cartelle;
  - implementazioni: locale, SFTP, FTP/FTPS, HTTP, e nella 3b gli archivi.

  La coda sceglie il provider per job. Tra due provider diversi si copia leggendo dall'uno e scrivendo nell'altro.
  - Pro: un solo motore per conflitti, pausa e annullamento, e la 3b si aggancia senza altro lavoro strutturale.
  - Contro: il cambiamento all'inizio è più invasivo. Tocca pianificazione e conflitti della coda, che oggi leggono il disco direttamente, e una dozzina di comandi del backend.
- **B. Remoto separato, locale intatto:** comandi e job di carica e scarica propri. Scartato perché duplica conflitti e coda, e ogni sorgente nuova sarebbe un altro innesto.
- **C. Montare i server come unità** con un driver esterno (WinFsp e rclone). Scartato per diversi motivi: servirebbero un'installazione con privilegi di amministratore e un programma esterno, le prestazioni sui log grandi sarebbero peggiori, le password finirebbero fuori dal Credential Manager e non ci sarebbe controllo su conflitti ed errori.

## Cosa è deciso (il come)

### Posizioni

- **Sorgenti remote.** La posizione di una scheda diventa un indirizzo con schema, sul modello dello pseudo-percorso `drives:` che esiste già: `<schema>://<id connessione>/<percorso>`.
  - Gli schemi sono `sftp`, `ftp`, `ftps`, `http`, `https`.
  - Il percorso è in stile POSIX (`/var/log/app`).
  - L'id è quello della connessione salvata, non l'host: credenziali, impronta e nome mostrato si ritrovano dall'id.
  - Le posizioni locali restano percorsi Windows come oggi.
- **Barra del percorso.** Mostra il nome della connessione seguito dai segmenti del percorso, cliccabili come oggi.
- **Indirizzo scritto a mano** (`sftp://server/var/log`, `https://server/logs/`). Dopo `://` il backend accetta solo un id di connessione salvata. Un indirizzo con un host vero si risolve nel frontend:
  - se esiste una connessione salvata con quello schema e quell'host (e con quella porta e quell'utente, se indicati), si apre quella;
  - altrimenti si apre il gestore delle connessioni con una connessione nuova già compilata, da salvare prima di collegarsi.

  Non esistono connessioni temporanee senza id. *(Scelta del progettista: password e impronta hanno sempre un id a cui legarsi.)*
- **Elenco delle condivisioni.** `\\server` è una posizione del provider locale: si elencano le condivisioni disco del server (`NetShareEnum`). Quelle che finiscono con `$` compaiono solo con "mostra nascosti". Da qui si entra come in una cartella.

### Provider di cartella (backend)

- **Interfaccia.** Un'unica interfaccia, usata da lista, coda, viewer, editor e ispettore.
  - Operazioni: elenco di una cartella, stat di un elemento, lettura di un intervallo di byte, scrittura a stream, rinomina, eliminazione, creazione di cartelle. Le operazioni lunghe sono annullabili, con il registro di annullamento che esiste già.
  - Metadati di un elemento: nome, è cartella, è collegamento, dimensione, data di modifica, e permessi POSIX dove ci sono.
- **Provider locale.** Avvolge il codice di oggi senza riscriverlo: `list_dir` di `fs/listing.rs`, `read_chunk` di `fs/content.rs` e l'`Executor` della coda, con `LocalExecutor` e l'elevazione UAC, che resta solo per il locale.
- **Instradamento.** Comandi e coda scelgono il provider dallo schema della posizione. Un percorso Windows non passa mai per `to_extended` o `normalize_lexical` se appartiene a una sorgente remota.
- **Coda.**
  - Pianificazione dei job (`ops/plan.rs`), controlli del runner (`ops/runner.rs`) e conflitti (`ops/conflict.rs`) passano per il provider, al posto delle chiamate dirette a `std::fs`.
  - Un job ha sorgenti e destinazione come posizioni, non come `PathBuf`.
  - La chiave di scheduling resta "un job alla volta per volume" in locale. Per le sorgenti remote diventa "un job alla volta per connessione".
- **Copia tra provider diversi** (download, upload, da remoto a remoto):
  - lettura a blocchi dalla sorgente, scrittura sulla destinazione con un nome temporaneo (`<nome>.<8 cifre esadecimali>.mcpart`, come per l'editor), poi rinomina sul nome definitivo;
  - avanzamento, pausa e annullamento come le copie locali; un annullamento elimina il temporaneo;
  - la data di modifica della sorgente si riporta sulla destinazione, dove il protocollo lo permette.
- **Scrittura sicura sul remoto e quando non si può.** Questo vale per F4 Ctrl+S e per Sovrascrivi nelle copie verso il remoto.
  - Si scrive un temporaneo nella stessa cartella e poi lo si rinomina sopra l'originale: in SFTP con l'estensione `posix-rename@openssh.com` se c'è, in FTP con `RNFR`/`RNTO`.
  - Dopo la rinomina si riapplicano i permessi POSIX originali, dove il protocollo lo permette. Il proprietario resta l'utente con cui ci si collega.
  - **Fallback, deciso dall'utente.** In alcuni casi la scrittura sicura non è possibile: la cartella non è scrivibile ma il file sì, il server non rinomina sopra un file esistente, oppure manca l'estensione `posix-rename`. Allora si chiede conferma con un avviso tradotto: "questo server non permette il salvataggio sicuro: se la connessione cade il file può restare incompleto. Salvo lo stesso?". Se l'utente conferma, si scrive direttamente sul file. Non si procede mai senza conferma.
  - Nella coda la conferma si dà una volta per job, con "per tutti".
- **Conflitti.** Stessa logica di oggi (sovrascrivi, salta, rinomina, più recente, più grande, per tutti). "Stesso file" si decide sulla posizione, senza aprire il file.
- **Collegamenti simbolici sul remoto.**
  - Un collegamento a file si copia come il file a cui punta.
  - Un collegamento a cartella non si segue: si salta, con un avviso nell'elenco degli errori del job, come oggi in locale (`UNSUPPORTED` sui reparse point). Così non nascono cicli né copie doppie (`current -> releases/...`).

  *(Scelta del progettista.)*
- **Sessioni e timeout.**
  - Una sessione per connessione aperta per navigare. I trasferimenti usano canali o sessioni a parte, così la navigazione non aspetta un upload in corso.
  - Riconnessione automatica alla prima operazione dopo una caduta.
  - Timeout di connessione 15 s.

### Protocolli

- **SFTP:** autenticazione con password.
  - Impronte accettate in `known_hosts.json` nella cartella di configurazione: host, porta, algoritmo, impronta SHA-256. Nello stesso file stanno le impronte dei certificati TLS accettati.
  - Impronta sconosciuta: dialogo che la mostra, con Accetta (e ricorda) o Annulla.
  - Impronta cambiata: errore bloccante; si sblocca solo togliendo l'impronta vecchia dal gestore delle connessioni.
- **FTP e FTPS:**
  - modalità passiva;
  - FTPS esplicito (`AUTH TLS`) e implicito (porta 990);
  - per i certificati vedi "Certificati TLS" qui sotto;
  - **date poco affidabili:** se il server non supporta `MDTM`, le date vengono dal `LIST` (al minuto, o solo l'anno per i file vecchi). In quel caso F4 chiede sempre conferma prima di salvare, e la politica "più recente" nei conflitti tratta le date come uguali quando la precisione non basta. *(Scelta del progettista.)*
- **Certificati TLS (FTPS e HTTPS).** Si usa l'archivio dei certificati di Windows, così una CA aziendale installata sul PC è accettata come nel browser. Un certificato che Windows non accetta (autofirmato, CA sconosciuta, nome diverso) richiede conferma con un dialogo che mostra l'impronta SHA-256.
  - Accettando, si ricorda quell'impronta per host e porta, nello stesso file delle impronte SSH.
  - Se il certificato cambia, si chiede di nuovo.
  - Non si accetta mai "qualunque certificato" per un host.

  *(Scelta del progettista: gli indici web interni in VPN hanno spesso certificati aziendali o autofirmati.)*
- **HTTP/HTTPS, elenco delle cartelle:**
  - si interpreta l'HTML dell'indice di Apache, nginx e IIS: link agli elementi, più dimensione e data quando la pagina le riporta;
  - autenticazione HTTP Basic e Digest;
  - F3 con richieste `Range`. Se il server non le supporta, F3 avvisa e propone di scaricare il file;
  - nessuna scrittura: F6, F7, F8, la rinomina e F4 sono disattivati, con un avviso tradotto.

### Credenziali e connessioni

- **Connessioni salvate.** Stanno in un nuovo file di configurazione `connections` (whitelist di `config.rs`) con id, nome, protocollo, host, porta, utente, cartella iniziale e opzioni FTPS. Le password no.
- **Password.**
  - Stanno nel Windows Credential Manager, con `CredWriteW`/`CredReadW`/`CredDeleteW` della crate `windows` (feature `Win32_Security_Credentials`): nessuna dipendenza nuova. La chiave è legata all'id della connessione.
  - Il frontend riceve solo "password presente sì/no".
  - Senza password salvata, la connessione la chiede e propone di ricordarla.
- **Gestore delle connessioni (Ctrl+F).** Elenco, nuova, modifica, duplica, elimina, connetti. Eliminando una connessione si eliminano anche la sua password e la sua impronta.
- **Hotlist (Ctrl+D) e Preferiti.** Accettano posizioni remote, che si riaprono connettendosi.
- **Importazione da WinSCP:**
  - legge i siti salvati, dal registro (`HKCU\Software\Martin Prikryl\WinSCP 2\Sessions`) oppure da `WinSCP.ini` se esiste;
  - mostra un'anteprima con nome, protocollo, host e utente, e importa quelli scelti, senza password;
  - i siti SCP diventano SFTP; quelli WebDAV o S3 si saltano, con il motivo.

### Interfaccia

- **Scheda remota.** Si comporta come una scheda locale: frecce, Invio, ricerca rapida, filtro, marcatura, colonne configurabili, ordinamento. La colonna Attributi mostra i permessi POSIX (`rwxr-xr-x`) dove ci sono.
- **Indicatori.** Un'icona di connessione nella scheda e nella barra del percorso. Errori di rete tradotti, con nuovi codici `AppError`:
  - autenticazione fallita;
  - connessione fallita;
  - timeout;
  - impronta sconosciuta o cambiata;
  - certificato non valido;
  - operazione non supportata dalla sorgente.
- **Menu col tasto destro.** Su una scheda remota compare un menu dell'app con i comandi che valgono lì: Vedi, Modifica, Copia, Sposta, Rinomina, Elimina, Nuova cartella, Aggiorna, Copia percorso. *(Scelta del progettista: lo spec generale prevede "il menu dell'app"; il contenuto è questo.)*
- **F3 su file remoti:**
  - testo ed esadecimale leggono a blocchi dal provider, con le stesse funzioni del viewer di oggi (righe intere, ricerca degli inizi riga, colonne);
  - immagini, PDF e media si scaricano prima in una cartella temporanea dell'app e da lì si mostrano come file locali, con avanzamento e annullamento;
  - la cartella temporanea si svuota alla chiusura dell'app e all'avvio successivo.
- **F4 su file remoti:**
  - si legge il file intero, con il limite di dimensione dell'editor di oggi;
  - Ctrl+S scrive sul server con nome temporaneo e rinomina, oppure, se non si può, con il fallback confermato descritto in "Scrittura sicura sul remoto";
  - se data di modifica o dimensione sul server sono cambiate dall'apertura, compare il conflitto come in locale (Sovrascrivi o Annulla);
  - sulle schede remote si usa sempre l'editor interno, anche con "Modifica con un editor esterno" attiva. *(Scelta del progettista: l'editor esterno richiederebbe download, sorveglianza del file temporaneo e ricaricamento; può essere un follow-up.)*
- **Ispettore su file remoti:** metadati e anteprima testuale a blocchi. Niente hash né anteprima di archivi.

### Sicurezza

- **CSP.** Una Content Security Policy che ammette solo:
  - l'origine dell'app;
  - l'asset protocol, per i file locali e temporanei;
  - i worker di Monaco e pdf.js;
  - gli stili in linea che le librerie richiedono.

  Niente script remoti né `eval`. La CSP va verificata su tutte le viste della fase 2 prima della fine della fase.
- **Contenuto remoto non fidato.**
  - L'HTML degli indici web non finisce mai nel DOM: lo legge il backend e restituisce solo nomi e metadati.
  - I nomi remoti si mostrano come testo.
  - Un nome remoto non valido su Windows (`:`, `*`, `?`, nomi riservati) blocca il download di quell'elemento con un errore tradotto. Non viene rinominato in silenzio.

## Lasciato al builder

- Crate per SFTP, FTP/FTPS e HTTP. Lo spec generale suggerisce `suppaftp` e `russh` o `ssh2`. Restano i vincoli generali di licenza (MIT o compatibili, niente GPL) e di dipendenze aggiunte solo con `cargo add`. Consigliabile una pila TLS sola per FTPS e HTTPS.
- Forma esatta dell'interfaccia del provider (trait sincrono o asincrono) e come conviva con l'`Executor` e l'helper elevato di oggi.
- Dimensione dei blocchi di trasferimento, numero di sessioni per connessione, keepalive.
- Struttura dei componenti del gestore delle connessioni e del dialogo dell'impronta.
- Come ripulire nel frontend i circa 15 controlli `=== DRIVES` sparsi, facendo di `lib/paths.ts` e di `parentPath` il punto unico che conosce gli schemi.

## Verifica

- **Test Rust:**
  - provider locale equivalente al codice di oggi: i test esistenti di coda, conflitti ed elevazione restano verdi senza modifiche di significato;
  - copia tra provider con un provider finto in memoria: avanzamento, annullamento a metà (niente temporanei rimasti), conflitti, data di modifica riportata;
  - parser degli indici HTML su campioni reali di Apache, nginx e IIS;
  - parser dei siti di WinSCP, da registro simulato e da `.ini`;
  - instradamento delle posizioni: un percorso remoto non viene mai trattato come percorso Windows;
  - Credential Manager: scrittura, lettura ed eliminazione su una chiave di test.
- **Test Vitest:** posizioni con schema in `lib/paths.ts` (genitore, barra del percorso, unione), comandi disattivati sulle sorgenti in sola lettura.
- **Prove su server reali, nelle verifiche manuali:** almeno un server SFTP dell'utente; un server FTP e FTPS di prova (per esempio in WSL o in un container locale); un indice HTTP con autenticazione; `\\vm645srv`.

# SFTP su russh: chiavi dell'host ed25519 ed ECDSA

Data: 2026-10-27 · Stato: approvata · Riferimenti: spec della fase 3a (`docs/superpowers/specs/2026-10-22-fase-3a-rete-design.md`), follow-up (`docs/superpowers/fase-3a-follow-up.md`, "SFTP: solo chiavi dell'host RSA")

## Perché

Il provider SFTP usa `ssh2` 0.9.6 su `libssh2-sys` 0.3.3, compilata con il backend WinCNG. Con questo backend libssh2 1.11 non ha ed25519 (`LIBSSH2_ED25519 0`) e ha ECDSA solo con `LIBSSH2_ECDSA_WINCNG`, che il build script non definisce: offre solo `rsa-sha2-512`, `rsa-sha2-256` e `ssh-rsa`. Un server con sole chiavi dell'host ed25519 o ECDSA chiude la negoziazione ("no matching host key type found") e l'app mostra un generico "Errore di input/output". Tra libssh2 con OpenSSL e russh l'utente ha scelto russh: è Rust puro, usa `ring` come la parte TLS dell'app e toglie anche lo strato FFI di `sftp/raw.rs`.

## Obiettivi

- Collegarsi a server con chiave dell'host ed25519, ECDSA (P-256, P-384, P-521) o RSA (`rsa-sha2-512`, `rsa-sha2-256`, `ssh-rsa`).
- Stesso comportamento verso il frontend: stessi codici di errore, stesso dialogo dell'impronta (`HOST_KEY_UNKNOWN`/`HOST_KEY_CHANGED` con `SHA256:<base64>` in `detail`), stesse richieste di password (`AUTH_REQUIRED`/`AUTH_FAILED`).
- Impronte già salvate in `known_hosts` ancora valide, senza nuove richieste all'utente: stesso formato, calcolato sullo stesso blob della chiave, e lo stesso tipo di chiave chiesto al server (vedi "Tipo di chiave dell'host").
- Nessuna regressione verso server vecchi che oggi si collegano: gli algoritmi datati che libssh2 accetta restano, in coda.
- Un errore specifico e tradotto quando il server ha solo algoritmi di chiave dell'host che l'app non gestisce (per esempio solo `ssh-dss`).
- Test del provider in `cargo test` contro un server SSH+SFTP finto, interno al processo, con sola chiave ed25519.
- Via `ssh2` e `libssh2-sys` da `Cargo.toml`.

## Non obiettivi

- Autenticazione a chiave pubblica, agent, certificati dell'host: l'app resta a password e keyboard-interactive.
- Cambiare `Provider`, `Vfs`, `Sessions` o i trasferimenti.
- Prestazioni migliori di oggi: basta non peggiorare sensibilmente (i trasferimenti grandi restano a letture e scritture in parallelo, vedi sotto).

## Anti-obiettivi

- `aws-lc-rs` nell'albero delle dipendenze (build con CMake/NASM, secondo provider crittografico).
- Una password in un file, in un log, in `AppError.detail` o in un evento.
- Un `panic` nel percorso di produzione: il profilo release ha `panic = "abort"`, quindi un `block_on` chiamato nel posto sbagliato chiuderebbe l'app.
- Impronte salvate che smettono di valere (l'utente si vedrebbe chiedere di nuovo di fidarsi di tutti i server).
- Una chiusura di sessione che blocca il thread di chi lascia cadere il provider (oggi risolto con `drop_in_background`).

## Vincoli

- Dipendenze aggiunte solo con `cargo add`; API lette nei sorgenti o nella documentazione della versione installata.
- `russh` senza feature predefinite (che includono `aws-lc-rs`): `default-features = false`, feature `ring`, `rsa`, `flate2`, `des` (per `3des-cbc`). `russh-sftp` non dipende da `russh` (solo in dev) e porta `tokio` senza feature predefinite.
- Versioni di riferimento al momento della scrittura: `russh` 0.65.0, `russh-sftp` 3.0.1; `tokio` è già nell'albero (1.53.x, tramite Tauri).
- Commenti, documentazione e commit in italiano, identificatori in inglese.

## Approcci per il ponte sincrono/asincrono

`Provider` è sincrono e lo chiamano solo thread bloccanti: i comandi Tauri passano da `fs::blocking` (`spawn_blocking`), i trasferimenti hanno i loro thread, `Sessions` è sincrono. russh e russh-sftp sono asincroni su tokio.

- **A. Runtime tokio dedicato al modulo SFTP, `block_on` in ogni metodo (scelto dall'utente).** Isolato da Tauri, testabile senza l'app, timeout uniformi con `tokio::time::timeout`; costa un secondo runtime con due thread e la regola "mai chiamare il provider da un task asincrono".
- **B. Runtime di Tauri (`tauri::async_runtime`).** Nessun runtime in più, ma le sessioni SSH dividono i worker con i comandi asincroni dell'app e `net` dipende da Tauri; stessa regola su `block_on`.
- **C. `Provider` asincrono.** Riscrittura di `Vfs`, `Sessions`, FTP, HTTP, SMB e trasferimenti: sproporzionato. Scartato.

## Progetto

### Runtime

- Un runtime per tutto il modulo, creato alla prima connessione SFTP (`LazyLock`/`OnceLock`): multi-thread, 2 worker, nome dei thread `sftp-rt`, `enable_all`. Se non si crea, l'apertura dà `IO` con il messaggio in `detail`.
- Ogni metodo di `SftpProvider` e ogni `read`/`write`/`flush` degli stream fa `runtime.block_on(...)` di un futuro limitato da `tokio::time::timeout`. I task di russh (lettura/scrittura del socket, canali) girano sui worker del runtime, quindi la connessione resta viva anche tra una chiamata e l'altra.
- Regola documentata nel modulo: i metodi del provider si chiamano solo da thread bloccanti, mai da un task tokio (né di Tauri né di `sftp-rt`). Oggi è sempre così; non si aggiungono controlli a runtime (tokio non offre un modo affidabile per distinguere un thread di `spawn_blocking` da un worker).

### Connessione (`open_in`)

1. TCP con la funzione `tcp` attuale (risoluzione, tentativo su ogni indirizzo, `CONNECT_TIMEOUT` 15 s), poi `TcpStream::set_nodelay` e conversione in `tokio::net::TcpStream` dentro il runtime.
2. `russh::client::connect_stream` con una `Config` costruita da `Preferred::DEFAULT` (vedi "Algoritmi" e "Tipo di chiave dell'host"), senza `inactivity_timeout` (la sessione di navigazione resta aperta finché serve; le cadute le scopre la chiamata successiva con `TIMEOUT` o `CONNECTION_FAILED`, come oggi).
3. Impronta dell'host nel `Handler::check_server_key` del client: blob della chiave nel formato SSH (lo stesso che restituiva `libssh2_session_hostkey`, cioè `PublicKey::to_bytes()` di `ssh-key`), `fingerprint` invariata, verifica con `trust::check_in`. Con `Known` si prosegue; con `Unknown`/`Changed` l'handler registra esito e impronta e rifiuta la chiave; `open_in` restituisce `HOST_KEY_UNKNOWN`/`HOST_KEY_CHANGED` con l'impronta in `detail`. Certificati dell'host: non si annunciano (`host_key_certificates` vuoto, come nel default di russh).
4. Autenticazione, con `auth_plan` invariata nel significato: `authenticate_none` (se il server accetta si è dentro); altrimenti dai metodi rimasti `password` → `authenticate_password`, oppure `keyboard-interactive` → `authenticate_keyboard_interactive_start` e risposte con la regola attuale di `Answer` (la password solo alla prima richiesta con una sola domanda nascosta, risposte vuote a tutto il resto, quindi `AUTH_FAILED`). Senza password e con un server che la chiede: `AUTH_REQUIRED`. Rifiuto: `AUTH_FAILED`.
5. **Un solo canale `subsystem sftp`, con `russh_sftp::client::RawSftpSession`** (in un `Arc`), per tutto: operazioni, file, `posix-rename@openssh.com` tramite `extended`. *Scelta mia*, dopo aver letto russh-sftp 3.0.1: `SftpSession` non espone la sessione grezza (servirebbe un secondo canale per `posix-rename`), e il suo `File` trasforma ogni stato SFTP in `io::Error::other(messaggio)` (`check_read_result`/`check_write_result` in `client/fs/file.rs`), perdendo `NOT_FOUND`, `ACCESS_DENIED` e la distinzione tra errore del server e connessione caduta, che serve a `sessions::dropped`. Con la sola sessione grezza i codici restano esatti e cade il limite noto "un server che rifiuta un secondo canale non si collega". Il prezzo: le letture e scritture in parallelo degli stream si scrivono a mano (vedi "Operazioni").
6. `atomic` dall'`init()` (estensione `posix-rename@openssh.com` versione `"1"` tra quelle dichiarate), senza più la prova su `/.mc-posix-rename-probe`. Se il server dichiara `limits@openssh.com`, se ne leggono i limiti (`limits()` e `set_limits`) per la dimensione dei blocchi.
7. Timeout delle richieste SFTP: `Config::request_timeout_secs` = 30 s (`IO_TIMEOUT_MS`), e lo stesso limite intorno a ogni `block_on`. Connessione, handshake e autenticazione insieme: `CONNECT_TIMEOUT`.

Il cuore di `open_in` prende uno stream qualsiasi (`AsyncRead + AsyncWrite`): `open_in` gli passa il TCP, i test un `tokio::io::duplex` collegato al server finto.

### Algoritmi

Si parte da `Preferred::DEFAULT` di russh 0.65 e si aggiungono **in coda** a ogni lista gli algoritmi datati che libssh2 con WinCNG accetta oggi (decisione dell'utente: nessuna regressione verso NAS e apparati vecchi), così si usano solo se il server non offre nient'altro:

- scambio di chiavi: `diffie-hellman-group14-sha1`, `diffie-hellman-group-exchange-sha1` (dopo `diffie-hellman-group14-sha256` e prima dei nomi-segnale delle estensioni, `ext-info-c` e `kex-strict-*`);
- cifrari: `aes256-cbc`, `aes192-cbc`, `aes128-cbc`, `3des-cbc` (quest'ultimo richiede la feature `des` di russh);
- MAC: `hmac-sha1-etm@openssh.com`, `hmac-sha1`.

Restano fuori, perché russh non li ha o sono rotti: `diffie-hellman-group1-sha1`, `arcfour*`, `hmac-md5`, `hmac-sha1-96`. Un server che offre solo quelli dà `CONNECTION_FAILED` con gli algoritmi in `detail`.

### Tipo di chiave dell'host

Con libssh2 ogni impronta `ssh` salvata è di una chiave RSA; russh preferisce ed25519, quindi un OpenSSH moderno presenterebbe un'altra chiave e l'app mostrerebbe "impronta cambiata" (l'avviso di possibile attacco) su quasi tutti i server salvati. Decisione dell'utente: **`known_hosts` ricorda il tipo di chiave**.

- `Entry` di `trust.rs` ha un campo nuovo facoltativo `key_type` (nome SSH del tipo: `ssh-ed25519`, `ecdsa-sha2-nistp256`, `ecdsa-sha2-nistp384`, `ecdsa-sha2-nistp521`, `ssh-rsa`), `#[serde(default)]`, omesso per le voci TLS. Una voce `ssh` senza `key_type` è una voce di libssh2, quindi RSA. Il file resta leggibile dalla versione precedente dell'app (campo in più ignorato da serde).
- Prima della connessione `open_in` legge la voce di host e porta: se c'è, mette in testa a `Preferred.key` quel tipo (per RSA le tre varianti `rsa-sha2-512`, `rsa-sha2-256`, `ssh-rsa`, che hanno lo stesso blob e quindi la stessa impronta), seguito dal resto della lista predefinita. Il server presenta la stessa chiave di prima: impronta nota, nessuna richiesta. Se il server quel tipo non ce l'ha più, presenta un'altra chiave: `HOST_KEY_CHANGED`, giusto, perché la chiave è davvero cambiata.
- Senza voce si usa l'ordine predefinito (ed25519 per primo).
- Accettazione: il frontend chiama `trust_host(id, fingerprint)` senza tipo e non cambia. `trust` tiene in memoria le chiavi presentate e rifiutate (host, porta, impronta → tipo) registrate da `check_in`; `accept_in` per `ssh` prende il tipo da lì se l'impronta corrisponde. Se non lo trova (caso che il flusso non produce: il dialogo segue subito l'errore nello stesso processo) dà `INVALID_REQUEST` e non scrive nulla: una voce senza tipo verrebbe letta come RSA. *Scelta mia.*
- Le voci vecchie non si riscrivono: "senza tipo = RSA" vale per sempre. Quando l'utente accetta una chiave nuova per quell'host (per esempio dopo un `HOST_KEY_CHANGED`) la voce si sostituisce con quella tipata, come oggi.

### Operazioni

Comportamento identico a oggi, riga per riga di `sftp.rs`:

- `stat`/`list`/`to_stat`: `lstat`, risoluzione dei collegamenti con `stat`, `dir_link`, dimensione 0 per le cartelle, `mtime_ms`, `mode & 0o7777`. `list` con `opendir` + `readdir` fino a `EOF` + `close`, senza `.` e `..`: russh-sftp decodifica i nomi con `from_utf8_lossy` già nel protocollo, quindi un nome non UTF-8 si vede con U+FFFD come oggi, senza FFI. Ogni operazione su un percorso con U+FFFD dà `INVALID_NAME` senza inviare nulla.
- `\` nei nomi: resta `INVALID_NAME`, come oggi. russh-sftp invierebbe il percorso così com'è, ma un nome remoto `a\b` copiato in locale diventerebbe `C:\dst\a\b` (sottocartella `a`), e i trasferimenti e `net/temp.rs` controllano solo l'ultimo componente. *Scelta mia*: sbloccarlo richiede un controllo sulla destinazione locale, fuori da questo lavoro; il motivo del limite noto nel follow-up si aggiorna.
- `read_at`: `open` in lettura, richieste `read` da `offset` fino a `len` byte o a `EOF` (una risposta può essere più corta del richiesto: si continua), `close`.
- `reader`/`writer`: stream sincroni sulla sessione grezza, con le richieste in volo in parallelo scritte nel modulo: fino a 16 richieste da 32 KiB (o il limite di `limits@openssh.com`, se più piccolo), come i default di russh-sftp. Ogni `read`/`write`/`flush` sincrono è un `block_on`.
  - `reader`: tiene una coda di letture anticipate a offset crescenti e restituisce i dati in ordine; una risposta corta si completa con una lettura del resto prima di andare avanti; `EOF` chiude.
  - `writer`: `open` con `WRITE | CREATE | EXCLUDE` se `create_new`, altrimenti `WRITE | CREATE | TRUNCATE`, permessi 0o644; ogni `write` accoda richieste e aspetta la più vecchia solo quando le richieste in volo sono al massimo. **`flush` aspetta le conferme di tutte le scritture in volo, chiude il file e restituisce il primo errore**: con le scritture in parallelo un errore del server arriva dopo il `write` che l'ha causato (oggi `flush` non fa nulla perché libssh2 scrive in modo sincrono). Dopo un errore ogni `write` successivo lo ripete. Un writer lasciato cadere senza `flush` chiude il file sul runtime, senza aspettare.
  - Gli errori portano l'`AppError` con il codice esatto dentro `io::Error::other`, come oggi (`stream_error` lo ritrova).
- `rename`: con `replace` e `atomic` → `posix-rename@openssh.com`; altrimenti controllo con lstat che la destinazione non esista (`ALREADY_EXISTS`) e `rename` di SFTP v3. Uguale a oggi.
- `delete`: collegamento a cartella con `remove_file`, mai `rmdir` né ricorsione; cartella con `remove_dir`; file con `remove_file`.
- `mkdir` 0o755 e `writer(create_new)`: un `SSH_FX_FAILURE` su un percorso che esiste diventa `ALREADY_EXISTS` (`exists_err`, SFTP v3).
- `set_meta`: `setstat` con `atime = mtime` in secondi e `perm`, nulla se entrambi mancano.

### Errori

| Sorgente | Codice |
|---|---|
| Stato SFTP `NoSuchFile` | `NOT_FOUND` |
| `PermissionDenied` | `ACCESS_DENIED` |
| `OpUnsupported` | `UNSUPPORTED` |
| `NoConnection`, `ConnectionLost` | `CONNECTION_FAILED` |
| `Failure`, `BadMessage`, altri | `IO` |
| Timeout di russh-sftp o di `tokio::time::timeout` | `TIMEOUT` |
| russh: socket chiuso, disconnessione, errore di I/O | `CONNECTION_FAILED` (timeout del socket: `TIMEOUT`) |
| russh: `NoCommonAlgo { kind: Key, .. }` | **`HOST_KEY_UNSUPPORTED`** (nuovo), `detail` con gli algoritmi offerti dal server |
| russh: `NoCommonAlgo` di altro tipo (kex, cifrario, MAC) | `CONNECTION_FAILED`, `detail` con il messaggio di russh |
| Autenticazione rifiutata | `AUTH_FAILED` |
| Canale SFTP chiuso (sessione grezza che non risponde più, invio fallito) | `CONNECTION_FAILED` |

Codici SFTP oltre l'8: il `StatusCode` di russh-sftp 3.0.1 conosce solo i valori 0–8 (SFTP v3). I codici v6 che `map_sftp_error` gestisce oggi (`FILE_ALREADY_EXISTS`, `NO_SPACE_ON_FILESYSTEM`, `QUOTA_EXCEEDED`, `WRITE_PROTECT`, `NO_SUCH_PATH`) spariscono dalla mappatura. Un server che li manda comunque fa fallire la decodifica del pacchetto, e russh-sftp chiude il canale: le richieste in corso finiscono in `TIMEOUT` o `CONNECTION_FAILED`, `Sessions` riapre la sessione. *Scelta mia*: si accetta il rischio (OpenSSH e i server comuni parlano v3) e lo si scrive tra i limiti noti; un filtro sullo stream del canale che riscrive gli stati oltre l'8 in `FAILURE` si aggiunge se un server reale lo richiede.

Nuovo codice `HOST_KEY_UNSUPPORTED` in `ErrorCode` (Rust), nel tipo `ErrorCode` di `src/ipc/types.ts` e nelle traduzioni (`it.json`: "Il server usa solo tipi di chiave dell'host non supportati.", `en.json`: "The server only offers unsupported host key types."). Non entra in `RECONNECT` né nei gruppi `AUTH`/`TRUST` del frontend: è un errore finale.

`detail` porta solo messaggi delle librerie o l'impronta; nessuna struttura che contenga la password ha `Debug`.

### Chiusura

`Drop` di `SftpProvider` manda sul runtime (`spawn`) la chiusura del canale SFTP e `disconnect` della sessione, con un timeout breve, e ritorna subito: sostituisce il thread `sftp-close` per SFTP. `drop_in_background` resta, perché la usa FTP; con `tcp`, `CONNECT_TIMEOUT` e `IO_TIMEOUT_MS` si sposta in `net/mod.rs` (oggi FTP e HTTP li importano da `sftp`).

### Test

- **Server finto** (solo `#[cfg(test)]`): `russh::server` con una chiave ed25519 generata nel test e il `server::Handler` di `russh-sftp` sopra un piccolo file system in memoria (file, cartelle, collegamenti, nomi in byte grezzi per il caso non UTF-8, `posix-rename@openssh.com` dichiarata o no). Collegato al client con `tokio::io::duplex`, senza socket: i test di loopback TCP falliscono con la VPN Check Point (vedi l'esito della fase 3a). Opzioni del finto: password attesa, solo password o solo keyboard-interactive, `posix-rename` sì/no, chiave dell'host fissa per provare `HOST_KEY_CHANGED`.
- Il server finto può avere più chiavi dell'host (ed25519 e RSA) per provare il tipo ricordato.
- Casi coperti in `cargo test`: impronta sconosciuta → accettata (con il tipo) → nota → cambiata; voce senza `key_type` (di libssh2) su un server con ed25519 e RSA: si negozia RSA e l'impronta è nota, senza richieste; voce ed25519: si negozia ed25519; `accept_in` senza chiave presentata: `INVALID_REQUEST`, file intatto; impronta del finto uguale a quella di `ssh-keygen -lf` sulla stessa chiave (`fingerprint_format` resta); `AUTH_REQUIRED`, `AUTH_FAILED`, password e keyboard-interactive (con la regola della domanda nascosta unica); tutte le operazioni di `sftp_live` (mkdir, writer con `create_new`, `read_at`, reader, `set_meta`, stat, list, rename con e senza `posix-rename`, collegamenti, delete); file più grandi di 16 × 32 KiB letti e scritti interi e in ordine; nome non UTF-8 elencato con U+FFFD e `INVALID_NAME` su ogni operazione; errore di scrittura (permesso negato dal finto) che emerge in `flush` con il codice esatto; canale chiuso dal finto → `CONNECTION_FAILED`; `HOST_KEY_UNSUPPORTED` dalla mappatura di `NoCommonAlgo`; la `Config` del client contiene gli algoritmi datati in coda.
- `trust.rs`: test di lettura di un `known_hosts` scritto dalla versione precedente (senza `key_type`).
- Restano: i test unitari di `auth_plan`, `Answer`, `mtime_ms`, `fingerprint_format`, la mappatura degli stati; `sftp_live` (ignorato) con la stessa variabile `MC_TEST_SFTP`, eseguito contro un server con sola chiave ed25519. `closing_happens_off_the_calling_thread` segue `drop_in_background` dove va.
- `sessions.rs` non cambia, i suoi test nemmeno.

### Documentazione

- `docs/superpowers/fase-3a-follow-up.md`: la voce "SFTP: solo chiavi dell'host RSA" diventa risolta (russh); limiti noti aggiornati (tolto "server che rifiuta un secondo canale"; nomi con `\` ancora non apribili, con il motivo nuovo; codici SFTP oltre l'8); verifiche manuali: server con sola chiave ed25519 (per esempio sshd in WSL con la sola `HostKey /etc/ssh/ssh_host_ed25519_key`, raggiunto sull'IP della macchina WSL), server salvato con la versione precedente che resta noto senza richieste, server con soli algoritmi datati se disponibile.
- Commento del modulo `sftp.rs` riscritto (russh, runtime, regola del `block_on`).

## Lasciato a chi costruisce

- Struttura interna del modulo (`sftp.rs` più file accanto per runtime, errori, server finto), nomi dei tipi interni.
- Come l'handler del client restituisce l'esito dell'impronta (stato condiviso o tipo d'errore proprio dell'handler).
- Feature di `tokio` da aggiungere esplicitamente (`rt-multi-thread`, `net`, `time`, `io-util`, e `macros` se serve nei test).
- Generazione della chiave ed25519 del finto (feature di `ssh-key`/`rand` se servono, solo in `dev-dependencies`).

## Decisioni dell'utente

- Approccio A: runtime tokio dedicato e `block_on`.
- `known_hosts` ricorda il tipo di chiave; le voci senza tipo valgono come RSA.
- Algoritmi datati di libssh2 riattivati in coda.

## Scelte mie, da controllare

- Un solo canale con `RawSftpSession` e richieste in parallelo scritte nel modulo, invece di `SftpSession`/`File` (che perdono i codici di errore negli stream e richiedono un secondo canale per `posix-rename`).
- `\` nei nomi remoti resta `INVALID_NAME` (sbloccarlo tocca i trasferimenti).
- Codici SFTP oltre l'8 non gestiti: rischio accettato e scritto tra i limiti noti.
- `accept_in` senza la chiave presentata in memoria: `INVALID_REQUEST`, nessuna voce senza tipo.
- `atomic` dall'elenco delle estensioni dichiarate, senza la prova di rinomina.
- Nome e testo del nuovo codice `HOST_KEY_UNSUPPORTED`.
- Server finto su `duplex` invece che su TCP locale.

# SFTP su russh: piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** sostituire il provider SFTP su `ssh2`/libssh2 (WinCNG, solo chiavi dell'host RSA) con `russh` + `russh-sftp`, così l'app si collega anche a server con chiavi ed25519 ed ECDSA, senza cambiare il comportamento verso il frontend.

**Architecture:** un runtime tokio dedicato al modulo (`sftp-rt`, 2 worker) esegue russh; ogni metodo sincrono di `Provider` e ogni `read`/`write`/`flush` degli stream fa `block_on` di un futuro limitato da un timeout. Una sola `RawSftpSession` per connessione, con letture e scritture in parallelo scritte nel modulo. `known_hosts` ricorda il tipo di chiave dell'host, così le impronte salvate con libssh2 (RSA) restano valide.

**Tech Stack:** Rust edition 2024, Tauri 2. Crate nuove, solo con `cargo add`: `russh` 0.65 (`default-features = false`, feature `ring`, `rsa`, `flate2`, `des`), `russh-sftp` 3.0, `tokio` (già nell'albero tramite Tauri; feature dirette `rt-multi-thread`, `net`, `time`, `io-util`, `macros`). Crate tolte: `ssh2`, `libssh2-sys`.

**Spec:** `docs/superpowers/specs/2026-10-27-sftp-russh-design.md`. Il provider attuale è `src-tauri/src/net/sftp.rs` (con `sftp/raw.rs`); il contratto è `src-tauri/src/vfs/provider.rs`.

## Global Constraints

- **Commit solo con conferma esplicita dell'utente**: i passi "Commit" sono proposte da confermare. Niente push.
- Documentazione, commenti e messaggi di commit in italiano; identificatori in inglese. Commit con `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Dipendenze solo con `cargo add`/`cargo remove`; le API si leggono nei sorgenti o su docs.rs **della versione installata** (russh 0.65, russh-sftp 3.0).
- **Mai `aws-lc-rs`**: `cargo tree -i aws-lc-rs` deve dare "did not match any packages" dopo ogni task che tocca `Cargo.toml`.
- **Password:** mai in file, log, `AppError.detail`, eventi; nessun tipo che la contiene ha `Debug`.
- Impronte nel formato `SHA256:<base64 senza padding>` del blob SSH della chiave (`fingerprint` attuale, invariata).
- I metodi del provider si chiamano solo da thread bloccanti, mai da un task tokio: il profilo release ha `panic = "abort"`.
- Timeout: `CONNECT_TIMEOUT` 15 s (connessione, handshake, autenticazione), `IO_TIMEOUT_MS` 30 000 ms (ogni operazione e `request_timeout_secs` di russh-sftp).
- Nessuna stringa UI nel codice: il nuovo `ErrorCode` ha `errors.<CODE>` in `it.json` ed `en.json`.
- I test di loopback TCP falliscono con la VPN Check Point: i test nuovi usano `tokio::io::duplex`, mai socket.
- `sessions.rs` non cambia.

## Review Focus

1. **Rete che smette di rispondere a metà upload** (VPN caduta, scritture in volo senza conferma): `write`/`flush` danno `TIMEOUT` entro il limite, nessuna attesa infinita, il `Drop` del writer non blocca. → test `writer_on_stalled_server_times_out` (Task 5).
2. **Server che risponde a una lettura con meno byte del richiesto** a metà file: il reader non lascia buchi né riordina. → test `reader_handles_short_reads_in_order` (Task 5).
3. **File vuoto e `read_at` oltre la fine:** risultato vuoto, nessun errore. → test `empty_file_and_read_past_end` (Task 5).
4. **Password con caratteri non ASCII o `:` rifiutata:** `AUTH_FAILED`, e né `detail` né il messaggio contengono la password. → test `auth_failure_never_leaks_the_password` (Task 4).
5. **Host salvato in maiuscolo o su un'altra porta:** il tipo di chiave ricordato si trova senza distinguere maiuscole e minuscole, mai da un'altra porta. → test `saved_key_type_matches_host_case_insensitively` (Task 2).

---

### Task 1: Dipendenze, runtime e utilità di rete condivise

**Files:**
- Modify: `src-tauri/Cargo.toml` (con `cargo add`), `src-tauri/src/net/mod.rs`, `src-tauri/src/net/ftp.rs`, `src-tauri/src/net/http_index.rs`, `src-tauri/src/net/sftp.rs` (import)
- Create: `src-tauri/src/net/sftp/rt.rs`

**Interfaces:**
- Produces (`net`, spostati da `sftp.rs` senza cambiarli): `pub(crate) const CONNECT_TIMEOUT: Duration`, `pub(crate) const IO_TIMEOUT_MS: u32`, `pub(crate) fn tcp(host: &str, port: u16) -> Result<std::net::TcpStream, AppError>`, `pub(crate) fn drop_in_background<T: Send + 'static>(v: T)`. `ftp.rs` e `http_index.rs` li importano da `super::`.
- Produces (`net::sftp::rt`):
  - `pub(super) fn runtime() -> Result<&'static tokio::runtime::Runtime, AppError>`: creato al primo uso (`OnceLock`), multi-thread, `worker_threads(2)`, `thread_name("sftp-rt")`, `enable_all()`; un errore di creazione dà `IO` con il messaggio in `detail` (e si ritenta alla chiamata successiva).
  - `pub(super) fn block<T>(limit: Duration, fut: impl Future<Output = Result<T, AppError>>) -> Result<T, AppError>`: `runtime()?.block_on(tokio::time::timeout(limit, fut))`, scaduto → `AppError::new(ErrorCode::Timeout, None)`.
  - `pub(super) fn spawn(fut: impl Future<Output = ()> + Send + 'static)`: sul runtime, senza aspettare; se il runtime non c'è, non fa nulla.

- [ ] **Step 1: Dipendenze.** `cargo add russh@0.65 --no-default-features --features ring,rsa,flate2,des`, `cargo add russh-sftp@3`, `cargo add tokio --features rt-multi-thread,net,time,io-util,macros`. Verifica: `cargo tree -i aws-lc-rs` → "did not match any packages".
- [ ] **Step 2: Test** in `rt.rs`: `block_returns_the_value` (`block(1 s, async { Ok(7) })` → `Ok(7)`); `block_times_out` (`block(20 ms, sleep 500 ms)` → codice `Timeout`, in meno di 400 ms); `spawn_runs_off_the_calling_thread` (il futuro manda `std::thread::current().name()` su un canale: `Some("sftp-rt")`).
- [ ] **Step 3: Verificare che falliscano** (compilazione).
- [ ] **Step 4: Implementare** `rt.rs`; spostare `tcp`, le due costanti, `drop_in_background` e il test `closing_happens_off_the_calling_thread` in `net/mod.rs`; aggiornare gli import di `ftp.rs`, `http_index.rs`, `sftp.rs`.
- [ ] **Step 5: Verificare:** `cargo test --manifest-path src-tauri/Cargo.toml` verde (a parte i 3 test HTTP di loopback se la VPN è attiva, vedi l'esito della fase 3a).
- [ ] **Step 6: Commit** (da confermare): `build(net): russh, russh-sftp e runtime tokio per SFTP`.

### Task 2: `known_hosts` ricorda il tipo di chiave dell'host

**Files:**
- Modify: `src-tauri/src/net/trust.rs`, `src-tauri/src/net/connections.rs` (test), `src-tauri/src/net/sftp.rs` (una riga nel provider attuale)

**Interfaces:**
- Produces (`net::trust`):
  - `Entry` con `#[serde(default, skip_serializing_if = "Option::is_none")] key_type: Option<String>` (nome SSH: `ssh-ed25519`, `ecdsa-sha2-nistp256|384|521`, `ssh-rsa`).
  - `pub(crate) fn saved_ssh_key_type_in(dir: &Path, host: &str, port: u16) -> Result<Option<String>, AppError>`: voce `ssh` di host e porta (con `same`) → `Some(key_type)`, o `Some("ssh-rsa")` se manca il tipo (voce di libssh2); nessuna voce → `None`.
  - `pub(crate) fn presented(host: &str, port: u16, fingerprint: &str, key_type: &str)`: registra in memoria (static `Mutex`, al massimo 16 voci, le più vecchie escono) una chiave presentata e rifiutata.
  - `accept_in(dir, kind, host, port, fingerprint)` (firma invariata): per `"ssh"` prende `key_type` dalla chiave presentata con stesso host (`same`), porta e impronta; se non c'è → `INVALID_REQUEST` e il file non cambia. Per `"tls"` come oggi, senza tipo.
  - `check_in` invariata.
- Il provider attuale (`ssh2`), finché c'è, chiama `trust::presented(host, port, &fp, "ssh-rsa")` prima di restituire `HOST_KEY_UNKNOWN`/`HOST_KEY_CHANGED`: l'app resta usabile tra un commit e l'altro.

- [ ] **Step 1: Test** in `trust.rs`:
  - `legacy_entry_reads_as_rsa`: un `known_hosts` scritto a mano con `[{"kind":"ssh","host":"h","port":22,"fingerprint":"SHA256:a"}]` (via `config::write_internal_in`) → `saved_ssh_key_type_in(d,"h",22) == Some("ssh-rsa")`, `check_in(d,"ssh","h",22,"SHA256:a") == Known`.
  - `accept_ssh_records_presented_key_type`: `presented("h",22,"SHA256:b","ssh-ed25519")`, `accept_in(d,"ssh","h",22,"SHA256:b")` → `saved_ssh_key_type_in == Some("ssh-ed25519")`; la voce TLS dello stesso host resta senza `key_type` nel JSON.
  - `accept_ssh_without_presented_key_is_refused`: `accept_in(d,"ssh","h2",22,"SHA256:z")` → `InvalidRequest`, `check_in` ancora `Unknown`.
  - `saved_key_type_matches_host_case_insensitively`: voce per `Example.ORG:22` → trovata con `example.org`, 22; `None` con `example.org`, 2222.
  - Aggiornare `trust_unknown_changed_known` e i test di `connections.rs` che chiamano `accept_in(.., "ssh", ..)`: prima `presented(..)` con la stessa impronta.
- [ ] **Step 2: Verificare che falliscano.**
- [ ] **Step 3: Implementare** in `trust.rs`; riga `presented` nel provider attuale.
- [ ] **Step 4: Verificare:** `cargo test --manifest-path src-tauri/Cargo.toml trust connections` verde.
- [ ] **Step 5: Commit** (da confermare): `feat(net): known_hosts ricorda il tipo di chiave dell'host`.

### Task 3: Errore `HOST_KEY_UNSUPPORTED` e traduzione degli errori di russh

**Files:**
- Modify: `src-tauri/src/error.rs`, `src/ipc/types.ts`, `src/i18n/it.json`, `src/i18n/en.json`
- Create: `src-tauri/src/net/sftp/errors.rs`

**Interfaces:**
- Produces: `ErrorCode::HostKeyUnsupported` (serializzato `HOST_KEY_UNSUPPORTED`, doc: "Il server offre solo tipi di chiave dell'host che l'app non gestisce: `detail` porta quelli offerti."), `"HOST_KEY_UNSUPPORTED"` nel tipo `ErrorCode` di `types.ts`, `errors.HOST_KEY_UNSUPPORTED` = it "Il server usa solo tipi di chiave dell'host non supportati." / en "The server only offers unsupported host key types.". Non entra in `RECONNECT`, `AUTH`, `TRUST` del frontend.
- Produces (`net::sftp::errors`):
  - `pub(super) fn status_code(code: russh_sftp::protocol::StatusCode) -> ErrorCode`: `NoSuchFile` → `NotFound`, `PermissionDenied` → `AccessDenied`, `OpUnsupported` → `Unsupported`, `NoConnection`/`ConnectionLost` → `ConnectionFailed`, il resto → `Io`.
  - `pub(super) fn sftp_error(e: russh_sftp::client::error::Error, path: Option<&str>) -> AppError`: `Status(s)` → `status_code` con `s.error_message` in `detail`; `Timeout` → `Timeout`; errori di invio/ricezione sul canale (`UnexpectedBehavior` da `SendError`/`RecvError`, `IO`) → `ConnectionFailed`; il resto → `Io`.
  - `pub(super) fn ssh_error(e: &russh::Error) -> AppError`: `NoCommonAlgo { kind: AlgorithmKind::Key, theirs, .. }` → `HostKeyUnsupported` con `theirs.join(", ")` in `detail`; `NoCommonAlgo` di altro tipo → `ConnectionFailed` con il messaggio; `IO` con `TimedOut` → `Timeout`; `IO`, `Disconnect`, `HUP`, `SendError`, `ConnectionTimeout`/`KeepaliveTimeout` (`Timeout` per questi due) → `ConnectionFailed`; il resto → `Io`. Le varianti esatte si leggono in `russh/src/lib_inner.rs` della 0.65.

- [ ] **Step 1: Test** in `errors.rs`: `sftp_status_mapping` (i cinque casi sopra più `Failure` → `Io`), `sftp_timeout_and_channel_errors` (`Error::Timeout` → `Timeout`), `no_common_host_key_is_host_key_unsupported` (`russh::Error::NoCommonAlgo { kind: Key, ours: vec!["ssh-ed25519".into()], theirs: vec!["ssh-dss".into()] }` → codice `HostKeyUnsupported`, `detail == Some("ssh-dss")`), `no_common_cipher_is_connection_failed`. Nel frontend, se c'è un test che confronta i codici con le traduzioni, deve restare verde.
- [ ] **Step 2: Verificare che falliscano.**
- [ ] **Step 3: Implementare.**
- [ ] **Step 4: Verificare:** `cargo test --manifest-path src-tauri/Cargo.toml sftp::errors` verde; `npm test` verde.
- [ ] **Step 5: Commit** (da confermare): `feat(net): errore per le chiavi dell'host non supportate e traduzione degli errori di russh`.

### Task 4: Connessione con russh (algoritmi, impronta, autenticazione) e server finto

**Files:**
- Create: `src-tauri/src/net/sftp/connect.rs`, `src-tauri/src/net/sftp/fake.rs` (`#[cfg(test)]`)

**Interfaces:**
- Consumes: `rt::block`, `rt::spawn` (Task 1); `trust::check_in`, `trust::presented`, `trust::saved_ssh_key_type_in` (Task 2); `errors::*` (Task 3); `fingerprint` di `sftp.rs`.
- Produces (`net::sftp::connect`):
  - `pub(super) struct Options { pub connect_timeout: Duration, pub io_timeout: Duration }`, `Options::DEFAULT` = 15 s / 30 s.
  - `pub(super) struct Link { pub handle: russh::client::Handle<HostKeyCheck>, pub sftp: Arc<russh_sftp::client::RawSftpSession> }`; `Drop` fa `rt::spawn` di `sftp.close_session()` e `handle.disconnect(Disconnect::ByApplication, "", "")` sotto un timeout di 5 s, e ritorna subito.
  - `pub(super) struct Connected { pub link: Arc<Link>, pub atomic: bool, pub chunk: u32 }`: `atomic` = `posix-rename@openssh.com` versione `"1"` nelle estensioni di `init()`; `chunk` = 32 768, o `max_read_len`/`max_write_len` di `limits@openssh.com` se più piccoli (applicati anche con `set_limits`).
  - `pub(super) async fn connect<S: AsyncRead + AsyncWrite + Unpin + Send + 'static>(dir: &Path, conn: &Connection, password: Option<&str>, stream: S, opts: &Options) -> Result<Connected, AppError>`: `client::connect_stream` con `Config { preferred: preferred(saved), .. }` (nessun `inactivity_timeout`), poi autenticazione, poi canale `session` + `request_subsystem(true, "sftp")` + `RawSftpSession::new_with_config(channel.into_stream(), Config { request_timeout_secs: opts.io_timeout.as_secs(), ..Default })` + `init()`.
  - `pub(super) fn preferred(saved_key_type: Option<&str>) -> russh::Preferred`: parte da `Preferred::DEFAULT`; chiavi: il tipo salvato in testa (per `ssh-rsa` le tre varianti RSA: `Sha512`, `Sha256`, `None`), poi il resto senza doppioni; kex: `diffie-hellman-group14-sha1`, `diffie-hellman-group-exchange-sha1` inseriti dopo `diffie-hellman-group14-sha256` e prima dei nomi-segnale (`ext-info-*`, `kex-strict-*`); cifrari: in coda `aes256-cbc`, `aes192-cbc`, `aes128-cbc`, `3des-cbc`; MAC: in coda `hmac-sha1-etm@openssh.com`, `hmac-sha1`.
  - `pub(super) struct HostKeyCheck` (`client::Handler`, `Error = russh::Error`): in `check_server_key` calcola `fingerprint(&key.to_bytes())` e il tipo (`key.algorithm().as_str()`), chiama `trust::check_in(dir, "ssh", host, port, &fp)`; `Known` → `Ok(true)`; altrimenti `trust::presented(..)`, salva `(Trust, fp)` in uno stato condiviso e `Ok(false)`. `connect` trasforma il rifiuto in `HOST_KEY_UNKNOWN`/`HOST_KEY_CHANGED` con `detail = fp`.
  - Autenticazione (stessa logica di oggi, `auth_plan` e `Answer` riscritti senza `ssh2`): `authenticate_none(user)`; `Success` → dentro; altrimenti `auth_plan(&methods, password.is_some())` con i nomi di `remaining_methods`; `Password` → `authenticate_password`; `KeyboardInteractive` → `authenticate_keyboard_interactive_start(user, None)` e, a ogni `InfoRequest`, `respond(answer(&prompts, &mut answered, pw))`; `answer` restituisce `[pw]` solo per un'unica domanda con `echo == false` e solo la prima volta, altrimenti tante stringhe vuote quante le domande. Rifiuto → `AUTH_FAILED` con `detail` fisso, mai il messaggio del server.
- Produces (`net::sftp::fake`, solo test):
  - `pub(super) struct FakeOptions { pub password: String, pub auth: FakeAuth /* Password | Keyboard | Both | NoneAccepted */, pub keys: Vec<ssh_key::Algorithm> /* default [Ed25519] */, pub posix_rename: bool /* true */, pub short_reads: bool /* false */, pub legacy_only: bool /* false */ }` con `Default` (password `"pw"`, `Password`).
  - `pub(super) fn start(opts: FakeOptions) -> (FakeServer, tokio::io::DuplexStream)`: `russh::server::run_stream` su un capo di `tokio::io::duplex(1 << 20)`, avviato con `rt::spawn`; al subsystem `sftp` avvia `russh_sftp::server::run` con l'handler del finto (in questo task: solo `init`, con l'estensione `posix-rename@openssh.com` = `"1"` se `posix_rename`). Chiavi generate con `PrivateKey::random` (RSA una volta per processo, in un `LazyLock`). Con `legacy_only` la `Config` del server offre solo `diffie-hellman-group14-sha1`, `aes128-cbc`, `hmac-sha1`.
  - `FakeServer::fingerprint(&self, alg: ssh_key::Algorithm) -> String` (con `ssh_key::PublicKey::fingerprint(HashAlg::Sha256)`), `FakeServer::kill(&self)` (chiude il canale e la sessione), `FakeServer::stall(&self)` (l'handler SFTP smette di rispondere).
  - `pub(super) fn conn() -> Connection`: `host "fake.test"`, `port 22`, `user "u"`.

- [ ] **Step 1: Test** in `connect.rs` (ogni test usa una cartella temporanea per `known_hosts` e `rt::block` con `Options::DEFAULT`):
  - `unknown_then_known_then_changed`: prima connessione → `HostKeyUnknown` con `detail` = `fake.fingerprint(Ed25519)`; `trust::accept_in(dir,"ssh","fake.test",22,&fp)`; seconda → `Ok`; un finto nuovo (altra chiave ed25519) → `HostKeyChanged`.
  - `fingerprint_matches_ssh_key`: il `detail` della prima connessione è uguale all'impronta calcolata da `ssh-key` sulla stessa chiave.
  - `legacy_entry_negotiates_rsa`: finto con `[Ed25519, Rsa]`; `known_hosts` scritto a mano con la sola impronta RSA del finto e senza `key_type` → `Ok`, nessun `HostKeyChanged`.
  - `saved_ed25519_negotiates_ed25519`: stesso finto, voce accettata dopo una chiave ed25519 presentata → `Ok`.
  - `preferred_puts_saved_type_first_and_legacy_last`: test puro su `preferred(Some("ssh-rsa"))` e `preferred(None)` (prima chiave `Ed25519`; ultimi cifrari e MAC come sopra).
  - `legacy_only_server_connects`: finto con `legacy_only` → `Ok`.
  - `auth_required_and_failed`: senza password → `AuthRequired`; `"sbagliata"` → `AuthFailed`; `"pw"` → `Ok`; con `NoneAccepted` e senza password → `Ok`.
  - `keyboard_interactive_only`: finto `Keyboard` → `Ok` con `"pw"`; il test unitario di `answer` riprende i casi di `keyboard_interactive_sends_the_password_once_to_a_single_hidden_prompt`.
  - `auth_failure_never_leaks_the_password`: password attesa `"pw"`, inviata `"pässwörd:ü"` → `AuthFailed`; né `detail` né `format!("{e:?}")` contengono `"pässwörd"`.
  - `posix_rename_detected`: `atomic` vero con `posix_rename`, falso senza.
  - `auth_without_password_is_required` (test di `auth_plan`) resta.
- [ ] **Step 2: Verificare che falliscano.**
- [ ] **Step 3: Implementare** `connect.rs` e `fake.rs`.
- [ ] **Step 4: Verificare:** `cargo test --manifest-path src-tauri/Cargo.toml sftp::connect` verde; `cargo tree -i aws-lc-rs` vuoto.
- [ ] **Step 5: Commit** (da confermare): `feat(net): connessione SSH con russh, impronta con il tipo di chiave e server finto per i test`.

### Task 5: Letture e scritture in parallelo

**Files:**
- Create: `src-tauri/src/net/sftp/stream.rs`
- Modify: `src-tauri/src/net/sftp/fake.rs` (file system in memoria)

**Interfaces:**
- Consumes: `Link`, `Connected`, `Options` (Task 4); `rt::block`, `rt::spawn` (Task 1); `errors::sftp_error` (Task 3).
- Produces (`net::sftp::stream`):
  - `pub(super) const IN_FLIGHT: usize = 16;`
  - `pub(super) async fn read_range(sftp: &RawSftpSession, handle: &str, offset: u64, len: u32, chunk: u32) -> Result<Vec<u8>, AppError>`: richieste da `chunk` byte, fino a `IN_FLIGHT` in volo, risultati in ordine; una risposta corta si completa con una richiesta del resto; `EOF` (stato `Eof` o dati vuoti) termina.
  - `pub(super) struct Reader` (`Read + Send`): `Reader::new(link: Arc<Link>, handle: String, path: String, chunk: u32, io_timeout: Duration)`; tiene letture anticipate a offset crescenti; ogni `read` è un `rt::block(io_timeout, ..)`. `Drop` chiude l'handle con `rt::spawn`.
  - `pub(super) struct Writer` (`Write + Send`): `Writer::new(link: Arc<Link>, handle: String, path: String, chunk: u32, io_timeout: Duration)`; `write` accoda richieste da `chunk` e aspetta la più vecchia solo con `IN_FLIGHT` richieste in volo; `flush` aspetta tutte le conferme, chiude l'handle e restituisce il primo errore; dopo un errore ogni `write`/`flush` lo ripete; dopo `flush` riuscito un `write` dà `InvalidRequest`. `Drop` senza `flush` chiude l'handle con `rt::spawn`.
  - Gli errori escono come `io::Error::other(AppError)` con il percorso (lo ritrova `vfs::provider::stream_error`).
- Il finto (`fake.rs`) diventa un file system in memoria: `MemFs` con nodi `File { data, mode, mtime }`, `Dir { mode, mtime }`, `Link { target }`; operazioni `open` (flag `CREATE`/`EXCLUDE`/`TRUNCATE`; `FAILURE` se `EXCLUDE` e il file c'è, come OpenSSH v3), `read` (con `short_reads` al massimo 1 000 byte per risposta), `write`, `close`, `lstat`, `stat` (segue i collegamenti), `setstat`, `opendir`/`readdir` (una pagina di nomi, poi `EOF`), `remove`, `mkdir` (`FAILURE` se esiste), `rmdir`, `rename` (`FAILURE` se la destinazione esiste), `symlink` (ordine della specifica: `linkpath`, `targetpath`), `extended("posix-rename@openssh.com")` (sostituisce la destinazione). Sotto `/ro` l'`open` in scrittura riesce ma ogni `write` dà `PERMISSION_DENIED` (così l'errore arriva in ritardo, come con le scritture in volo), e `mkdir`, `remove`, `rename`, `setstat` danno `PERMISSION_DENIED`; con `stall()` nessuna risposta. `FakeServer::put(path, bytes)` e `FakeServer::get(path) -> Option<Vec<u8>>` per preparare e controllare.

- [ ] **Step 1: Test** in `stream.rs` (finto con impronta già accettata; dati di prova = 600 KiB pseudo-casuali, più di `IN_FLIGHT × chunk`):
  - `reader_reads_everything_in_order`: `put` dei 600 KiB, `Reader` fino alla fine → stessi byte.
  - `reader_handles_short_reads_in_order`: come sopra con `short_reads`.
  - `read_range_from_offset`: `read_range` da 100 000 per 70 000 byte → la fetta giusta; da `len - 10` per 100 → 10 byte.
  - `empty_file_and_read_past_end`: file vuoto → `Reader` dà 0 al primo `read`; `read_range` oltre la fine → vuoto.
  - `writer_writes_everything_and_flush_closes`: 600 KiB in `write` da 8 KiB, `flush` → `get` uguale.
  - `write_error_surfaces_with_its_code`: `Writer` su `/ro/x` → `flush` (o un `write` successivo) dà `AccessDenied` tramite `stream_error`.
  - `writer_on_stalled_server_times_out`: `io_timeout` 300 ms, `stall()`, scrivere oltre `IN_FLIGHT` blocchi → `Timeout` in meno di 2 s; il `drop` del writer ritorna in meno di 100 ms.
  - `killed_connection_is_connection_failed`: `kill()` a metà lettura → `ConnectionFailed`.
- [ ] **Step 2: Verificare che falliscano.**
- [ ] **Step 3: Implementare** `stream.rs` e il file system del finto.
- [ ] **Step 4: Verificare:** `cargo test --manifest-path src-tauri/Cargo.toml sftp::stream` verde, più volte di fila (i test di tempo non devono essere instabili).
- [ ] **Step 5: Commit** (da confermare): `feat(net): letture e scritture SFTP in parallelo con errori esatti`.

### Task 6: Nuovo `SftpProvider` e addio a `ssh2`

**Files:**
- Modify: `src-tauri/src/net/sftp.rs` (riscritto), `src-tauri/Cargo.toml` (`cargo remove ssh2 libssh2-sys`)
- Delete: `src-tauri/src/net/sftp/raw.rs`

**Interfaces:**
- Consumes: tutto quanto sopra.
- Produces (`net::sftp`, firme pubbliche invariate): `pub struct SftpFactory` (`ProviderFactory`), `pub(crate) fn open_in(dir: &Path, conn: &Connection, password: Option<&str>) -> Result<SftpProvider, AppError>` (TCP con `net::tcp`, `set_nodelay(true)`, `set_nonblocking(true)`, `tokio::net::TcpStream::from_std` dentro il futuro, poi `open_on`), `#[cfg(test)] fn open_on<S>(dir, conn, password, stream: S, opts: &Options) -> Result<SftpProvider, AppError>` (`rt::block(opts.connect_timeout, connect(..))`), `pub fn fingerprint(key: &[u8]) -> Result<String, AppError>` invariata.
- `pub(crate) struct SftpProvider { link: Arc<Link>, atomic: bool, chunk: u32, io_timeout: Duration }`; niente più `Drop` proprio (la chiusura è nel `Drop` di `Link`, che parte quando provider e stream sono caduti tutti).
- Ogni metodo di `Provider` come oggi (commenti di `sftp.rs` da conservare), sulla `RawSftpSession`, dentro `rt::block(io_timeout, ..)`: `stat`/`list`/`to_stat` (`list` = `opendir` + `readdir` fino a `EOF` + `close`, senza `.` e `..`), `read_at` (`open` + `read_range` + `close`), `reader`/`writer` (`open` + `Reader`/`Writer`; `writer` con `WRITE|CREATE|EXCLUDE` o `WRITE|CREATE|TRUNCATE`, permessi 0o644), `rename` (`posix-rename@openssh.com` con `extended` e i due percorsi codificati come stringhe SSH; altrimenti `lstat` + `rename`), `delete`, `mkdir` 0o755, `set_meta` (`atime = mtime`), `exists_err` per `FAILURE` su un percorso esistente. `checked` rifiuta U+FFFD **e `\`** come oggi; il commento del modulo spiega il nuovo motivo di `\` (trasferimenti verso Windows).
- Test condivisi: `fn exercise(p: &SftpProvider, dir: &str, symlink: impl Fn(&str, &str))` con il corpo attuale di `sftp_live` (mkdir, writer `create_new`, `read_at`, reader, `set_meta`, stat, list, rename con e senza `atomic`, collegamenti, delete); `symlink` è il modo di creare un collegamento (sul finto con la `RawSftpSession` del provider).

- [ ] **Step 1: Test** in `sftp.rs`:
  - `provider_on_fake_with_posix_rename` e `provider_on_fake_without_posix_rename`: `open_on` sul finto (impronta accettata) e `exercise(&p, "/home/u", ..)`.
  - `unrepresentable_name_listed_but_never_sent`: il finto ha `"/d/caf\u{FFFD}.txt"` → `list("/d")` lo mostra con U+FFFD; `stat`, `read_at`, `delete` danno `InvalidName`. Il server di russh-sftp scrive solo `String`, quindi non può mandare byte non UTF-8: la decodifica lossy è di russh-sftp (`buf.rs`), qui si prova solo la parte dell'app.
  - `sftp_live` (`#[ignore]`, stessa `MC_TEST_SFTP`): `open_in` con impronta sconosciuta → `accept_in` → `AuthRequired` senza password → `AuthFailed` con una password sbagliata → `exercise`.
  - Restano: `unrepresentable_names_are_never_sent`, `server_mtime_never_overflows`, `fingerprint_format`. Si tolgono `sftp_status_mapping` (sostituito dal Task 3) e i test legati a `ssh2`.
- [ ] **Step 2: Verificare che falliscano.**
- [ ] **Step 3: Implementare**, togliere `raw.rs`, `cargo remove ssh2 libssh2-sys`.
- [ ] **Step 4: Verificare:** `cargo test --manifest-path src-tauri/Cargo.toml` verde (a parte i 3 test HTTP di loopback con la VPN attiva); `cargo tree -i libssh2-sys` e `cargo tree -i aws-lc-rs` → "did not match any packages"; `cargo build --release --manifest-path src-tauri/Cargo.toml` riuscita.
- [ ] **Step 5: Commit** (da confermare): `feat(net): provider SFTP su russh, chiavi dell'host ed25519 ed ECDSA`.

### Task 7: Documentazione e verifica su un server con sola chiave ed25519

**Files:**
- Modify: `docs/superpowers/fase-3a-follow-up.md`

- [ ] **Step 1: Follow-up:** la voce "SFTP: solo chiavi dell'host RSA" diventa risolta (russh 0.65, tipo di chiave ricordato, algoritmi datati in coda, nuovo `HOST_KEY_UNSUPPORTED`). Limiti noti: togliere "Un server SFTP che rifiuta un secondo canale non si collega"; "nomi con `\` o non UTF-8 visibili ma non apribili" resta, con il motivo nuovo di `\`; aggiungere "codici di stato SFTP oltre l'8 chiudono il canale (russh-sftp 3.0 conosce solo SFTP v3)". Verifiche manuali: server con sola chiave ed25519 (sshd in WSL con la sola `HostKey /etc/ssh/ssh_host_ed25519_key`, sull'IP della macchina WSL da `hostname -I`); server salvato con la versione precedente che resta noto senza richieste; server con soli algoritmi datati, se disponibile.
- [ ] **Step 2: Test live:** chiedere all'utente `MC_TEST_SFTP` per un server con sola chiave ed25519 (o il permesso di configurare sshd in WSL), poi `cargo test --manifest-path src-tauri/Cargo.toml -- --ignored sftp_live` → verde.
- [ ] **Step 3: Verifica finale:** `cargo test --manifest-path src-tauri/Cargo.toml`, `npm test`, `npm run build` verdi; `rg -n "ssh2|libssh2" src-tauri/src src-tauri/Cargo.toml` senza risultati.
- [ ] **Step 4: Commit** (da confermare): `docs: follow-up di SFTP su russh`.

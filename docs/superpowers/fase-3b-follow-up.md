# Fase 3b: verifiche manuali e follow-up

Data: 2026-10-28 · Branch: `fase-3b-archivi` (spec: `docs/superpowers/specs/2026-10-28-fase-3b-archivi-design.md`)

## Verifiche manuali da fare

Sulla build release. I test automatici coprono indici, lettori, scrittura degli ZIP e job con provider in memoria; non coprono archivi prodotti da altri programmi né server veri.

- **ZIP di Explorer e di 7-Zip con nomi accentati**: aprirli con Invio, controllare i nomi nell'elenco, estrarre una voce con F5 e l'archivio intero con Alt+F9, confrontare il contenuto.
- **7z "Ultra" di 7-Zip** (dizionario 64 MiB): aprirlo, elencarlo, estrarlo con Alt+F9; nessun `ARCHIVE_TOO_DEMANDING`. Un 7z con dizionario sopra 256 MiB deve dare l'errore tradotto.
- **ZIP di rilascio da 300 MB su un server Lepida**: aprirlo (Invio), elencarlo, estrarre una voce, aggiungere un file con F5. Misurare tempo e traffico: l'apertura deve leggere solo la directory centrale e l'estrazione solo la voce, non l'intero ZIP; l'aggiunta riscrive lo ZIP (una volta) e carica il nuovo file.
- **Alt+F9 su un 7z solido grande**: verificare che il blocco solido si decomprima una sola volta, in sequenza: tempo vicino a quello di 7-Zip e byte letti dal contenitore circa pari alla sua dimensione (non uno scorrimento dall'inizio per ogni voce).
- **F4 su una configurazione dentro uno ZIP remoto**: modifica, Ctrl+S, riapertura; il salvataggio compare come job nella vista Operazioni (avanzamento, annullabile), lo ZIP si riscrive e il resto del contenuto resta uguale. Con un altro job sullo stesso ZIP in corso il salvataggio aspetta che finisca.
- **FTP senza sostituzione atomica**: aggiunta di un file a uno ZIP. Controllare il comportamento descritto in "Limiti noti" (originale eliminato prima della rinomina; in caso di errore il messaggio nomina il temporaneo remoto).
- **Interruzioni**: annullare un job di scrittura su ZIP locale e remoto: nessun `.mcpart` residuo, originale intatto.
- **ZIP aperto in un altro programma**: la modifica dà `ACCESS_DENIED` e l'originale resta intatto.
- **Rinominare una cartella dopo aver navigato uno ZIP al suo interno**: entrare in `rel\a.zip`, aprire una voce con F3, tornare indietro, aspettare un paio di secondi; poi F5 Sovrascrivi su `a.zip` e Shift+F6 su `rel`: nessun `ACCESS_DENIED` (il contenitore si chiude dopo 1 s senza letture).
- **Caduta della VPN con una scheda d'archivio UNC aperta**: aprire `\\srv\share\a.zip`, staccare la VPN, F3 su una voce (errore), riattaccare la VPN e rientrare nell'archivio: si rilegge senza riavviare l'app.

## Stato dei test di riferimento

`cargo test` (anche con `--features test-hooks`): 3 test di `net::http_index` falliscono anche su `main`; `parallel_writes_never_leave_a_half_written_file` e `fs::watch::a_continuous_stream_still_notifies_within_max_wait` falliscono a volte (tempi). Gli altri passano, come `npm test` e `npx tsc --noEmit`; clippy senza avvisi nuovi.

## Limiti noti

Accettati in questa fase.

- **7z**: dizionario LZMA/LZMA2 (e memoria PPMd) oltre 256 MiB rifiutato con `ARCHIVE_TOO_DEMANDING`, come un'intestazione (in chiaro o decodificata) oltre 128 MiB; oltre 1.000.000 di voci per archivio `UNSUPPORTED` (`too_many_entries`). L'intestazione si legge e si controlla prima del crate (voci, folder, flussi compressi e sottoflussi oltre 1.000.000, più di 64 coder o flussi in un folder): quella codificata si decodifica solo se è Copy, LZMA o LZMA2 (7-Zip usa LZMA), con un altro metodo `UNSUPPORTED`. La memoria dell'indice resta quella del crate: fino a qualche centinaio di MiB per un 7z da un milione di voci. Intestazione di avvio tutta a zero (7z interrotto a metà scrittura) trattata come `CORRUPT_ARCHIVE`. I decoder 7z in attesa tengono il dizionario per 30 s (fino a 8 x 256 MiB nel caso peggiore).
- **TAR**: tetto di 128 MiB ai nomi dell'indice, che in memoria occupa circa 3 volte i byte dei nomi (percorso della voce, mappa per percorso, nomi dei figli): fino a circa 400 MiB al tetto; voci sparse (flag `S`) `UNSUPPORTED`; un TAR senza il padding finale è `CORRUPT_ARCHIVE` anche nell'ispettore. Un TAR.GZ remoto si scarica per intero (non c'è accesso casuale in un flusso gzip).
- **Cifratura**: archivi cifrati non supportati; un commit su uno ZIP con voci cifrate dà `UNSUPPORTED` (`encrypted`). RAR e archivi annidati esclusi.
- **Date**: sulle voci ZIP invariate di una riscrittura il campo UT (precisione al secondo) non si ricopia: resta la data DOS (2 s).
- **ZIP locale aperto da un altro programma**: la rinomina finale del commit riprova per circa 2 s (copre le letture brevi dell'app stessa, come l'ispettore che rielenca lo ZIP quando il `.mcpart` fa ricaricare la cartella, e l'antivirus sul file appena scritto); poi fallisce con `ACCESS_DENIED`, originale intatto (rinomina senza semantica POSIX). Un'estrazione lunga in corso sullo stesso ZIP supera l'attesa. Il contenitore locale risulta sempre scrivibile nelle `caps`: l'errore arriva al commit, non a comando disattivato. Idea: `FileRenameInfoEx` con semantica POSIX; `ReplaceFileW` per non perdere ACL e flussi alternativi (Mark-of-the-Web) che la rinomina con `MOVEFILE_REPLACE_EXISTING` oggi perde.
- **FTP/SFTP senza rinomina atomica**: la sostituzione di uno ZIP elimina l'originale prima della rinomina finale; se questa fallisce l'errore nomina il temporaneo remoto, che contiene lo ZIP completo.
- **Indice remoto non annullabile dalla lista**: l'indice costruito da `Vfs::remote` si interrompe solo da `list_archive`; list_dir, viewer e job non hanno un request id.
- **Ispettore**: elenca ZIP e 7z remoti ma non TAR e TAR.GZ remoti; il 7z resta su richiesta ("Mostra contenuto"); su un server senza letture a salti (HTTP senza `Range`) nemmeno ZIP e 7z (`UNSUPPORTED`), che andrebbero scaricati per intero.
- **ZIP64**: voci `Stored` ricomprese in Deflate alla riscrittura.
- **Cache degli indici**: 8 archivi, ricontrollo di dimensione e data del contenitore al più ogni 2 s; un errore di lettura del contenitore scarta l'archivio. Il file contenitore locale si apre a richiesta e si chiude dopo 1 s senza letture: una sostituzione o una rinomina fatta da altre operazioni (F5 Sovrascrivi, Shift+F6) subito dopo una lettura (entro 1 s) può ancora dare `ACCESS_DENIED`; il commit di uno ZIP invece riprova.
- **Coda**: un job in un archivio occupa la chiave dell'archivio e quella del volume del contenitore (unità, condivisione o connessione). Il salvataggio F4 di una voce è un job con sorgente nella cartella temporanea dell'app: aspetta anche i job sull'unità di `%TEMP%`.
- **Modifica senza elevazione**: i job che toccano un archivio non usano l'helper elevato.

## Piccoli difetti rimasti

Da valutare, nessuno blocca.

- Pack con file di destinazione già esistente e `replace=false`: `ALREADY_EXISTS` emerge al commit dopo lo staging, non all'avvio. Pack con sorgente dentro lo ZIP di destinazione dà `NOT_FOUND` invece di essere rifiutato in `expand`.
- Nome digitato nel prompt già preso da un altro elemento del job: sostituito in silenzio col primo libero.
- Errori fuori da `Counted` (file preparato mancante, CRC) lasciano `finalize` scrivere e chiamare `progress`.
- `normalize_name` accetta `/c:x` in scrittura ma l'indice lo marca `bad_name` in lettura.
- `install` tratta gli errori transitori di `stat` come "non esiste" (pattern già presente in `write`).
- Dopo un annullamento la copia di una voce si rifà al ritorno sulla scheda; il campione si legge dall'archivio prima della copia (doppia estrazione per 7z e TAR.GZ).
- Il menu contestuale non ha "Modifica" sulle voci non scrivibili (F4 sì).
- Validazione del contenitore diversa da Rust per indirizzi digitati (`\\srv\share`, `C:/x.zip`); doppio prompt password nel rimbalzo; confronto del contenitore case-sensitive nel rimbalzo.
- La logica busy/stale del `PackDialog` non ha test React; l'avviso di errore dell'enqueue compare sulla scheda attiva, non su quella d'origine.
- Duplicazioni di codice: copia a 64 KiB (`provider.rs` e `cursor.rs`), mappatura degli errori 7z (`cursor.rs` e `index.rs`).

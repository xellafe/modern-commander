# Fase 3b: verifiche manuali e follow-up

Data: 2026-10-28 · Branch: `fase-3b-archivi` (spec: `docs/superpowers/specs/2026-10-28-fase-3b-archivi-design.md`)

## Verifiche manuali da fare

Sulla build release. I test automatici coprono indici, lettori, scrittura degli ZIP e job con provider in memoria; non coprono archivi prodotti da altri programmi né server veri.

- **ZIP di Explorer e di 7-Zip con nomi accentati**: aprirli con Invio, controllare i nomi nell'elenco, estrarre una voce con F5 e con Alt+F9, confrontare il contenuto.
- **7z "Ultra" di 7-Zip** (dizionario 64 MiB): aprirlo, elencarlo, estrarlo con Alt+F9; nessun `ARCHIVE_TOO_DEMANDING`. Un 7z con dizionario sopra 256 MiB deve dare l'errore tradotto.
- **ZIP di rilascio da 300 MB su un server Lepida**: aprirlo (Invio), elencarlo, estrarre una voce, aggiungere un file con F5. Misurare tempo e traffico: l'apertura deve leggere solo la directory centrale e l'estrazione solo la voce, non l'intero ZIP; l'aggiunta riscrive lo ZIP (una volta) e carica il nuovo file.
- **Alt+F9 su un 7z solido grande**: misurare il tempo; il blocco solido si decomprime in sequenza, quindi il costo cresce con la posizione delle voci.
- **F4 su una configurazione dentro uno ZIP remoto**: modifica, Ctrl+S, riapertura; lo ZIP si riscrive e il resto del contenuto resta uguale.
- **FTP senza sostituzione atomica**: aggiunta di un file a uno ZIP. Controllare il comportamento descritto in "Limiti noti" (originale eliminato prima della rinomina; in caso di errore il messaggio nomina il temporaneo remoto).
- **Interruzioni**: annullare un job di scrittura su ZIP locale e remoto: nessun `.mcpart` residuo, originale intatto.
- **ZIP aperto in un altro programma**: la modifica dà `ACCESS_DENIED` e l'originale resta intatto.

## Limiti noti

Accettati in questa fase.

- **7z**: dizionario LZMA/LZMA2 (e memoria PPMd) oltre 256 MiB rifiutato con `ARCHIVE_TOO_DEMANDING`; oltre 1.000.000 di voci per archivio `UNSUPPORTED` (`too_many_entries`). Intestazione di avvio tutta a zero (7z interrotto a metà scrittura) trattata come `CORRUPT_ARCHIVE`. I decoder 7z in attesa tengono il dizionario per 30 s (fino a 8 x 256 MiB nel caso peggiore).
- **TAR**: tetto di 128 MiB ai nomi dell'indice; voci sparse (flag `S`) `UNSUPPORTED`; un TAR senza il padding finale è `CORRUPT_ARCHIVE` anche nell'ispettore. Un TAR.GZ remoto si scarica per intero (non c'è accesso casuale in un flusso gzip).
- **Cifratura**: archivi cifrati non supportati; un commit su uno ZIP con voci cifrate dà `UNSUPPORTED` (`encrypted`). RAR e archivi annidati esclusi.
- **Date**: sulle voci ZIP invariate di una riscrittura il campo UT (precisione al secondo) non si ricopia: resta la data DOS (2 s).
- **ZIP locale aperto da un altro programma**: il commit fallisce con `ACCESS_DENIED`, originale intatto (rinomina senza semantica POSIX). Il contenitore locale risulta sempre scrivibile nelle `caps`: l'errore arriva al commit, non a comando disattivato. Idea: `FileRenameInfoEx` con semantica POSIX; `ReplaceFileW` per non perdere ACL e flussi alternativi (Mark-of-the-Web) che la rinomina con `MOVEFILE_REPLACE_EXISTING` oggi perde.
- **FTP/SFTP senza rinomina atomica**: la sostituzione di uno ZIP elimina l'originale prima della rinomina finale; se questa fallisce l'errore nomina il temporaneo remoto, che contiene lo ZIP completo.
- **Indice remoto non annullabile dalla lista**: l'indice costruito da `Vfs::remote` si interrompe solo da `list_archive`; list_dir, viewer e job non hanno un request id.
- **Ispettore**: elenca ZIP e 7z remoti ma non TAR e TAR.GZ remoti; il 7z resta su richiesta ("Mostra contenuto"); con `ranged_read` falso scarica comunque ZIP e 7z remoti.
- **ZIP64**: voci `Stored` ricomprese in Deflate alla riscrittura.
- **Cache degli indici**: 8 archivi, ricontrollo di dimensione e data del contenitore al più ogni 2 s.
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

# Viewer F3: righe lunghe intere con scorrimento orizzontale

Data: 2026-10-19 · Stato: bozza da approvare · Riferimenti: piano fase 2 (`docs/superpowers/plans/2026-10-17-fase-2-vedere.md`), verifiche (`docs/superpowers/fase-2-follow-up.md`)

## Perché

Nella verifica manuale della fase 2, `one-line.json` (200 MB, una riga sola) compare spezzato in righe da 4096 byte (`MAX_LINE` in `src/lib/textPaging.ts`). L'utente vuole vedere una riga logica come **un'unica riga** e scorrerla in orizzontale, come fanno `less -S` o il Lister di Total Commander, **senza mai caricare la riga intera**.

## Obiettivi

- Con l'a capo **spento**, una riga del viewer coincide con una riga logica del file, cioè tutto ciò che sta tra due LF, qualunque sia la sua lunghezza.
- Lo scorrimento orizzontale è **allineato al carattere**: dopo lo scorrimento, lo stesso numero di colonna cade nella stessa posizione a schermo su tutte le righe, anche con lettere accentate UTF-8 e con tab. È stato scelto dall'utente al posto dello scorrimento a byte, dopo aver visto l'esempio del disallineamento.
- Le operazioni che devono attraversare molti byte (Fine su una riga da 200 MB, ↓ oltre una riga enorme, trascinamento delle barre) mostrano un **indicatore di avanzamento annullabile con Esc**. L'utente ha accettato che non siano istantanee.
- Le righe normali, come `big.log` e il codice, si comportano come oggi per fluidità e tempi.

## Non obiettivi

- L'a capo **attivo** resta com'è oggi, a spezzoni da `MAX_LINE`. Una riga da 200 MB a capo non si può mostrare per intero comunque.
- I caratteri a doppia larghezza (CJK) contano una colonna: con quei caratteri le colonne possono disallinearsi. Lo stesso vale per i caratteri combinanti e per le emoji composte.
- Ricerca nel testo, selezione su più schermate, modalità esadecimale: non cambiano.

## Anti-obiettivi (falliscono anche se "funzionano")

- Caricare una riga intera in memoria, nel frontend o nel backend, anche solo per contarne i caratteri.
- Bloccare la UI durante una scansione lunga, o non poterla interrompere.
- Peggiorare `big.log`: apertura sotto 300 ms, tasto tenuto premuto fluido (tasto → cursore < 16 ms), memoria della WebView sotto 300 MB.
- Mostrare come inizio riga un byte che non è davvero un inizio riga, con l'a capo spento.

## Comportamento

### Modello

- `ViewerState` (`src/state/viewers.ts`) ottiene `left: number`: la colonna del primo carattere visibile, uguale per tutte le righe e conservata per scheda come `top`. Con l'a capo attivo `left` vale 0 e viene ignorato.
- Con l'a capo spento `top` è **sempre** un inizio riga vero: offset 0, oppure il byte dopo un LF.
- Colonna: ogni carattere decodificato vale 1. Un tab porta alla colonna multipla di 8 successiva, con le colonne contate dall'inizio vero della riga. Il CR prima dell'LF non si mostra e non conta. Gli altri caratteri di controllo valgono 1. Il conteggio deve coincidere carattere per carattere con ciò che `TextDecoder` produce per quella codifica: una sequenza UTF-8 non valida vale quanti U+FFFD emette il decoder WHATWG, e una coppia surrogata UTF-16 vale 1.

### Tasti e mouse

| Input | A capo spento (nuovo) | Oggi |
|---|---|---|
| ↑ ↓ PgSu PgGiù | per righe logiche intere | per righe o spezzoni |
| ← → | di una colonna | nessun effetto |
| Ctrl+← Ctrl+→ | di una larghezza del viewport meno una colonna | nessun effetto |
| Home / Fine | inizio riga (`left = 0`) / fine della riga visibile più lunga | inizio / fine file |
| Ctrl+Home / Ctrl+Fine | inizio / fine file | inizio / fine file |
| Rotella | verticale come oggi; Maiusc+rotella e `deltaX` orizzontali | verticale |
| Esc | annulla l'operazione in corso, se c'è; altrimenti chiude la scheda come oggi | chiude |

Con l'a capo attivo, Home e Fine restano inizio e fine file come oggi.

### Barre di scorrimento

- La barra verticale (`ScrollRail`) resta proporzionale ai byte (`top / size`), ma con l'a capo spento **compare solo se c'è qualcosa da scorrere in verticale**: si nasconde quando la fine del file è nota e tutte le righe stanno nel viewport. È il caso di `one-line.json`, dove si naviga solo con la barra orizzontale. *(Deciso con l'utente: in un file di una riga la barra verticale non ha senso.)* Finché la fine del file non è nota, la barra resta visibile.
- Trascinando la barra verticale, `top` diventa l'**inizio della riga che contiene l'offset puntato** e `left` non cambia. Per trovarlo può servire una scansione all'indietro, con l'indicatore. Con righe normali il risultato è quello di oggi, a meno di una riga.
- Compare una barra orizzontale sotto il testo, solo con l'a capo spento e solo se una riga visibile è più larga del viewport. La sua estensione è la larghezza in colonne della riga visibile più lunga. Finché una riga lunga non è stata contata, la sua larghezza si stima così: i byte per UTF-8 e per le code page a un byte, i byte diviso 2 per UTF-16. La stima si corregge appena il conteggio esatto è noto. Trascinandola si sposta `left`.

### Indicatore di avanzamento

- Riguarda qualunque scansione del backend (inizio riga successiva o precedente, conteggio delle colonne) che duri più di circa 200 ms. Nella barra del viewer, al posto della percentuale di posizione, compare per esempio "Scansione 45 % · Esc annulla".
- **Scansioni automatiche.** Le scansioni partono anche senza un comando dell'utente, quando servono per disegnare la vista. Due esempi: la fine di una riga visibile che esce dalla finestra, all'apertura di `one-line.json`; l'offset della colonna `left` per una riga appena entrata nella vista. Mostrano lo stesso indicatore. Finché una riga non è risolta, al suo posto si vede una riga segnaposto attenuata ("…"). Le righe sopra, già note, si vedono subito. *(Scelta del progettista.)*
- **Esc** annulla tutte le scansioni in corso della scheda. Le righe non risolte restano segnaposto finché una nuova navigazione non le richiede. La vista non cambia e non compaiono errori. *(Scelta del progettista.)*
- **Navigazione durante una scansione.** Un tasto nella stessa direzione non annulla la scansione: si accoda e si somma. Per esempio, ↓ tenuto premuto su righe enormi avanza di tante righe quante sono le ripetizioni, man mano che le scansioni finiscono. Un tasto nella direzione opposta, un salto (Home/Fine, barre, Ctrl+Home/Fine) o Esc annullano le scansioni pendenti di quella navigazione. *(Scelta del progettista.)*
- **Quello che si scopre resta.** Gli LF trovati e i punti di controllo raccolti si conservano nella cache della scheda anche quando la scansione viene annullata a metà. Una scansione ripetuta riparte dal punto più avanzato già noto. *(Scelta del progettista.)*
- Cambiare scheda o chiudere il viewer annulla le scansioni di quella scheda.

### Codifiche

Tutte le codifiche di `ENCODINGS` (`src/lib/encoding.ts`) si contano a carattere, comprese quelle a due byte (`shift_jis`, `gbk`, `big5`, `euc-kr`). Il backend decodifica in streaming con `encoding_rs`, che implementa lo stesso standard WHATWG di `TextDecoder`: i caratteri contati, U+FFFD compresi, coincidono con quelli che il frontend mostra. `encoding_rs` è già una dipendenza diretta del backend (`src-tauri/Cargo.toml`, usata dal salvataggio dell'editor). *(Scelta del progettista, dopo la revisione: elimina il ripiego a byte per le code page a due byte, che non poteva trovare un confine di carattere affidabile senza decodificare.)*

## Come si fa: cosa è deciso

1. **Le scansioni lunghe stanno nel backend Rust** e leggono il file a blocchi, senza tenere in memoria più di un blocco. Sono annullabili con il registro esistente (`src-tauri/src/requests.rs`, `nextRequestId`/`cancelRequest` in `src/ipc/api.ts`) e riportano l'avanzamento su un `Channel`, come fa già `scan_tree`. Servono tre primitive:
   - **inizio della riga successiva** dopo `from`: il primo LF, rispettando l'unità di codice;
   - **inizio della riga** che contiene `at`: l'ultimo LF prima di `at`, cercando all'indietro;
   - **byte della colonna** `column` in una riga che inizia a `lineStart`, con la codifica data. Restituisce l'offset del carattere che contiene quella colonna e la colonna dove quel carattere inizia (diversa solo dentro un tab). Se la riga è più corta, restituisce la fine riga e il numero totale di colonne. Accetta un **punto di partenza noto** `(offset, colonna)` sulla stessa riga, per non ricontare dall'inizio, e restituisce **punti di controllo** `(offset, colonna)` raccolti durante il conteggio.
2. **Una sola implementazione del conteggio delle colonne**, in Rust. Il frontend non conta colonne: con `left = 0` non serve, perché il ritaglio parte da `lineStart`. Con `left > 0` chiede al backend l'offset di quella colonna per tutte le righe visibili insieme, in una chiamata sola.
3. **Punti di controllo per riga nel frontend:** una mappa `lineStart → punti (offset, colonna)` per scheda, alimentata dalle risposte del backend. Ogni richiesta parte dal punto noto più vicino prima della colonna cercata. ← e → ripartono dall'ultimo punto e costano pochi byte. Il costo pieno si paga una volta per riga: al primo Fine o al primo trascinamento lontano.
4. **Rendering di una riga:** si leggono pochi byte a partire dall'offset della colonna `left`, abbastanza per le colonne visibili più una. La lettura si ferma all'LF, si decodifica e si espandono i tab con le colonne assolute. Si aggiungono a sinistra gli spazi che servono quando `left` cade dentro un tab. Le letture delle righe visibili partono insieme, anche loro senza una chiamata IPC per riga.
5. **Righe dentro la finestra:** la finestra da 256 KiB di `TextPane` resta. Quando l'LF successivo è nella finestra, gli inizi riga si trovano come oggi senza backend. Il backend serve solo quando una riga esce dalla finestra.
6. **A capo attivo:** codice e comportamento di oggi (`splitLines`, `lineUp`, `seekLine` con `MAX_LINE`). Passando da a capo attivo a spento, `top` si riporta all'inizio vero della sua riga, con la primitiva 1.2 se serve.

## Lasciato al builder

- Nomi e firme dei comandi Tauri, formato dei punti di controllo e loro intervallo (indicativamente ogni 1 MiB letto), dimensione dei blocchi di lettura.
- Se le letture delle righe visibili usano un comando nuovo a intervalli multipli o `read_chunk` in parallelo, purché ← → restino sotto 16 ms su `one-line.json` con i punti di controllo già noti.
- Ritardo esatto prima di mostrare l'indicatore e testo nelle due lingue (`src/i18n/it.json`, `en.json`).
- Come organizzare `TextPane`: un pannello unico con due rami o un componente separato per l'a capo spento.
- Il passo della rotella orizzontale.

## Verifica

- **Test Rust:**
  - inizio riga successiva e precedente con unità 1 e 2, LF a cavallo di due blocchi, file senza LF, annullamento;
  - conteggio colonne in UTF-8 multibyte, UTF-8 non valido confrontato con l'output atteso di `TextDecoder`, UTF-16 con surrogate, tab, colonna dentro un tab, riga più corta della colonna;
  - ripartenza da un punto di controllo che dà lo stesso risultato del conteggio da zero.
- **Test Vitest:** navigazione verticale per righe intere con finestra e risposte del backend simulate, aggiornamento della mappa dei punti di controllo, rendering di un ritaglio con tab e spazi iniziali.
- **Verifiche manuali da aggiungere** a `docs/superpowers/fase-2-follow-up.md`:
  - `one-line.json` mostra una riga sola; Fine porta in fondo alla riga con l'indicatore ed Esc lo annulla; ← → fluidi dopo;
  - `big.log` invariato per tempi e memoria;
  - un TSV con tab e un testo accentato restano allineati dopo lo scorrimento;
  - su `one-line.json` la barra verticale non compare; su un file con poche righe enormi, trascinandola a metà si arriva all'inizio della riga che contiene quel punto.

## Fuori da questo lavoro

La correzione del layout della shell (`grid-template-columns: minmax(0, 1fr)` in `src/components/shell/shell.css`) che impediva al gruppo di sparire con le righe lunghe è già fatta, a parte, durante la verifica manuale.

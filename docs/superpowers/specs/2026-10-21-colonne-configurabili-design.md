# Lista file: colonne configurabili ed estensione nel nome

Data: 2026-10-21 · Stato: approvata · Riferimenti: spec generale §5.3 (`docs/superpowers/specs/2026-10-16-modern-commander-design.md`), bozza interattiva approvata (`.superpowers/brainstorm/1157-1792587513/content/colonne.html`, non versionata)

## Perché

Lo spec generale dice che le colonne della lista "sono configurabili", ma nessuna fase le ha implementate: oggi la vista dettagliata ha sei colonne fisse (`grid-template-columns: 16px minmax(0, 1fr) 52px 72px 112px 36px` in `src/components/groups/groups.css`). Durante le verifiche della fase 2 l'utente ha chiesto di poter **decidere quali colonne vedere**, di cambiarne **ordine e larghezza**, di avere colonne in più e di poter mostrare **l'estensione attaccata al nome** invece che in una colonna separata.

## Obiettivi

- Click destro sull'intestazione della lista → menu con tutte le colonne, spuntate se visibili; un click accende o spegne la colonna.
- Larghezza delle colonne cambiabile trascinando il bordo destro dell'intestazione.
- Ordine delle colonne cambiabile trascinando l'intestazione.
- Due colonne nuove gratuite (date di creazione e di ultimo accesso) e una colonna Tipo (descrizione di Windows).
- Opzione "Estensione nel nome".
- La configurazione è **unica e globale**: vale per tutte le schede e tutti i gruppi e sopravvive al riavvio.

## Non obiettivi

- Colonna Proprietario: esclusa dall'utente.
- Versione del file (exe/dll), dimensioni delle immagini e simili colonne che richiedono di aprire ogni file: si potranno aggiungere più avanti.
- Configurazione per scheda, per cartella o per tipo di cartella (i "set di colonne" di TC).
- Adattamento automatico della larghezza con doppio click sul bordo.
- Viste breve e miniature: non cambiano.
- Pagina Impostazioni: nessuna voce nuova, il menu sull'intestazione basta.

## Anti-obiettivi (falliscono anche se "funzionano")

- Rallentare input → cursore oltre 16 ms o rendere più lenta la lettura di una cartella grande per colpa delle colonne.
- Far aspettare il cursore o la comparsa delle righe su un dato di colonna (il Tipo arriva dopo, mai prima delle righe).
- Cambiare le scorciatoie di TC: Ctrl+F3…F6 e Ctrl+F1/F2 restano come sono.
- Un'intestazione che non si allinea più alle righe.

## Catalogo delle colonne

| Id | Etichetta (it / en) | Default | Larghezza default | Allineamento | Ordinabile | Fonte del dato |
|---|---|---|---|---|---|---|
| `icon` | — | sempre visibile, fissa | 16 px | centro | no | come oggi |
| `name` | Nome / Name | sempre visibile | il resto (`minmax(80px, 1fr)`) | sinistra | sì (`name`) | come oggi |
| `ext` | Est. / Ext | visibile | 52 px | sinistra | sì (`ext`) | come oggi |
| `size` | Dim. / Size | visibile | 72 px | destra | sì (`size`) | come oggi |
| `modified` | Modificato / Modified | visibile | 112 px | destra | sì (`date`) | come oggi |
| `created` | Creato / Created | nascosta | 112 px | destra | sì (`created`) | nuovo, da `FindFirstFileExW` |
| `accessed` | Ultimo accesso / Accessed | nascosta | 112 px | destra | sì (`accessed`) | nuovo, da `FindFirstFileExW` |
| `attributes` | Attr. / Attr | visibile | 36 px | destra | no | come oggi |
| `type` | Tipo / Type | nascosta | 130 px | sinistra | sì (`type`) | nuovo, da Windows per estensione |

L'ordine della tabella è l'ordine di default. Nel menu le colonne compaiono con l'etichetta estesa ("Data di creazione", "Data di ultimo accesso", "Data di modifica"…), nell'intestazione con quella breve; i testi esatti li sceglie il builder, in italiano e inglese.

## Comportamento

### Intestazione e menu

- **Click destro sull'intestazione** apre un menu nello stile di `.mc-menu`, titolo "Colonne":
  - una voce per colonna (tranne l'icona) con la spunta se visibile; "Nome" è sempre spuntato e disattivato ("sempre visibile");
  - separatore, poi "Estensione nel nome" con spunta;
  - separatore, poi "Ripristina colonne predefinite": torna al catalogo di default (sei colonne di oggi, larghezze e ordine di default, estensione nel nome spenta).
  - Il menu resta aperto dopo un click su una voce, così si possono cambiare più colonne di fila; si chiude con Esc o con un click fuori.
- Il click destro sulle righe e sullo spazio vuoto della lista continua ad aprire il menu della shell come oggi.
- **Click sinistro** su un'intestazione ordinabile ordina per quella colonna; un secondo click inverte il verso (`nextSort`, come oggi). La freccia del verso compare sull'intestazione della colonna ordinata.

### Larghezza

- Ogni colonna tranne icona e Nome ha una maniglia sul bordo destro dell'intestazione: trascinandola cambia la larghezza, in tempo reale su intestazione e righe.
- Larghezza minima 28 px per le colonne, 80 px per il Nome. Il Nome prende sempre lo spazio rimanente; se le colonne non ci stanno, il Nome si restringe con i puntini come oggi e oltre il minimo il contenuto viene tagliato. Niente scorrimento orizzontale: col minimo di 80 px la griglia può superare la larghezza della riga, quindi `.mc-list-scroll` della vista dettagliata deve nascondere l'overflow orizzontale (oggi è `overflow: auto`).
- La larghezza si salva al rilascio del mouse, non durante il trascinamento.

### Ordine

- Trascinando un'intestazione (Nome compreso, icona esclusa) la si sposta; durante il trascinamento un segno nel colore di risalto indica dove cadrà, come per le schede (`drop-before` / `drop-after`).
- L'icona resta sempre la prima colonna.

### Estensione nel nome

- Accesa: la colonna Nome mostra il nome completo (`README.md`), la colonna Estensione sparisce e la sua voce nel menu è disattivata (senza spunta). Spegnendo l'opzione la colonna Estensione torna com'era (visibile o no, nella sua posizione).
- Bozza approvata: l'estensione nel nome è nel colore attenuato (`--mc-muted`) dopo il nome base. I nomi marcati restano interamente nel colore di risalto.
- L'ordinamento non cambia: per nome resta sul nome completo, Ctrl+F4 continua a ordinare per estensione anche con la colonna nascosta.
- Shift+F6 seleziona già solo il nome senza estensione e resta così.
- Cartelle, `..` e unità non cambiano (non hanno estensione da separare).

### Righe speciali

- Riga `..`: tutte le colonne vuote tranne icona e Nome, come oggi.
- Unità (pseudo-cartella `drives:`): oggi l'etichetta e lo spazio libero occupano le colonne da 3 in poi (`.c-drive { grid-column: 3 / -1 }`). Devono continuare a occupare tutte le colonne visibili dopo il Nome, qualunque sia l'ordine. Se il Nome non è subito dopo l'icona, il builder sceglie la soluzione più semplice che resti leggibile.

### Viste

- Le colonne riguardano solo la vista dettagliata. Breve e miniature restano come sono e mostrano il nome completo come oggi.

## Dati

### Date di creazione e di ultimo accesso

- `Entry` (Rust `src-tauri/src/fs/listing.rs`, TS `src/ipc/types.ts`) ottiene `created_ms` / `createdMs` e `accessed_ms` / `accessedMs`, letti da `ftCreationTime` e `ftLastAccessTime` della stessa `WIN32_FIND_DATAW` che si usa già: nessuna chiamata di sistema in più.
- I due campi viaggiano **sempre**, anche con le colonne spente (scelta del designer, la più semplice): sono circa 55 byte di JSON in più per voce. Il builder misura la lettura di una cartella da 10.000 voci prima e dopo; se il budget dello spec generale (meno di 150 ms) non regge più, `list_dir` riceve un parametro per includere le date solo quando una colonna o l'ordinamento le usano, e le schede aperte si rileggono quando si accendono.
- Le righe sintetiche (`..` in `buildRows`, unità in `tabViews.ts`) li mettono a 0 e non li mostrano.
- Limite noto, non da correggere: Windows spesso non aggiorna la data di ultimo accesso (`NtfsDisableLastAccessUpdate`), quindi può essere vecchia.

### Tipo

- Testo di Windows per l'estensione, ottenuto con `SHGetFileInfoW` + `SHGFI_TYPENAME | SHGFI_USEFILEATTRIBUTES` sul solo nome fittizio (`*.ext`): non tocca il disco e non apre il file. Le cartelle hanno il testo di Windows per una cartella ("Cartella di file"); i file senza estensione quello che Windows restituisce per "File".
- Nuovo comando IPC che riceve **un elenco di estensioni** e restituisce i testi, così una cartella costa una chiamata sola.
- L'estensione è quella che il codice usa già (`extOf` / `splitName`): `.gitignore` non ha estensione e prende il Tipo dei file senza estensione (scelta del designer, coerente con la colonna Estensione).
- Cache nel frontend `estensione → testo` per tutta la sessione. Quando arriva un listing, oppure quando si accende la colonna Tipo con schede già caricate, e la colonna Tipo è visibile o l'ordinamento è per Tipo, si chiedono in un'unica chiamata le estensioni non ancora in cache; le righe compaiono subito con il Tipo vuoto e si riempiono all'arrivo, ri-ordinando se l'ordinamento è per Tipo.
- Il comando gira fuori dal thread principale; se fallisce per un'estensione, il Tipo resta vuoto.

### Ordinamento

- `SortKey` ottiene `created`, `accessed`, `type`. `buildRows` confronta le date numericamente e il Tipo con lo stesso `collator` del nome; a parità decide il nome, come oggi. Le cartelle restano prima dei file.
- Il Tipo non ancora arrivato conta come stringa vuota.
- Nessun nuovo comando `cm_*` né scorciatoia: si ordina per le colonne nuove solo col click sull'intestazione.

## Salvataggio

- In `Settings` (`src/state/settings.ts`), quindi nel file `settings` già scritto da `writeConfig`:
  - `columns`: elenco ordinato di `{ id, visible, width }` per tutte le colonne del catalogo tranne l'icona; il Nome ha `width` nulla.
  - `extInName: boolean`, default `false`.
- Va aggiornata `pickSettings`. Alla lettura (`load`) un elenco `columns` salvato va normalizzato: id sconosciuti scartati, colonne del catalogo mancanti aggiunte in fondo con i valori di default, Nome forzato visibile, larghezze riportate sopra il minimo. Così una versione futura con una colonna in più non rompe le impostazioni vecchie.

## Prestazioni

- Le righe restano `memo` e virtualizzate. Le colonne visibili e le larghezze arrivano alla riga come un unico valore stabile (per esempio la stringa di `grid-template-columns` e l'elenco degli id), così spostare il cursore ridisegna sempre e solo le due righe che cambiano classe.
- Il trascinamento di una larghezza può ridisegnare tutte le righe visibili: è accettabile, ma non deve scrivere le impostazioni a ogni movimento.

## Verifica

- Test unitari: normalizzazione delle colonne salvate; `buildRows` con i nuovi `SortKey`; costruzione del `grid-template-columns` dalla configurazione; test Rust che le due date nuove non siano zero su un file creato dal test.
- A mano, nell'app: menu, nascondi/mostra, ridimensiona, sposta, ripristina, estensione nel nome, riavvio con configurazione conservata, Tipo su una cartella con molte estensioni, unità, riga `..`, rinomina in place con estensione nel nome, tasto tenuto premuto su una cartella grande con tutte le colonne accese (nessun peggioramento rispetto a oggi).

## Lasciato al builder

- Testi esatti di etichette e voci del menu nelle due lingue.
- Come realizzare il trascinamento dell'intestazione (eventi pointer o drag-and-drop HTML) purché coerente con quello delle schede.
- Struttura del codice delle colonne (un piccolo catalogo con etichetta, allineamento, chiave di ordinamento e funzione di cella va bene).
- Soluzione per la riga delle unità quando il Nome non è subito dopo l'icona.
- Formato esatto del comando IPC per il Tipo.

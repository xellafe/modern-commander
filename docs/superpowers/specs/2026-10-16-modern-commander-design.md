# Modern Commander — documento di design

Data: 2026-10-16 · Stato: bozza da approvare · Nome del progetto: provvisorio

Convenzione: **[U]** = deciso dall'utente · **[S]** = scelta del designer, modificabile in revisione.

## 1. Intento

È un file manager a doppio pannello (o meglio, a N pannelli) per Windows. Offre le funzioni di Total Commander con un'interfaccia moderna, nello stile di VS Code e di Claude Code Desktop. L'utente lo userà tutti i giorni al posto di TC. **[U]**

Il motivo: TC funziona bene, ma è poco intuitivo e poco gradevole da vedere ("anche l'occhio vuole la sua parte"). **[U]** Il progetto riesce quando l'utente sceglie di aprirlo al posto di TC perché l'esperienza d'uso è più piacevole. **[U]**

## 2. Obiettivi, non-obiettivi, anti-obiettivi

**Obiettivi**
- Le funzioni di TC elencate in §6, con le scorciatoie di TC come default, tutte rimappabili. **[U]**
- Più di due pannelli affiancati, organizzati come gli editor group di VS Code, ognuno con le sue schede. **[U]**
- Funzioni moderne: command palette, ricerca fuzzy, anteprima ricca, integrazione git, terminale integrato, tag e colori, workspace. **[U]**
- Velocità: "se è veloce è meglio". Obiettivi numerici in §7.4. **[U]/[S]**
- Interfaccia in inglese e italiano, pronta per aggiungere altre lingue. **[U]**
- Progetto open source, distribuito sia come portable sia con installer. **[U]**

**Non-obiettivi per la v1** **[U]**
- Cloud storage (OneDrive, S3, Google Drive).
- Funzioni AI.
- Plugin: né compatibilità con quelli di TC (WCX/WLX/WFX/WDX), né un sistema di estensioni proprio.
- Auto-update.
- macOS e Linux: solo Windows (10/11 x64).

**Non-obiettivi per la v1** **[S]**
- La barra dei pulsanti personalizzabile di TC (button bar).
- La riga di comando separata di TC: la sostituisce il terminale integrato (§5.6). **[U, confermato]**
- Undo generale delle operazioni su file. Fa eccezione l'undo dell'ultimo multi-rename (§6.4).
- Operazioni git di scrittura (stage, commit, push): in v1 git è in sola lettura (§6.5).

**Anti-obiettivi** (cose che renderebbero il progetto un fallimento anche se "funziona")
- Un clone di TC fedele ma brutto o ostico. L'aspetto conta quanto le funzioni.
- Un'app lenta da avviare o che rallenta con cartelle grandi, cioè la sensazione "Electron pesante".
- Scorciatoie di TC che non funzionano o fanno altro: la memoria muscolare dell'utente va rispettata.
- Operazioni su file che perdono dati in silenzio (conflitti, errori, copie interrotte).

## 3. Vincoli

- **Piattaforma:** solo Windows. Si possono usare liberamente le API Win32/COM. **[U]**
- **Licenza:** open source, **MIT**. Le dipendenze devono avere licenze compatibili con MIT: niente GPL (vedi §8, punto unrar). **[U]**
- **Distribuzione:** portable (zip) e installer, senza auto-update. **[U]**
- **Repo:** `D:\Generale\modern-commander`, branch `main`. Commit solo su richiesta esplicita dell'utente.
- **Lingua del progetto:** documentazione, commenti e messaggi di commit in italiano; nomi nel codice in inglese.

## 4. Stack tecnico **[S]**, scelta delegata dall'utente e confermata

- **Tauri 2**: backend in Rust, UI in **React + TypeScript** (Vite) dentro WebView2.
  - Il Rust gestisce file system, operazioni, archivi, rete, git, PTY e API Windows (menu contestuale, cestino, drag & drop OLE).
  - La UI web rende naturale lo stile VS Code e permette di riusare componenti collaudati.
- **Perché non le alternative:** Electron è più pesante e lento all'avvio, il che va contro un anti-obiettivo. WinUI 3 / Avalonia sono più rigidi da rendere "tech" e non hanno equivalenti pronti di Monaco e xterm.js.
- **Componenti UI suggeriti** (il builder li verifica e sceglie le versioni col package manager):
  - lista virtualizzata (es. TanStack Virtual)
  - Monaco Editor (editor F4 e diff)
  - xterm.js (terminale)
  - pdf.js (anteprima PDF)
  - un evidenziatore di sintassi leggero per l'ispettore (es. Shiki)
  - i18next
- **Crate Rust suggeriti** (stesse regole):
  - `windows` (Win32/COM)
  - `notify` (watcher)
  - `zip`, `sevenz-rust`, `tar` + `flate2`
  - `suppaftp` (FTP/FTPS), `russh` o `ssh2` (SFTP)
  - `gix` o `git2`
  - `portable-pty` (ConPTY)
  - `rusqlite`
  - `keyring` o Credential Manager diretto

## 5. Interfaccia **[U]** salvo dove indicato

Riferimento visivo: le bozze `layout-v2.html` e `visual-style.html` (variante A per il tema scuro, C per il chiaro) in `.superpowers/brainstorm/`, escluse dal repo.

### 5.1 Struttura della finestra (dall'alto in basso)

1. **Barra del titolo personalizzata**:
   - campo di ricerca centrale che apre la command palette
   - pulsanti per mostrare o nascondere barra laterale, terminale e ispettore
   - controlli finestra standard
2. **Corpo:**
   - **Barra delle attività** a sinistra (icone): Esplora, Cerca, Git, Preferiti, Impostazioni.
   - **Barra laterale** (si può nascondere), con il contenuto della vista scelta. La vista Esplora mostra Unità (locali, di rete mappate, removibili), un albero delle cartelle e i Preferiti.
     - **Cliccando una cartella nella barra laterale la si apre nella scheda attiva del gruppo attivo.** **[U]**
   - **Area dei gruppi**: N gruppi affiancati orizzontalmente, ognuno con barra delle schede, pulsante "+" accanto alle schede, breadcrumb del percorso e lista dei file.
   - **Ispettore** a destra (si può nascondere, §5.5).
3. **Pannello in basso** (si può nascondere), con due schede: Terminale e Operazioni (§5.6, §6.2).
4. **Barra delle F-key** ridisegnata: F3 Vedi, F4 Modifica, F5 Copia, F6 Sposta, F7 Cartella, F8 Elimina, Alt+F4 Esci. Le etichette cambiano mentre si tengono premuti Shift, Ctrl o Alt, come in TC. **[S]**
5. **Barra di stato**: branch git, selezione (numero di elementi e dimensione), avanzamento dell'operazione in corso, spazio libero sull'unità.

### 5.2 Gruppi e schede

- Gruppo attivo: la scheda attiva ha il bordo superiore nel colore di risalto; gli altri gruppi sono attenuati.
- Si crea un nuovo gruppo con il comando "Dividi" o trascinando una scheda sul bordo di un gruppo. Ctrl+\\ di VS Code non si può usare perché in TC porta alla cartella radice. Il builder sceglie un tasto libero nella mappa di TC (§5.8). **[S]**
- Le schede si possono riordinare e spostare tra gruppi. "+" apre una nuova scheda sulla cartella corrente.
- Non c'è un numero massimo di gruppi. La larghezza minima di un gruppo la decide il builder.
- **Destinazione di F5/F6:** il gruppo attivo più di recente tra quelli diversi da quello corrente, che con due gruppi equivale "all'altro pannello" di TC. Con tre o più gruppi la finestra di copia mostra la destinazione proposta e permette di cambiarla scegliendo il gruppo (anche con i tasti 1…9). **[U, confermato]**
- Tab passa al gruppo successivo, come in TC; Shift+Tab al precedente. **[S]**

### 5.3 Lista dei file

- Colonne di default: icona, Nome, Estensione, Dimensione, Data di modifica, Attributi. Sono configurabili. Ordinamento per click sull'intestazione o con Ctrl+F3…F6, come in TC.
- Viste: dettagliata (default), breve e miniature. Le scorciatoie sono quelle di TC.
- Marcatura e cursore sono distinti, come in TC: Ins/Spazio marcano, Num+ e Num- selezionano per maschera, Num* inverte, Ctrl+A seleziona tutto. Gli elementi marcati hanno il nome nel colore di risalto.
- Ricerca rapida: digitando lettere il cursore salta al file. In TC di fabbrica le lettere vanno alla riga di comando, che qui non c'è, quindi le lettere sono libere; funziona anche Ctrl+Alt+lettera di TC. Ctrl+S attiva il filtro rapido della lista, come in TC. **[S]**
- Badge git accanto al nome: M, U, A, D; gli ignorati sono attenuati.
- I tag sono un pallino colorato accanto al nome.
- Aggiornamento automatico tramite watcher del file system.

### 5.4 Stile visivo **[U]**

- **Due temi**: scuro = variante A, chiaro = variante C. Si scelgono nelle impostazioni; c'è anche l'opzione "segui Windows" **[S]**.
- Variante A (scuro): icone monocromatiche al tratto, righe fitte (circa 22 px), accento arancio (circa `#e07a4f`), spigoli vivi, neutri scuri (circa `#0e1013` / `#15171c`).
- Variante C (chiaro): palette crema (circa `#faf9f5` / `#f0eee6`), accento terracotta (circa `#c96442`), righe più ariose, angoli morbidi, icone colorate tenui.
- *Ipotesi del designer da confermare:* la densità delle righe e lo stile delle icone sono legati al tema, come nelle bozze, e non sono impostazioni separate.
- Font dell'interfaccia: Segoe UI Variable / Segoe UI. Font monospazio: Cascadia Code / Consolas. **[S]**
- Il colore dei temi passa da design token (variabili CSS), così aggiungere temi in futuro non richiede refactoring. **[S]**

### 5.5 Ispettore **[U]**

- Ctrl+Q lo mostra o lo nasconde, come il Quick View di TC. Mostra il file sotto il cursore del gruppo attivo:
  - immagini
  - PDF (prima pagina, poi si scorre)
  - codice con evidenziazione della sintassi
  - Markdown renderizzato
  - audio e video HTML5
  - per le cartelle: dimensione, numero di file e sottocartelle
  - per gli archivi: elenco dei contenuti
- Metadati: dimensione, date, attributi, stato git (con diff rispetto a HEAD per i file di testo), tag, hash calcolato su richiesta.
- L'anteprima si carica in modo asincrono e cancellabile: muovere il cursore velocemente non deve bloccare la UI.

### 5.6 Terminale **[U]**

- Ctrl+ò lo mostra o lo nasconde. Il tasto va legato al virtual key `VK_OEM_3`, che sulla tastiera italiana è "ò", così coincide con Ctrl+` di VS Code. **[S]**
- **Più schede di terminale** (raccomandazione del designer, accettata). Ogni nuova scheda parte nella cartella del gruppo attivo. `exit` chiude la scheda; chiudere l'ultima scheda chiude il pannello.
- Shell di default configurabile: PowerShell 7 se presente, altrimenti Windows PowerShell; poi cmd e WSL. **[S]**
- Sostituisce la riga di comando di TC.

### 5.7 Command palette e ricerca fuzzy **[S]**

- **Ctrl+Shift+P** apre la command palette: tutti i comandi, ognuno con la sua scorciatoia, ricerca fuzzy.
- **Ctrl+P** apre il salto rapido: ricerca fuzzy tra le cartelle recenti, i preferiti e i file sotto la cartella corrente (scansione ricorsiva incrementale con limite di profondità e di numero). In TC Ctrl+P copia il percorso nella riga di comando, che qui non esiste, quindi il tasto è libero.

### 5.8 Scorciatoie

- Default = mappa **di fabbrica** di Total Commander: l'utente non ha personalizzato le scorciatoie, quindi l'import da `wincmd.ini` resta in fase 5. Il builder ricava la mappa dalla documentazione ufficiale di TC (help "Keyboard") e la registra come tabella dei comandi. **[U]**
- Remapping: pagina delle impostazioni con ricerca e cattura del tasto, più un file `keybindings.json` modificabile a mano. **[U]/[S]**
- Le scorciatoie nuove non devono sostituire quelle di TC. Il builder controlla ogni nuova scorciatoia proposta qui (Ctrl+Shift+E, Ctrl+Shift+P, Ctrl+ò, "Dividi") sulla mappa di TC; in caso di conflitto vince TC e si sceglie un altro tasto. **Correzione rispetto alla discussione:** avevo proposto Ctrl+B per la barra laterale, ma in TC Ctrl+B è la "branch view". La barra laterale va quindi su **Ctrl+Shift+E** **[S]**, e Ctrl+B resta la branch view (lista piatta di tutti i file nelle sottocartelle).

## 6. Funzionalità

### 6.1 Navigazione
- Breadcrumb cliccabile; cliccando sullo spazio vuoto diventa un campo di testo modificabile.
- Indietro e avanti (Alt+←/→), storico delle cartelle (Alt+↓), cartella superiore (Backspace).
- Hotlist dei preferiti (Ctrl+D) con cartelle e sottomenu, sincronizzata con la vista Preferiti.
- Unità di rete, percorsi UNC e percorsi lunghi oltre 260 caratteri (prefisso `\\?\` nel backend).

### 6.2 Operazioni su file e coda
- F5 copia, F6 sposta; Shift+F6 rinomina in place; F7 crea una cartella; F8/Canc elimina nel cestino; Shift+F8/Shift+Canc elimina in modo definitivo.
- Prima di copiare o spostare si apre una finestra con la destinazione modificabile, il filtro e le opzioni. La gestione dei conflitti offre: sovrascrivi, salta, rinomina, solo se più recente, sovrascrivi se più grande, ricorda per tutti.
- Tutte le operazioni lunghe vanno in una **coda in background**: avanzamento per file e totale, velocità, tempo rimanente, pausa/riprendi/annulla, riordino.
  - Di default la coda è sequenziale per unità fisica e parallela tra unità diverse **[S]**.
  - L'avanzamento sintetico è nella barra di stato, il dettaglio nella scheda "Operazioni".
- Gli errori non si perdono mai: ogni file in errore dà le scelte riprova/salta/annulla e compare in un riepilogo finale.
- Una copia interrotta non deve lasciare un file di destinazione troncato che sembra completo: si scrive su un file temporaneo e lo si rinomina alla fine, oppure lo si elimina in caso di annullamento.
- Copia con `CopyFileExW` (progresso, annullamento, attributi e ADS conservati) o equivalente; spostamento sulla stessa unità come rename. **[S]**
- **Elevazione (UAC) in v1, fase 1** **[U]**:
  - L'app gira senza privilegi. Quando un'operazione fallisce per accesso negato, oltre a riprova/salta/annulla si offre "Riprova come amministratore", per il singolo file o per tutti i file rimanenti dell'operazione. Vale anche per F7 e per la rinomina.
  - Al primo uso si avvia un **processo helper elevato**: lo stesso eseguibile con un argomento dedicato, lanciato con `ShellExecuteEx` + verbo `runas`, quindi compare il prompt UAC. L'helper resta vivo finché ci sono operazioni elevate, poi esce dopo un periodo di inattività.
  - **Sicurezza** (non semplificabile) **[S]**:
    - L'IPC passa per una named pipe con ACL limitata al SID dell'utente corrente e un token monouso passato all'avvio.
    - L'helper esegue solo le primitive su file della coda (copia, sposta, elimina, crea cartella, rinomina, attributi, elenco in sola lettura di una cartella; l'elenco serve per "Riprova come amministratore" sulle cartelle illeggibili). Mai comandi arbitrari, shell o caricamento di codice.
    - I percorsi vengono validati e normalizzati nell'helper.
  - La coda tratta le operazioni elevate come le altre: avanzamento, pausa e annullamento passano per la pipe.
- Calcolo della dimensione delle cartelle: Spazio su una cartella la marca e ne mostra la dimensione nella colonna. Esiste anche un comando "calcola tutte".
- Menu contestuale **nativo di Windows** con il tasto destro, Shift+F10 o il tasto Menu. **[U]**
- Drag & drop: tra gruppi e schede; verso Explorer e altre app (OLE); da Explorer verso l'app. **[U]**
- Per i file dentro archivi e FTP/SFTP **[S]**:
  - il menu contestuale è quello dell'app, non quello nativo
  - il drag verso Explorer estrae o scarica prima i file in una cartella temporanea e poi passa quei percorsi, come fa TC
  - niente "file virtuali" OLE (`CFSTR_FILEDESCRIPTOR`) in v1

### 6.3 Visualizzazione e modifica
- **F3 Viewer:** apre una scheda viewer nel gruppo corrente. Modalità testo (con codifica selezionabile), esadecimale, immagine, PDF, media. I file di più GB si leggono a blocchi senza caricarli in memoria.
- **F4 Editor:** apre una scheda Monaco interna. Un'impostazione permette di usare un editor esterno, per esempio VS Code. **[S]**
- **Confronto contenuti:** diff Monaco per i file di testo, confronto byte a byte con evidenziazione per i binari.

### 6.4 Strumenti
- **Multi-rename (Ctrl+M):**
  - maschere in stile TC (`[N]`, `[E]`, `[C]` contatore, `[YMD]`, `[N1-5]`…)
  - cerca e sostituisci con regex
  - maiuscole e minuscole
  - anteprima in tempo reale in tabella, con i conflitti evidenziati
  - undo dell'ultimo batch
- **Sincronizza cartelle:** confronto per data, dimensione e opzionalmente contenuto; filtri; elenco delle differenze con la direzione scelta per ogni riga; esecuzione tramite la coda.
- **Ricerca (Alt+F7 e vista Cerca):**
  - per nome (glob o fuzzy), per contenuto (testo o regex), per dimensione, data e attributi
  - anche dentro gli archivi (opzione)
  - i risultati si aprono come scheda "virtuale", su cui si possono fare operazioni come in TC ("Feed to listbox")

### 6.5 Funzioni moderne
- **Git (sola lettura):**
  - badge nei file e branch nella barra di stato
  - la vista Git della barra laterale elenca i file modificati del repo corrente
  - diff nell'ispettore
- **Tag e colori:** tag definiti dall'utente, ognuno con un colore. Si assegnano da un comando o dal menu, compaiono come pallino accanto al nome e si possono usare come filtro. Sono salvati in un database SQLite locale indicizzato per percorso. Se un file viene spostato fuori dall'app, il tag si perde: è un limite accettato in v1. **[S]**
- **Workspace:**
  - all'avvio si ripristina l'ultima sessione (gruppi, schede, percorsi, pannelli visibili)
  - si possono salvare workspace con un nome e passare dall'uno all'altro con la command palette

### 6.6 Archivi
- ZIP, 7z, TAR, TAR.GZ si aprono come cartelle (Invio o Ctrl+PgDn). RAR solo in lettura.
- Estrazione = copia fuori dall'archivio (anche F5 verso un altro gruppo). Alt+F9 estrae tutto.
- Creazione con Alt+F5 (ZIP, 7z, TAR.GZ).
- Aggiunta ed eliminazione dentro un archivio esistente: solo ZIP in v1 **[S]**.

### 6.7 Rete
- FTP, FTPS e SFTP, con un gestore delle connessioni salvate (Ctrl+F, come TC). Una connessione si apre come scheda; le operazioni passano per la coda.
- Le credenziali stanno nel **Windows Credential Manager**, mai in chiaro nei file di configurazione o nei log.
- Le condivisioni SMB passano dal file system normale tramite percorsi UNC.

### 6.8 Configurazione, import e lingue
- Impostazioni in JSON con una pagina UI in stile VS Code (ricerca, categorie).
- **Portable:** se accanto all'eseguibile esiste un file marcatore `portable`, la configurazione sta accanto all'exe; altrimenti sta in `%APPDATA%\ModernCommander`. **[S]**
- **Import da `wincmd.ini`** **[U]**, con un comando dedicato e un'anteprima di ciò che verrà importato:
  - hotlist (DirMenu)
  - schede salvate
  - scorciatoie personalizzate
  - connessioni FTP, senza password
  - *Ambito [S]:* colori e button bar non si importano.
- Lingue: inglese (default) e italiano. Stringhe in file JSON per lingua. Il backend restituisce codici di errore, non testo, che il frontend traduce.

## 7. Architettura **[S]**

### 7.1 Divisione delle responsabilità
- **Backend Rust**:
  - è l'unica fonte di verità sul file system
  - gestisce la coda operazioni, watcher, archivi, rete, git, PTY, tag (SQLite) e chiamate Win32/COM
  - espone comandi Tauri e invia eventi (avanzamento, cambi di cartella, output del PTY)
- **Frontend**:
  - stato della UI (layout, gruppi, schede, selezione), comandi e keybinding, rendering
  - non accede mai direttamente al disco

### 7.2 Astrazione delle sorgenti
Una sola interfaccia nel backend, "provider di cartella" (elenco, stat, lettura e scrittura a stream), con queste implementazioni: file system locale, archivio, FTP/SFTP, risultati di ricerca. La lista dei file, la coda e il viewer lavorano su questa interfaccia. È l'unica astrazione che si giustifica fin dalla v1, perché ha quattro implementazioni reali.

### 7.3 Comandi
Ogni azione dell'utente è un comando con id, titolo tradotto e handler. Tastiera, F-key, command palette e menu richiamano tutti lo stesso registro dei comandi. Il remapping lavora su questo registro.

### 7.4 Obiettivi di prestazioni **[S]**
- Avvio a finestra pronta: meno di 1 s su SSD.
- Cartella da 10.000 elementi: lista visibile in meno di 150 ms. Cartella da 100.000: utilizzabile, con scorrimento fluido e caricamento progressivo.
- Navigazione da tastiera senza ritardo percepibile: input → cursore in meno di 16 ms.
- Anteprima nell'ispettore: mai bloccante; si annulla al cambio di file.

## 8. Rischi noti
- **Menu contestuale nativo e drag & drop OLE dentro WebView2:** vanno implementati in Rust sull'HWND della finestra. Conviene fare uno spike all'inizio della fase 1.
- **RAR:** l'unica libreria di estrazione affidabile (UnRAR) ha una licenza non OSI. Va verificato che si possa distribuire insieme a un progetto MIT; se non si può, RAR si toglie dalla v1.
- **Helper elevato:** è una superficie di attacco locale, quindi le regole di §6.2 sono vincolanti. Il lancio con `runas`, la named pipe e l'annullamento tra processi vanno validati con uno spike in fase 1.
- **Ctrl+ò e altre scorciatoie con layout di tastiera diversi:** i binding si basano sui virtual key, non sui caratteri.

## 9. Fasi di consegna **[U]** (struttura), contenuto **[S]**

Ogni fase produce un'app usabile.

1. **Fondamenta**:
   - layout completo (barra delle attività, barra laterale, gruppi e schede, F-key, barra di stato)
   - navigazione, lista virtualizzata, marcatura, scorciatoie TC e remapping
   - copia, spostamento, eliminazione, rinomina e creazione cartella con coda e conflitti
   - helper elevato per "Riprova come amministratore"
   - menu contestuale nativo, drag & drop, watcher
   - hotlist, dimensione delle cartelle
   - temi A/C, i18n en/it, impostazioni, build portable + installer
2. **Vedere**:
   - viewer F3, editor F4, ispettore Ctrl+Q
   - command palette, Ctrl+P
   - terminale a schede
3. **Archivi e rete**:
   - archivi come cartelle, creazione ed estrazione
   - FTP/FTPS/SFTP con gestore delle connessioni
4. **Strumenti**:
   - multi-rename, sincronizzazione cartelle, confronto contenuti
   - ricerca avanzata con risultati come scheda
   - branch view
5. **Extra**:
   - git in sola lettura, tag e colori
   - workspace
   - import da `wincmd.ini`

## 10. Lasciato al builder
- Scelta e versioni delle librerie, verificate sulla documentazione delle versioni effettivamente installate.
- Struttura delle cartelle del codice, gestione dello stato nel frontend, formato esatto di settings e keybinding.
- Icone: set monocromatico per il tema A, colorato tenue per il C, purché con licenza open source compatibile.
- Strategia di test. Minimo richiesto: test automatici in Rust per la logica che può perdere dati o produrre risultati sbagliati (coda, conflitti, maschere di multi-rename, confronto di sincronizzazione, provider di archivio).
- Dettagli delle finestre di dialogo e micro-interazioni, coerenti con le bozze e con lo stile di VS Code.

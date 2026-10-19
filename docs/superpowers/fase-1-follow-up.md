# Fase 1: verifiche manuali e follow-up

Data: 2026-10-17 · Branch: `feat/fase-1-fondamenta`

## Verifiche manuali da fare

Non si possono automatizzare: richiedono UAC, hardware reale o l'osservazione dello schermo.

- **Installer NSIS** (`src-tauri\target\release\bundle\nsis\Modern Commander_0.1.0_x64-setup.exe`, per-machine, chiede UAC): installare, avviare, verificare che la configurazione finisca in `%APPDATA%\ModernCommander`, poi disinstallare.
- **Avvio**: tempo con cronometro, senza debugger, a caldo e a freddo. Il budget è sotto 1 s; misurato con CDP: 577–878 ms a caldo.
- **Elevazione**:
  - F7 in `C:\Program Files`, poi "Riprova come amministratore": deve comparire UAC e la cartella va creata.
  - Una seconda operazione entro 60 s non deve chiedere di nuovo UAC.
  - Se l'UAC viene rifiutato si torna al dialogo.
- **Shell**:
  - menu contestuale nativo: aspetto e sottomenu "Apri con";
  - drag reale verso Explorer e da Explorer;
  - drag tra due gruppi.
- **Cestino**: F8 reale; cartella più grande della quota del Cestino (deve comparire l'avviso della Shell).
- **Unità**:
  - chiavetta USB aperta in una scheda: rimozione sicura e poi estrazione fisica, la scheda deve passare alla lista delle unità;
  - cartella aperta eliminata da Explorer: la scheda deve risalire all'antenato.
- **Copia**: F5/F6 dai dialoghi reali; copia con disco pieno; copia tra due dischi fisici.
- **UI**: tema chiaro; DPI oltre il 100%; scorrimento su 100.000 file; AltGr su tastiera italiana.
- **Scorciatoie nuove**: Ctrl+Shift+E, Ctrl+ò, Ctrl+, e Ctrl+Shift+\\ sono libere in TC secondo fonti secondarie. Sulla documentazione ufficiale non è stato possibile verificarlo.

## Scelte da confermare

- Nel dialogo dei conflitti il focus di default è su **Salta**. TC usa Sovrascrivi; Salta è più sicuro, perché Invio non sovrascrive.
- Il drag tra gruppi apre sempre il dialogo di **copia**: l'evento di drop di Tauri non riporta i tasti modificatori, quindi Shift per spostare non è disponibile.
- In release l'app usa `panic = "abort"`: un errore interno chiude l'app. I dati restano integri, perché la copia passa per `.mcpart` e la sorgente si elimina solo a copia riuscita.

## Follow-up noti

**Prima del rilascio pubblico**
- CSP stretta in `tauri.conf.json` (oggi `csp: null`), da provare sulla build.
- Le unità di rete mappate non sono visibili all'helper elevato: convertire in UNC o documentarlo. In attesa della verifica manuale di `docs/superpowers/plans/2026-10-19-helper-uac-toctou.md` (Task 3, Step 2): la risoluzione lato app potrebbe averlo già risolto.

**Operazioni su file**
- `runner.rs` `dir()`: nei nuovi tentativi usare `self.items[i].unlisted`. Oggi il difetto è latente.
- Avanzamento per singolo file nel pannello Operazioni.
- Il dialogo della Shell per cartelle oltre la quota del Cestino parte senza finestra proprietaria.
- Mancano un'icona e un comportamento dedicati per F8 quando le righe marcate sono nascoste dal filtro rapido.

**Configurazione**
- Hotlist con una voce malformata: viene scartata intera e la prima modifica sovrascrive il file senza `.bak`.

**Scorciatoie**
- Reset di un comando: toglie in silenzio i tasti a un altro comando.
- La cattura sostituisce tutti i tasti del comando.

**Varie UI**
- Hover sui sottomenu della hotlist.
- Navigazione da tastiera di albero e preferiti.
- Il popup della ricerca rapida resta orfano al cambio di gruppo.
- La pressione prolungata di Backspace non avanza con IPC lento.

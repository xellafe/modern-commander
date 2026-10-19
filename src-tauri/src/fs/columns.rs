//! Conteggio delle colonne a blocchi, per il visualizzatore a righe lunghe intere: da un punto noto della riga (offset
//! e colonna) trova il carattere che occupa una colonna, senza caricare mai la riga in memoria. Servono solo i confini
//! dei caratteri, non il testo: il conteggio coincide con `encoding_rs` per la stessa codifica. Annullabile.

use super::content::open_shared;
use super::lines::read_block;
use crate::error::{AppError, ErrorCode};
use encoding_rs::Encoding;
use serde::{Deserialize, Serialize};
use std::ops::ControlFlow::{self, Break, Continue};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// Confine di carattere sulla riga, con la colonna dove inizia quel carattere.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mark {
    pub offset: u64,
    pub column: u64,
}

/// Carattere trovato (`eol` falso) o fine riga (`eol` vero: offset del CR, dell'LF o dimensione del file).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnHit {
    pub offset: u64,
    pub column: u64,
    pub eol: bool,
}

/// Punto di controllo del conteggio `index`-esimo di `column_offsets`.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnProgress {
    pub index: u32,
    pub mark: Mark,
}

pub const TAB_WIDTH: u64 = 8;

/// Byte letti per volta: annullamento e punti di controllo a ogni blocco.
const BLOCK: usize = 1 << 20;

/// Carattere decodificato, per quel che serve alle colonne. `Other(n)`: `n` caratteri che non si separano sui byte
/// (di solito 1; certe coppie Big5 ne danno 2).
#[derive(Clone, Copy, PartialEq)]
enum Ch {
    Tab,
    Lf,
    Cr,
    Other(u64),
}

fn ch_of(b: u8) -> Ch {
    match b {
        b'\t' => Ch::Tab,
        b'\n' => Ch::Lf,
        b'\r' => Ch::Cr,
        _ => Ch::Other(1),
    }
}

/// Colonne sui caratteri: `Break` appena un carattere occupa `target` o la riga finisce.
struct Counter {
    col: u64,
    target: u64,
    /// Offset di un CR in sospeso: prima di un LF non è una colonna.
    cr: Option<u64>,
}

impl Counter {
    fn advance(&mut self, offset: u64, width: u64) -> ControlFlow<ColumnHit> {
        if self.target < self.col + width {
            return Break(ColumnHit { offset, column: self.col, eol: false });
        }
        self.col += width;
        Continue(())
    }

    fn ch(&mut self, offset: u64, ch: Ch) -> ControlFlow<ColumnHit> {
        if let Some(cr) = self.cr.take() {
            if ch == Ch::Lf {
                return Break(ColumnHit { offset: cr, column: self.col, eol: true });
            }
            self.advance(cr, 1)?;
        }
        match ch {
            Ch::Lf => Break(ColumnHit { offset, column: self.col, eol: true }),
            Ch::Cr => {
                self.cr = Some(offset);
                Continue(())
            }
            Ch::Tab => self.advance(offset, TAB_WIDTH - self.col % TAB_WIDTH),
            Ch::Other(n) => self.advance(offset, n),
        }
    }

    fn end(&mut self, size: u64) -> ColumnHit {
        if let Some(cr) = self.cr.take()
            && let Break(h) = self.advance(cr, 1)
        {
            return h;
        }
        ColumnHit { offset: size, column: self.col, eol: true }
    }
}

/// Confini dei caratteri byte per byte, con lo stato a cavallo dei blocchi.
enum Seg {
    /// Decodifica WHATWG: byte di continuazione ancora attesi e limiti per il prossimo; `start` è il primo byte.
    Utf8 { needed: u8, lower: u8, upper: u8, start: u64 },
    /// `half`: primo byte di un'unità; `lead`: offset di un surrogato alto in attesa del basso.
    Utf16 { be: bool, half: Option<u8>, lead: Option<u64> },
    Single,
    /// Code page multibyte: i byte da `start` non ancora attribuiti a un carattere.
    Multi { enc: &'static Encoding, pend: Vec<u8>, start: u64 },
}

/// Decodifica di `bytes` con un decoder nuovo, in `out` (UTF-16); restituisce le unità scritte.
fn decode(enc: &'static Encoding, bytes: &[u8], last: bool, out: &mut [u16; 16]) -> usize {
    enc.new_decoder_without_bom_handling().decode_to_utf16(bytes, out, last).2
}

fn chars(units: &[u16]) -> u64 {
    units.iter().filter(|u| !(0xDC00..=0xDFFF).contains(*u)).count() as u64
}

/// Primo carattere (o gruppo inseparabile) di `pend`, come `(byte, caratteri)`; `None` se il decoder aspetta altri byte.
/// Il gruppo è il prefisso più corto la cui decodifica, seguita da quella del resto con un decoder nuovo, dà la stessa
/// uscita di tutto `pend`, sia con `last` sia con il flush: dove finisce si può ripartire da zero. Così U+FFFD seguito
/// da un byte ASCII riletto (un iniziale Shift_JIS davanti a uno spazio) dà due caratteri su due byte. Il confronto con
/// il flush scarta il resto che il decoder tiene in sospeso: in una coppia non mappata il secondo byte, anche se è un
/// iniziale valido, appartiene al U+FFFD.
fn split(enc: &'static Encoding, pend: &[u8], last: bool) -> Option<(usize, u64)> {
    let (mut all, mut flushed, mut head, mut rest) = ([0; 16], [0; 16], [0; 16], [0; 16]);
    let n = decode(enc, pend, last, &mut all);
    if n == 0 {
        return None;
    }
    let nf = decode(enc, pend, true, &mut flushed);
    let same_rest = |out: &[u16], rest_bytes: &[u8], last: bool, buf: &mut [u16; 16]| {
        let r = decode(enc, rest_bytes, last, buf);
        out == &buf[..r]
    };
    for j in 1..pend.len() {
        let h = decode(enc, &pend[..j], true, &mut head);
        if h > 0
            && all[..n].starts_with(&head[..h])
            && same_rest(&all[h..n], &pend[j..], last, &mut rest)
            && flushed[..nf].starts_with(&head[..h])
            && same_rest(&flushed[h..nf], &pend[j..], true, &mut rest)
        {
            return Some((j, chars(&head[..h])));
        }
    }
    // Tutto `pend` è un gruppo: con le code page ammesse il decoder qui non ha byte in sospeso, che andrebbero persi.
    debug_assert!(flushed[..nf] == all[..n], "byte in sospeso nel ripiego di `split`: {pend:02X?}");
    Some((pend.len(), chars(&all[..n])))
}

/// Attribuisce i caratteri completi di `pend`. Da stato iniziale un byte ASCII è sempre un carattere a sé, in tutte le
/// code page multibyte ammesse: tab, CR e LF arrivano solo da lì.
fn settle(
    enc: &'static Encoding,
    pend: &mut Vec<u8>,
    start: &mut u64,
    last: bool,
    c: &mut Counter,
) -> ControlFlow<ColumnHit> {
    while let Some(&b) = pend.first() {
        let (len, ch) = if b < 0x80 {
            (1, ch_of(b))
        } else {
            match split(enc, pend, last) {
                Some((j, n)) => (j, Ch::Other(n)),
                None => break,
            }
        };
        c.ch(*start, ch)?;
        pend.drain(..len);
        *start += len as u64;
    }
    Continue(())
}

impl Seg {
    fn new(enc: &'static Encoding) -> Self {
        if enc == encoding_rs::UTF_8 {
            Seg::Utf8 { needed: 0, lower: 0x80, upper: 0xBF, start: 0 }
        } else if enc == encoding_rs::UTF_16LE || enc == encoding_rs::UTF_16BE {
            Seg::Utf16 { be: enc == encoding_rs::UTF_16BE, half: None, lead: None }
        } else if enc.is_single_byte() {
            Seg::Single
        } else {
            Seg::Multi { enc, pend: Vec::new(), start: 0 }
        }
    }

    /// Il byte `b` all'offset `k`.
    fn push(&mut self, b: u8, k: u64, c: &mut Counter) -> ControlFlow<ColumnHit> {
        match self {
            Seg::Utf8 { needed, lower, upper, start } => {
                if *needed > 0 {
                    let ok = (*lower..=*upper).contains(&b);
                    (*lower, *upper) = (0x80, 0xBF);
                    if ok {
                        *needed -= 1;
                        return if *needed == 0 { c.ch(*start, Ch::Other(1)) } else { Continue(()) };
                    }
                    // Sottoparte massimale: un U+FFFD, poi il byte si rilegge da capo.
                    *needed = 0;
                    c.ch(*start, Ch::Other(1))?;
                }
                *start = k;
                match b {
                    0..=0x7F => return c.ch(k, ch_of(b)),
                    0xC2..=0xDF => *needed = 1,
                    0xE0..=0xEF => *needed = 2,
                    0xF0..=0xF4 => *needed = 3,
                    _ => return c.ch(k, Ch::Other(1)),
                }
                match b {
                    0xE0 => *lower = 0xA0,
                    0xED => *upper = 0x9F,
                    0xF0 => *lower = 0x90,
                    0xF4 => *upper = 0x8F,
                    _ => {}
                }
                Continue(())
            }
            Seg::Utf16 { be, half, lead } => {
                let Some(h) = half.take() else {
                    *half = Some(b);
                    return Continue(());
                };
                let u = if *be { u16::from_be_bytes([h, b]) } else { u16::from_le_bytes([h, b]) };
                if let Some(s) = lead.take() {
                    if (0xDC00..=0xDFFF).contains(&u) {
                        return c.ch(s, Ch::Other(1)); // coppia surrogata
                    }
                    c.ch(s, Ch::Other(1))?; // surrogato alto spaiato: U+FFFD, l'unità si rilegge
                }
                if (0xD800..=0xDBFF).contains(&u) {
                    *lead = Some(k - 1);
                    return Continue(());
                }
                c.ch(k - 1, if u < 0x80 { ch_of(u as u8) } else { Ch::Other(1) })
            }
            Seg::Single => c.ch(k, ch_of(b)),
            Seg::Multi { enc, pend, start } => {
                if pend.is_empty() {
                    if b < 0x80 {
                        return c.ch(k, ch_of(b));
                    }
                    *start = k;
                }
                pend.push(b);
                settle(enc, pend, start, false, c)
            }
        }
    }

    /// Fine del file a `size`: i byte in sospeso diventano U+FFFD come nel flush del decoder.
    fn finish(&mut self, size: u64, c: &mut Counter) -> ControlFlow<ColumnHit> {
        match self {
            Seg::Utf8 { needed, start, .. } if *needed > 0 => c.ch(*start, Ch::Other(1)),
            Seg::Utf16 { half, lead, .. } if half.is_some() || lead.is_some() => {
                c.ch(lead.unwrap_or(size - 1), Ch::Other(1))
            }
            Seg::Multi { enc, pend, start } => settle(enc, pend, start, true, c),
            _ => Continue(()),
        }
    }

    /// Primo byte non ancora attribuito a un carattere, con `pos` il prossimo da leggere.
    fn boundary(&self, pos: u64) -> u64 {
        match self {
            Seg::Utf8 { needed, start, .. } if *needed > 0 => *start,
            Seg::Utf16 { half, lead, .. } => lead.unwrap_or(pos - half.is_some() as u64),
            Seg::Multi { pend, start, .. } if !pend.is_empty() => *start,
            _ => pos,
        }
    }
}

/// Conta le colonne da `from` (offset su un confine di carattere, con la sua colonna vera dall'inizio della riga). Il
/// primo carattere che occupa `target` dà il suo inizio e la sua colonna iniziale, minore di `target` solo dentro un
/// tab (o un gruppo inseparabile); un LF, un CR+LF o la fine del file prima danno la fine riga con `eol`. Con
/// `u64::MAX` si chiede la fine riga. Ogni blocco letto chiama `progress` con il confine di carattere corrente.
/// `INVALID_REQUEST` per un'etichetta sconosciuta (o `replacement` e `iso-2022-jp`, che hanno stato fra i caratteri).
pub fn column_offset(
    path: &Path,
    encoding: &str,
    from: Mark,
    target: u64,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Mark),
) -> Result<ColumnHit, AppError> {
    column_offset_in(path, encoding, from, target, cancel, progress, BLOCK)
}

fn column_offset_in(
    path: &Path,
    encoding: &str,
    from: Mark,
    target: u64,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Mark),
    block: usize,
) -> Result<ColumnHit, AppError> {
    let enc = Encoding::for_label(encoding.as_bytes())
        .filter(|e| *e != encoding_rs::REPLACEMENT && *e != encoding_rs::ISO_2022_JP)
        .ok_or_else(|| AppError::new(ErrorCode::InvalidRequest, Some(path)))?;
    let mut seg = Seg::new(enc);
    let mut c = Counter { col: from.column, target, cr: None };
    let mut f = open_shared(path).map_err(|e| AppError::from_io(&e, path))?;
    let mut buf = Vec::with_capacity(block);
    let mut pos = from.offset;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(AppError::new(ErrorCode::Cancelled, Some(path)));
        }
        read_block(&mut f, pos, block, &mut buf, path)?;
        for &b in &buf {
            if let Break(h) = seg.push(b, pos, &mut c) {
                return Ok(h);
            }
            pos += 1;
        }
        if buf.len() < block {
            break;
        }
        progress(Mark { offset: c.cr.unwrap_or(seg.boundary(pos)), column: c.col });
    }
    Ok(match seg.finish(pos, &mut c) {
        Break(h) => h,
        Continue(()) => c.end(pos),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const ZERO: Mark = Mark { offset: 0, column: 0 };

    fn file(bytes: &[u8]) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(bytes).unwrap();
        f
    }

    // Blocchi da 3 byte: quasi ogni carattere multibyte finisce a cavallo di due blocchi.
    fn col(bytes: &[u8], enc: &str, target: u64) -> ColumnHit {
        let f = file(bytes);
        column_offset_in(f.path(), enc, ZERO, target, &AtomicBool::new(false), &mut |_| {}, 3).unwrap()
    }

    fn hit(offset: u64, column: u64, eol: bool) -> ColumnHit {
        ColumnHit { offset, column, eol }
    }

    fn utf16(s: &str, be: bool) -> Vec<u8> {
        s.encode_utf16().flat_map(|u| if be { u.to_be_bytes() } else { u.to_le_bytes() }).collect()
    }

    #[test]
    fn ascii() {
        assert_eq!(col(b"abcdef", "utf-8", 3), hit(3, 3, false));
    }

    #[test]
    fn utf8_accents() {
        assert_eq!(col("Perché è già".as_bytes(), "utf-8", 8), hit(10, 8, false));
    }

    #[test]
    fn tab_stops() {
        assert_eq!(col(b"a\tb", "utf-8", 3), hit(1, 1, false)); // dentro il tab
        assert_eq!(col(b"a\tb", "utf-8", 8), hit(2, 8, false));
        assert_eq!(col(b"a\tb", "utf-8", u64::MAX), hit(3, 9, true));
    }

    #[test]
    fn crlf_is_not_a_column() {
        assert_eq!(col(b"ab\r\ncd", "utf-8", u64::MAX), hit(2, 2, true));
        // Un CR da solo vale 1.
        assert_eq!(col(b"a\rb", "utf-8", 1), hit(1, 1, false));
        assert_eq!(col(b"a\rb", "utf-8", u64::MAX), hit(3, 3, true));
        assert_eq!(col(b"a\r", "utf-8", u64::MAX), hit(2, 2, true));
    }

    #[test]
    fn shorter_line_is_eol() {
        assert_eq!(col(b"abc\ndef", "utf-8", 10), hit(3, 3, true));
    }

    #[test]
    fn utf16_surrogate_pair_is_one() {
        assert_eq!(col(&utf16("a😀b", false), "utf-16le", 2), hit(6, 2, false));
        assert_eq!(col(&utf16("a😀b", true), "utf-16be", 2), hit(6, 2, false));
        assert_eq!(col(&utf16("ab\r\ncd", true), "utf-16be", u64::MAX), hit(4, 2, true));
    }

    #[test]
    fn shift_jis_double_byte() {
        let (bytes, _, _) = encoding_rs::SHIFT_JIS.encode("日本a");
        assert_eq!(bytes.len(), 5);
        assert_eq!(col(&bytes, "shift_jis", 1), hit(2, 1, false));
        assert_eq!(col(&bytes, "shift_jis", 2), hit(4, 2, false));
    }

    #[test]
    fn matches_encoding_rs_decode() {
        let cases: &[(&str, &[u8])] = &[
            ("utf-8", &[0x61, 0xF0, 0x9F, 0x61, 0x62]),
            ("utf-8", &[0x61, 0xC3]),
            ("utf-8", &[0xED, 0xA0, 0x80]),
            ("utf-8", &[0xE0, 0x80, 0xF4, 0x90, 0xC3, 0xA0]),
            ("utf-16le", &[0x3D, 0xD8, 0x61, 0x00]), // surrogato alto spaiato
            ("utf-16le", &[0x00, 0xDE, 0x61, 0x00]), // surrogato basso spaiato
            ("utf-16le", &[0x61, 0x00, 0x3D, 0xD8]), // surrogato alto a fine file
            ("utf-16le", &[0x3D, 0xD8, 0x61]),       // surrogato alto e mezza unità
            ("utf-16be", &[0x00, 0x61, 0x00]),
            ("windows-1252", &[0x61, 0x80, 0x81, 0xFF]),
            ("shift_jis", &[0x61, 0x82]),       // iniziale senza seguito
            ("shift_jis", &[0x82, 0x61, 0x62]), // 0x82 0x61 è una coppia valida (Ｂ)
            ("shift_jis", &[0x82, 0x20, 0x62]), // iniziale seguito da ASCII: U+FFFD e lo spazio
            ("shift_jis", &[0x82, 0xFF, 0x62]), // secondo byte non ASCII non valido: un solo U+FFFD
            ("gbk", &[0x81, 0x30, 0x81, 0x20]), // quattro byte interrotti: tre byte si rileggono
            ("big5", &[0x88, 0x62, 0x61]),      // una coppia che dà due caratteri
            // Coppie non mappate con secondo byte non ASCII, che è anche un iniziale: un solo U+FFFD.
            ("shift_jis", &[0x81, 0xE9, 0x40]),
            ("shift_jis", &[0x81, 0xE9, 0xE9, 0x40]),
            ("big5", &[0x81, 0xA1, 0xA4, 0x40]),
            ("euc-kr", &[0xC9, 0xA1, 0xB0, 0xA1]),
        ];
        for (label, bytes) in cases {
            let enc = encoding_rs::Encoding::for_label(label.as_bytes()).unwrap();
            let text = |b: &[u8]| enc.decode_without_bom_handling(b).0.into_owned();
            let all = text(bytes);
            let want = all.chars().count() as u64;
            let ctx = format!("{label} {bytes:02X?}");
            assert_eq!(col(bytes, label, u64::MAX), hit(bytes.len() as u64, want, true), "{ctx}");
            // Per ogni colonna: ripartendo da zero dall'offset trovato si decodificano proprio i caratteri da
            // `column` in poi (la colonna è uguale al target, tranne dentro un gruppo inseparabile di Big5). La testa
            // non si decodifica a parte: a fine file gb18030 non rilegge i byte di una sequenza interrotta.
            for t in 0..want {
                let h = col(bytes, label, t);
                let rest: String = all.chars().skip(h.column as usize).collect();
                assert_eq!(text(&bytes[h.offset as usize..]), rest, "{ctx} target {t}");
                assert!(h.column == t || (*label == "big5" && h.column < t), "{ctx} target {t}: {h:?}");
            }
        }
        assert_eq!(col(&[0x81, 0xE9, 0x40], "shift_jis", 1), hit(2, 1, false));
        // U+FFFD sul byte iniziale, lo spazio e la `b` ognuno sul proprio byte.
        assert_eq!(col(&[0x82, 0x20, 0x62], "shift_jis", 1), hit(1, 1, false));
        assert_eq!(col(&[0x82, 0x20, 0x62], "shift_jis", 2), hit(2, 2, false));
        // `Ê̄` di Big5 sono due caratteri da una coppia inseparabile: la colonna 1 è dentro, come in un tab.
        assert_eq!(col(&[0x88, 0x62, 0x61], "big5", 1), hit(0, 0, false));
        assert_eq!(col(&[0x88, 0x62, 0x61], "big5", 2), hit(2, 2, false));
    }

    #[test]
    fn resume_from_mark_equals_full_count() {
        let line = "\tèé".repeat(3 * BLOCK / 5 + 1).into_bytes();
        let f = file(&line);
        let no = AtomicBool::new(false);
        let mut marks = vec![];
        let full = column_offset(f.path(), "utf-8", ZERO, u64::MAX, &no, &mut |m| marks.push(m)).unwrap();
        assert_eq!((full.offset, full.eol), (line.len() as u64, true));
        assert_eq!(marks.len(), 3);
        // Il secondo confine di blocco cade dentro una `è`: il punto di controllo è il suo inizio.
        assert_eq!(marks[1].offset, 2 * BLOCK as u64 - 1);
        for m in &marks {
            let target = m.column + 1000;
            let resumed = column_offset(f.path(), "utf-8", *m, target, &no, &mut |_| {}).unwrap();
            let fresh = column_offset(f.path(), "utf-8", ZERO, target, &no, &mut |_| {}).unwrap();
            assert_eq!(resumed, fresh);
            assert_eq!(column_offset(f.path(), "utf-8", *m, u64::MAX, &no, &mut |_| {}).unwrap(), full);
        }
    }

    #[test]
    fn unknown_label_is_invalid_request() {
        let f = file(b"abc");
        let e = column_offset(f.path(), "klingon", ZERO, 1, &AtomicBool::new(false), &mut |_| {}).unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidRequest);
    }

    #[test]
    fn cancel_returns_cancelled() {
        let f = file(b"abc");
        let e = column_offset(f.path(), "utf-8", ZERO, 1, &AtomicBool::new(true), &mut |_| {}).unwrap_err();
        assert_eq!(e.code, ErrorCode::Cancelled);
    }
}

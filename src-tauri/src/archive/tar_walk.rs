//! Lettura a mano delle intestazioni TAR (ustar, GNU, PAX). Il crate `tar` carica per intero in memoria le voci
//! GNU `L` e PAX `x`: con un TAR.GZ ostile bastano pochi KB compressi per esaurire la memoria. Qui le estensioni
//! si leggono solo sotto un tetto piccolo e i contenuti si scartano a blocchi, sempre da un lettore a flusso.

use super::ArchiveListing;
use super::list::{corrupt, entry, io_err};
use crate::error::AppError;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const BLOCK: usize = 512;

/// Tetto in byte per il contenuto di una voce GNU `L` o PAX `x`: oltre, si scarta e vale il nome dell'intestazione.
const MAX_EXT: u64 = 64 * 1024;

/// Numero di un campo dell'intestazione: ottale ASCII o base 256 (GNU, bit alto del primo byte). `None` se non valido.
fn parse_num(field: &[u8]) -> Option<u64> {
    let first = *field.first()?;
    if first & 0x80 != 0 {
        if first & 0x40 != 0 {
            return None; // negativo
        }
        return field[1..].iter().try_fold(u64::from(first & 0x3F), |v, &b| v.checked_mul(256)?.checked_add(u64::from(b)));
    }
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    let digits = field[..end].trim_ascii();
    digits.iter().try_fold(0u64, |v, &b| match b {
        b'0'..=b'7' => v.checked_mul(8)?.checked_add(u64::from(b - b'0')),
        _ => None,
    })
}

/// Legge un blocco intero; `Ok(false)` se il flusso finisce esattamente prima del blocco.
fn read_block<R: Read>(r: &mut R, b: &mut [u8; BLOCK]) -> io::Result<bool> {
    let mut n = 0;
    while n < BLOCK {
        match r.read(&mut b[n..])? {
            0 => break,
            k => n += k,
        }
    }
    match n {
        0 => Ok(false),
        BLOCK => Ok(true),
        _ => Err(io::ErrorKind::UnexpectedEof.into()),
    }
}

/// Somma dei byte con il campo della somma letto come spazi; alcuni TAR vecchi la calcolano con i byte con segno.
fn checksum_ok(b: &[u8; BLOCK]) -> bool {
    let Some(stored) = parse_num(&b[148..156]) else { return false };
    let (mut unsigned, mut signed) = (0u64, 0i64);
    for (i, &x) in b.iter().enumerate() {
        let x = if (148..156).contains(&i) { b' ' } else { x };
        unsigned += u64::from(x);
        signed += i64::from(x as i8);
    }
    stored == unsigned || i64::try_from(stored) == Ok(signed)
}

/// Scarta `n` byte leggendoli a blocchi piccoli: il lettore decide se annullare (vedi `Cancellable`).
pub(super) fn skip_read<R: Read>(r: &mut R, n: u64) -> io::Result<()> {
    if io::copy(&mut r.by_ref().take(n), &mut io::sink())? == n { Ok(()) } else { Err(io::ErrorKind::UnexpectedEof.into()) }
}

/// Scarta `n` byte con un salto: per il TAR su file, senza leggere i contenuti. Saltare oltre la fine riesce, quindi
/// la lunghezza si controlla qui: un TAR troncato dentro i dati di una voce è danneggiato.
pub(super) fn skip_seek<R: Read + Seek>(r: &mut R, n: u64) -> io::Result<()> {
    let n = i64::try_from(n).map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
    let to = r.seek(SeekFrom::Current(n))?;
    if to > r.seek(SeekFrom::End(0))? {
        return Err(io::ErrorKind::UnexpectedEof.into());
    }
    r.seek(SeekFrom::Start(to)).map(drop)
}

/// Dimensione di `size` byte di dati più il riempimento fino a 512.
fn padded(size: u64, path: &Path) -> Result<u64, AppError> {
    size.checked_add(BLOCK as u64 - 1).map(|s| s / BLOCK as u64 * BLOCK as u64).ok_or_else(|| corrupt(path, "dimensione assurda"))
}

fn c_str(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// `path` e `size` dei record PAX (`<lunghezza> chiave=valore\n`); un record malformato interrompe la lettura.
fn parse_pax(mut d: &[u8]) -> (Option<String>, Option<u64>) {
    let (mut name, mut size) = (None, None);
    while let Some(sp) = d.iter().position(|&b| b == b' ') {
        let Some(len) = std::str::from_utf8(&d[..sp]).ok().and_then(|n| n.parse::<usize>().ok()) else { break };
        if len <= sp + 1 || len > d.len() {
            break;
        }
        let rec = d[sp + 1..len].strip_suffix(b"\n").unwrap_or(&d[sp + 1..len]);
        if let Some(eq) = rec.iter().position(|&b| b == b'=') {
            match &rec[..eq] {
                b"path" => name = Some(String::from_utf8_lossy(&rec[eq + 1..]).into_owned()),
                b"size" => size = std::str::from_utf8(&rec[eq + 1..]).ok().and_then(|s| s.parse().ok()),
                _ => {}
            }
        }
        d = &d[len..];
    }
    (name, size)
}

/// Intestazione di una voce TAR, con le estensioni GNU `L` e PAX `x` già applicate.
pub(super) struct TarHeader {
    pub name: String,
    pub flag: u8,
    /// Byte di dati nell'archivio: zero per collegamenti, cartelle e dispositivi, qualunque cosa dica il campo.
    #[allow(dead_code)] // per chi legge i dati della voce dal flusso (cursore del TAR.GZ)
    pub data_len: u64,
    /// Dimensione da mostrare: per i file GNU sparse quella vera, non quella memorizzata.
    pub shown_size: u64,
    pub modified_ms: Option<i64>,
    /// Inizio dei dati, dall'inizio del TAR.
    pub data_offset: u64,
}

impl TarHeader {
    /// Cartella anche col solo nome a `/` finale (TAR vecchi).
    pub fn is_dir(&self) -> bool {
        self.flag == b'5' || self.name.ends_with('/')
    }
}

/// Voci di un TAR da flusso, una alla volta. `skip` scarta i contenuti (lettura o salto). Non si fida di nulla:
/// le estensioni oltre `MAX_EXT` si scartano e la somma di controllo di ogni intestazione deve tornare. Le voci `L`,
/// `K`, `x`, `g` non si restituiscono. I dati della voce corrente si scartano alla chiamata successiva.
pub(super) struct TarWalker<R> {
    r: R,
    skip: fn(&mut R, u64) -> io::Result<()>,
    path: PathBuf,
    /// Byte letti o saltati dall'inizio.
    pos: u64,
    /// Inizio dell'intestazione successiva.
    next: u64,
}

impl<R: Read> TarWalker<R> {
    pub(super) fn new(r: R, skip: fn(&mut R, u64) -> io::Result<()>, path: &Path) -> Self {
        Self { r, skip, path: path.to_path_buf(), pos: 0, next: 0 }
    }

    fn io(&self, e: io::Error) -> AppError {
        io_err(&e, &self.path)
    }

    fn bad(&self) -> AppError {
        corrupt(&self.path, "intestazione TAR non valida")
    }

    fn skip(&mut self, n: u64) -> Result<(), AppError> {
        (self.skip)(&mut self.r, n).map_err(|e| self.io(e))?;
        self.pos += n;
        Ok(())
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), AppError> {
        self.r.read_exact(buf).map_err(|e| self.io(e))?;
        self.pos += buf.len() as u64;
        Ok(())
    }

    /// Un blocco intero; `false` a fine flusso esatta.
    fn block(&mut self, b: &mut [u8; BLOCK]) -> Result<bool, AppError> {
        let ok = read_block(&mut self.r, b).map_err(|e| self.io(e))?;
        self.pos += if ok { BLOCK as u64 } else { 0 };
        Ok(ok)
    }

    /// La voce successiva; `None` al primo blocco di zeri (si tollera anche il secondo mancante) o a fine flusso.
    pub(super) fn next_header(&mut self) -> Result<Option<TarHeader>, AppError> {
        if self.next > self.pos {
            self.skip(self.next - self.pos)?;
        }
        // Nome (GNU `L` o PAX `path`) e dimensione (PAX `size`) per la sola voce seguente.
        let (mut long_name, mut pax_size): (Option<String>, Option<u64>) = (None, None);
        let mut b = [0u8; BLOCK];
        loop {
            if !self.block(&mut b)? || b.iter().all(|&x| x == 0) {
                self.next = self.pos;
                return Ok(None);
            }
            if !checksum_ok(&b) {
                return Err(self.bad());
            }
            let flag = b[156];
            let size = parse_num(&b[124..136]).ok_or_else(|| self.bad())?;
            match flag {
                b'L' | b'x' | b'X' if size <= MAX_EXT => {
                    let mut payload = vec![0u8; size as usize];
                    self.read_exact(&mut payload)?;
                    self.skip(padded(size, &self.path)? - size)?;
                    // Un nome vuoto non indica nessuna voce: estrarla vorrebbe dire scrivere sulla cartella.
                    if flag == b'L' {
                        let name = c_str(&payload);
                        if name.is_empty() {
                            return Err(corrupt(&self.path, "nome lungo GNU vuoto"));
                        }
                        long_name = Some(name);
                    } else {
                        let (name, sz) = parse_pax(&payload);
                        if name.as_deref() == Some("") {
                            return Err(corrupt(&self.path, "path PAX vuoto"));
                        }
                        (long_name, pax_size) = (name.or(long_name), sz.or(pax_size));
                    }
                    continue;
                }
                // Troppo grandi o da ignorare: si scartano (con le grandi vale il nome dell'intestazione).
                b'L' | b'x' | b'X' | b'K' | b'g' => {
                    self.skip(padded(size, &self.path)?)?;
                    continue;
                }
                _ => {}
            }
            let data_len = if matches!(flag, b'1'..=b'6') { 0 } else { pax_size.unwrap_or(size) };
            let name = long_name.unwrap_or_else(|| {
                let (name, prefix) = (c_str(&b[..100]), c_str(&b[345..500]));
                // Il prefisso esiste solo in ustar (`ustar\0`); in GNU quei byte sono altro.
                if &b[257..263] == b"ustar\0" && !prefix.is_empty() { format!("{prefix}/{name}") } else { name }
            });
            let modified_ms = parse_num(&b[136..148]).filter(|&t| t > 0).and_then(|t| i64::try_from(t).ok()?.checked_mul(1000));
            // GNU sparse: si salta quanto memorizzato (più i blocchi di mappa), si mostra la dimensione vera.
            let sparse = flag == b'S';
            let shown_size = if sparse { parse_num(&b[483..495]).unwrap_or(data_len) } else { data_len };
            let mut more_map = sparse && b[482] == 1;
            while more_map {
                if !self.block(&mut b)? {
                    return Err(self.bad());
                }
                more_map = b[504] == 1;
            }
            let data_offset = self.pos;
            self.next = data_offset.checked_add(padded(data_len, &self.path)?).ok_or_else(|| self.bad())?;
            return Ok(Some(TarHeader { name, flag, data_len, shown_size, modified_ms, data_offset }));
        }
    }
}

/// Elenca un TAR da flusso, fino a `limit` voci: la memoria resta limitata da `limit` voci più il tetto sui nomi.
/// Il totale è noto solo se l'archivio è stato letto tutto entro il limite.
pub(super) fn walk_tar<R: Read>(r: R, skip: fn(&mut R, u64) -> io::Result<()>, path: &Path, limit: usize) -> Result<ArchiveListing, AppError> {
    let mut walker = TarWalker::new(r, skip, path);
    let mut entries = Vec::new();
    while let Some(h) = walker.next_header()? {
        if let Some(x) = entry(&h.name, h.is_dir(), h.shown_size, h.modified_ms, path)? {
            if entries.len() == limit {
                return Ok(ArchiveListing { entries, total: None, truncated: true });
            }
            entries.push(x);
        }
    }
    let total = Some(entries.len() as u64);
    Ok(ArchiveListing { entries, total, truncated: false })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use std::io::Cursor;

    fn octal(buf: &mut [u8], v: u64) {
        let s = format!("{:0w$o}", v, w = buf.len() - 1);
        buf[..s.len()].copy_from_slice(s.as_bytes());
    }

    /// Intestazione ustar minima con somma di controllo valida.
    fn block(name: &str, flag: u8, size: u64) -> [u8; 512] {
        let mut b = [0u8; 512];
        b[..name.len()].copy_from_slice(name.as_bytes());
        octal(&mut b[100..108], 0o644);
        octal(&mut b[124..136], size);
        octal(&mut b[136..148], 1_700_000_000);
        b[156] = flag;
        b[148..156].fill(b' ');
        let sum: u32 = b.iter().map(|&x| u32::from(x)).sum();
        octal(&mut b[148..155], u64::from(sum));
        b[155] = b' ';
        b
    }

    fn reseal(b: &mut [u8; 512]) {
        b[148..156].fill(b' ');
        let sum: u32 = b.iter().map(|&x| u32::from(x)).sum();
        octal(&mut b[148..155], u64::from(sum));
        b[155] = b' ';
    }

    /// Dati riempiti a multiplo di 512.
    fn data(d: &[u8]) -> Vec<u8> {
        let mut v = d.to_vec();
        v.resize(d.len().div_ceil(512) * 512, 0);
        v
    }

    fn file(name: &str, content: &[u8]) -> Vec<u8> {
        [block(name, b'0', content.len() as u64).to_vec(), data(content)].concat()
    }

    fn pax(pairs: &[(&str, &str)]) -> Vec<u8> {
        let mut out = Vec::new();
        for (k, v) in pairs {
            let body = format!(" {k}={v}\n");
            let mut len = body.len() + 1;
            while format!("{len}{body}").len() != len {
                len += 1;
            }
            out.extend(format!("{len}{body}").bytes());
        }
        out
    }

    fn walk(parts: &[Vec<u8>], limit: usize) -> Result<ArchiveListing, AppError> {
        let mut r = Cursor::new([parts.concat(), vec![0; 1024]].concat());
        walk_tar(&mut r, skip_read, Path::new("t.tar"), limit)
    }

    fn names(l: &ArchiveListing) -> Vec<&str> {
        l.entries.iter().map(|e| e.path.as_str()).collect()
    }

    #[test]
    fn global_pax_header_is_never_listed() {
        let g = pax(&[("comment", "0123456789abcdef0123456789abcdef01234567")]);
        let l = walk(&[block("pax_global_header", b'g', g.len() as u64).to_vec(), data(&g), file("a.txt", b"hi")], 10).unwrap();
        assert_eq!(names(&l), ["a.txt"]);
        assert_eq!(l.total, Some(1));
    }

    #[test]
    fn pax_path_renames_the_next_entry_only() {
        let x = pax(&[("path", "dir/nome lungo.txt"), ("mtime", "1.5")]);
        let l = walk(&[block("PaxHeader/x", b'x', x.len() as u64).to_vec(), data(&x), file("short", b"abc"), file("b", b"")], 10).unwrap();
        assert_eq!(names(&l), ["dir\\nome lungo.txt", "b"]);
        assert_eq!(l.entries[0].size, 3);
    }

    #[test]
    fn pax_size_overrides_the_header_size() {
        let x = pax(&[("size", "600")]);
        let l = walk(&[block("x", b'x', x.len() as u64).to_vec(), data(&x), block("big", b'0', 0).to_vec(), data(&[7; 600]), file("after", b"")], 10).unwrap();
        assert_eq!(names(&l), ["big", "after"]);
        assert_eq!(l.entries[0].size, 600);
    }

    #[test]
    fn gnu_long_name_is_used_and_long_link_is_not_listed() {
        let long = format!("{}/f.txt", "d".repeat(300));
        let mut payload = long.clone().into_bytes();
        payload.push(0);
        let link = b"target\0".to_vec();
        let l = walk(
            &[
                block("././@LongLink", b'L', payload.len() as u64).to_vec(),
                data(&payload),
                block("././@LongLink", b'K', link.len() as u64).to_vec(),
                data(&link),
                file("truncated-name", b"x"),
            ],
            10,
        )
        .unwrap();
        assert_eq!(names(&l), [long.replace('/', "\\").as_str()]);
    }

    #[test]
    fn empty_long_name_or_pax_path_is_corrupt() {
        let x = pax(&[("path", "")]);
        let e = walk(&[block("x", b'x', x.len() as u64).to_vec(), data(&x), file("a", b"")], 10).unwrap_err();
        assert_eq!(e.code, ErrorCode::CorruptArchive);
        let e = walk(&[block("././@LongLink", b'L', 0).to_vec(), file("a", b"")], 10).unwrap_err();
        assert_eq!(e.code, ErrorCode::CorruptArchive);
    }

    #[test]
    fn extension_over_the_cap_is_skipped_and_header_name_kept() {
        let big = vec![b'a'; (MAX_EXT + 1) as usize];
        let l = walk(&[block("././@LongLink", b'L', big.len() as u64).to_vec(), data(&big), file("kept.txt", b"x")], 10).unwrap();
        assert_eq!(names(&l), ["kept.txt"]);
    }

    /// Flusso di zeri che non accetta richieste di lettura grandi: chi lo legge "per intero" in un `Vec` fallisce.
    struct Zeros {
        left: u64,
        read: u64,
        on_read: Option<(u64, std::sync::Arc<std::sync::atomic::AtomicBool>)>,
    }

    impl Read for Zeros {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            assert!(buf.len() <= 64 * 1024, "lettura da {} byte: contenuto non scartato a blocchi", buf.len());
            let n = buf.len().min(self.left.min(usize::MAX as u64) as usize);
            buf[..n].fill(0);
            self.left -= n as u64;
            self.read += n as u64;
            if let Some((at, flag)) = &self.on_read
                && self.read >= *at
            {
                flag.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            Ok(n)
        }
    }

    #[test]
    fn huge_long_name_is_skipped_in_bounded_chunks() {
        const HUGE: u64 = 1 << 30;
        let head = block("././@LongLink", b'L', HUGE).to_vec();
        let tail = [file("ok.txt", b"1"), vec![0; 1024]].concat();
        let mut r = Cursor::new(head).chain(Zeros { left: HUGE, read: 0, on_read: None }).chain(Cursor::new(tail));
        let l = walk_tar(&mut r, skip_read, Path::new("t.tar"), 10).unwrap();
        assert_eq!(names(&l), ["ok.txt"]);
    }

    #[test]
    fn cancellation_stops_a_huge_skip_early() {
        use std::sync::{Arc, atomic::AtomicBool};
        const HUGE: u64 = 1 << 30;
        let flag = Arc::new(AtomicBool::new(false));
        let z = Zeros { left: HUGE, read: 0, on_read: Some((1 << 20, flag.clone())) };
        let mut r = super::super::list::Cancellable::new(Cursor::new(block("big", b'0', HUGE).to_vec()).chain(z), &flag);
        let e = walk_tar(&mut r, skip_read, Path::new("t.tar"), 10).unwrap_err();
        assert_eq!(e.code, ErrorCode::CorruptArchive); // `list_archive` lo trasforma in CANCELLED a flag alzato
        assert!(flag.load(std::sync::atomic::Ordering::Relaxed));
    }

    #[test]
    fn old_style_directory_has_a_trailing_slash() {
        let l = walk(&[block("old/", b'0', 0).to_vec(), block("real", b'5', 0).to_vec()], 10).unwrap();
        assert_eq!(names(&l), ["old", "real"]);
        assert!(l.entries.iter().all(|e| e.is_dir));
    }

    #[test]
    fn total_is_known_only_when_the_whole_archive_was_read() {
        let parts = [file("a", b"1"), file("b", b"2"), file("c", b"3")];
        let l = walk(&parts, 3).unwrap();
        assert_eq!((l.entries.len(), l.total, l.truncated), (3, Some(3), false));
        let l = walk(&parts, 2).unwrap();
        assert_eq!((l.entries.len(), l.total, l.truncated), (2, None, true));
    }

    #[test]
    fn base_256_sizes_and_octal_parse() {
        assert_eq!(parse_num(b"0000644\0").unwrap(), 0o644);
        assert_eq!(parse_num(b"   17 \0").unwrap(), 0o17);
        assert_eq!(parse_num(&[0u8; 12]).unwrap(), 0);
        let mut f = [0u8; 12];
        f[0] = 0x80;
        f[8..].copy_from_slice(&[0, 0, 0x01, 0]);
        assert_eq!(parse_num(&f).unwrap(), 0x100);
        assert!(parse_num(b"12x4\0").is_none());
        assert!(parse_num(&[0xFF; 12]).is_none()); // base 256 negativo
    }

    #[test]
    fn garbage_bad_checksum_and_truncation_are_corrupt() {
        let junk: Vec<u8> = (0..700u32).map(|i| (i.wrapping_mul(37) + 11) as u8).collect();
        assert_eq!(walk(&[junk], 10).unwrap_err().code, ErrorCode::CorruptArchive);
        let mut b = block("a", b'0', 0);
        b[0] = b'z'; // la somma non torna
        assert_eq!(walk(&[b.to_vec()], 10).unwrap_err().code, ErrorCode::CorruptArchive);
        let mut r = Cursor::new(block("a", b'0', 0)[..100].to_vec());
        assert_eq!(walk_tar(&mut r, skip_read, Path::new("t.tar"), 10).unwrap_err().code, ErrorCode::CorruptArchive);
        let mut r = Cursor::new([block("a", b'0', 1000).to_vec(), vec![1; 10]].concat());
        assert_eq!(walk_tar(&mut r, skip_read, Path::new("t.tar"), 10).unwrap_err().code, ErrorCode::CorruptArchive);
    }

    #[test]
    fn empty_and_zero_block_archives_are_empty_listings() {
        let mut r = Cursor::new(Vec::new());
        let l = walk_tar(&mut r, skip_read, Path::new("t.tar"), 10).unwrap();
        assert_eq!((l.entries.len(), l.total), (0, Some(0)));
        assert!(walk(&[], 10).unwrap().entries.is_empty());
    }

    #[test]
    fn gnu_sparse_skips_map_blocks_and_shows_the_real_size() {
        let mut h = block("sparse.bin", b'S', 512);
        h[482] = 1; // seguono blocchi di mappa
        octal(&mut h[483..495], 5000);
        reseal(&mut h);
        let mut ext = [0u8; 512];
        ext[504] = 0; // ultimo blocco di mappa
        ext[0] = b'1'; // non zero: non è la fine dell'archivio
        let l = walk(&[h.to_vec(), ext.to_vec(), data(&[9; 512]), file("next", b"")], 10).unwrap();
        assert_eq!(names(&l), ["sparse.bin", "next"]);
        assert_eq!(l.entries[0].size, 5000);
    }
}

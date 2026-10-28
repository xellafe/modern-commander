//! Protezione dei 7z: `sevenz-rust2` crea i decoder senza limite di memoria (dizionario LZMA/LZMA2 e memoria PPMd
//! fino a 4 GiB) e con `panic = "abort"` un'allocazione fallita chiude l'app. Qui si leggono le proprietà dei coder
//! prima che il crate li usi: quelli dell'intestazione codificata, che il crate decodifica già dentro
//! `Archive::read` (`check_header`), e quelli dei blocchi dei dati, decodificati all'estrazione (`check_blocks`).

use super::list::io_err;
use crate::error::{AppError, ErrorCode};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Memoria per decoder oltre cui il 7z si rifiuta: i 7z normali usano dizionari fino a 64 MiB, "Ultra" fino a 192.
const MAX_MEMORY: u64 = 256 << 20;

/// Byte dell'intestazione codificata letti per il controllo: contiene solo la descrizione di un blocco.
const MAX_ENCODED_HEADER: u64 = 64 << 10;

/// Gli errori non hanno percorso: lo aggiunge `index::build_7z`.
fn corrupt(detail: &str) -> AppError {
    AppError { detail: Some(detail.into()), ..AppError::new(ErrorCode::CorruptArchive, None) }
}

/// Rifiuta un coder LZMA, LZMA2 o PPMd che chiederebbe oltre `MAX_MEMORY`; gli altri metodi passano.
pub fn check_coder(method_id: &[u8], props: &[u8]) -> Result<(), AppError> {
    let le32 = || props.get(1..5).map(|b| u64::from(u32::from_le_bytes([b[0], b[1], b[2], b[3]])));
    let memory = match method_id {
        // LZMA: dizionario; PPMd: memoria del modello. Entrambi dopo il primo byte.
        [0x03, 0x01, 0x01] | [0x03, 0x04, 0x01] => le32(),
        [0x21] => match props.first() {
            Some(40) => Some(u64::from(u32::MAX)),
            Some(&p) if p < 40 => Some(u64::from(2 | (p & 1)) << (p / 2 + 11)),
            _ => None,
        },
        _ => return Ok(()),
    };
    match memory {
        None => Err(corrupt("proprietà del coder 7z non valide")),
        Some(m) if m > MAX_MEMORY => Err(AppError::new(ErrorCode::ArchiveTooDemanding, None)),
        Some(_) => Ok(()),
    }
}

/// Controlla i coder dell'intestazione codificata, che `Archive::read` decodifica: si chiama prima del crate.
/// Un'intestazione in chiaro non decodifica nulla (i suoi blocchi passano da `check_blocks`).
pub fn check_header(r: &mut (impl Read + Seek)) -> Result<(), AppError> {
    let io = |e: std::io::Error| AppError { path: None, ..io_err(&e, Path::new("")) };
    let mut start = [0u8; 32];
    r.seek(SeekFrom::Start(0)).and_then(|_| r.read_exact(&mut start)).map_err(io)?;
    // Firma e versione le controlla il crate. Con l'intestazione di avvio a zero (archivio scritto a metà) il crate
    // cercherebbe da solo l'intestazione finale e la decodificherebbe senza passare da qui.
    // ponytail: questi archivi si rifiutano; se servono, ripetere qui la stessa ricerca.
    if start[8..].iter().all(|&b| b == 0) {
        return Err(corrupt("intestazione di avvio 7z vuota"));
    }
    let le64 = |i: usize| u64::from_le_bytes(start[i..i + 8].try_into().expect("8 byte"));
    let at = le64(12).checked_add(32).ok_or_else(|| corrupt("intestazione 7z fuori dal file"))?;
    let mut header = Vec::new();
    r.seek(SeekFrom::Start(at)).map_err(io)?;
    r.take(le64(20).min(MAX_ENCODED_HEADER)).read_to_end(&mut header).map_err(io)?;
    match header.split_first() {
        Some((0x17, rest)) => check_streams_info(&mut Bytes(rest)),
        _ => Ok(()),
    }
}

/// `check_coder` su tutti i coder di tutti i blocchi: da chiamare prima di decodificarne uno.
pub fn check_blocks(a: &sevenz_rust2::Archive) -> Result<(), AppError> {
    a.blocks.iter().flat_map(|b| &b.coders).try_for_each(|c| check_coder(c.encoder_method_id(), c.properties()))
}

/// Lettura di un'intestazione già in memoria; ogni lettura oltre la fine è un archivio danneggiato.
struct Bytes<'a>(&'a [u8]);

impl<'a> Bytes<'a> {
    fn take(&mut self, n: u64) -> Result<&'a [u8], AppError> {
        let n = usize::try_from(n).ok().filter(|&n| n <= self.0.len()).ok_or_else(|| corrupt("intestazione 7z troncata"))?;
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(head)
    }

    fn u8(&mut self) -> Result<u8, AppError> {
        Ok(self.take(1)?[0])
    }

    /// Numero a lunghezza variabile del 7z: i bit alti del primo byte dicono quanti byte seguono.
    fn num(&mut self) -> Result<u64, AppError> {
        let first = self.u8()?;
        let mut value = 0u64;
        for i in 0..8 {
            let mask = 0x80u8 >> i;
            if first & mask == 0 {
                return Ok(value | (u64::from(first & mask.wrapping_sub(1)) << (8 * i)));
            }
            value |= u64::from(self.u8()?) << (8 * i);
        }
        Ok(value)
    }

    /// Vettore di `n` bit preceduto dal byte "tutti definiti"; restituisce quanti sono a uno.
    fn defined(&mut self, n: u64) -> Result<u64, AppError> {
        if self.u8()? != 0 {
            return Ok(n);
        }
        let bits = self.take(n.div_ceil(8))?;
        Ok((0..n as usize).filter(|i| bits[i / 8] & (0x80 >> (i % 8)) != 0).count() as u64)
    }
}

/// `StreamsInfo` dell'intestazione codificata, letto come fa il crate fino ai coder di ogni folder. Ogni ciclo
/// consuma almeno un byte: un conteggio assurdo finisce contro la fine del buffer.
fn check_streams_info(h: &mut Bytes) -> Result<(), AppError> {
    let mut nid = h.u8()?;
    if nid == 0x06 {
        // PackInfo: posizione, flussi, dimensioni e somme di controllo facoltative.
        h.num()?;
        let streams = h.num()?;
        nid = h.u8()?;
        if nid == 0x09 {
            for _ in 0..streams {
                h.num()?;
            }
            nid = h.u8()?;
        }
        if nid == 0x0A {
            let crcs = h.defined(streams)?;
            h.take(crcs.saturating_mul(4))?;
            nid = h.u8()?;
        }
        if nid != 0x00 {
            return Err(corrupt("PackInfo 7z non terminato"));
        }
        nid = h.u8()?;
    }
    if nid != 0x07 {
        // Nessun folder: il crate non ha niente da decodificare e rifiuta l'intestazione.
        return Ok(());
    }
    if h.u8()? != 0x0B {
        return Err(corrupt("UnpackInfo 7z senza folder"));
    }
    let folders = h.num()?;
    // Folder "esterni": nemmeno il crate li gestisce.
    if h.u8()? != 0 {
        return Err(corrupt("folder 7z esterni"));
    }
    for _ in 0..folders {
        check_folder(h)?;
    }
    Ok(())
}

/// Un folder: i coder (controllati), poi le coppie di collegamento e i flussi compressi per arrivare al successivo.
fn check_folder(h: &mut Bytes) -> Result<(), AppError> {
    let bad = || corrupt("folder 7z non valido");
    let (mut ins, mut outs) = (0u64, 0u64);
    for _ in 0..h.num()? {
        let flags = h.u8()?;
        let id = h.take(u64::from(flags & 0x0F))?;
        let (i, o) = if flags & 0x10 == 0 { (1, 1) } else { (h.num()?, h.num()?) };
        ins = ins.checked_add(i).ok_or_else(bad)?;
        outs = outs.checked_add(o).ok_or_else(bad)?;
        let props = if flags & 0x20 != 0 {
            let n = h.num()?;
            h.take(n)?
        } else {
            &[]
        };
        check_coder(id, props)?;
        // Metodi alternativi: nemmeno il crate li gestisce.
        if flags & 0x80 != 0 {
            return Err(bad());
        }
    }
    let pairs = outs.checked_sub(1).ok_or_else(bad)?;
    for _ in 0..pairs {
        h.num()?;
        h.num()?;
    }
    let packed = ins.checked_sub(pairs).ok_or_else(bad)?;
    if packed > 1 {
        for _ in 0..packed {
            h.num()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::container::open_local;
    use crate::archive::index::{EntryAt, build_7z};
    use crate::error::ErrorCode;
    use std::path::Path;
    use std::sync::atomic::AtomicBool;

    fn code(r: Result<(), AppError>) -> Option<ErrorCode> {
        r.err().map(|e| e.code)
    }

    #[test]
    fn lzma2_dictionary_limits() {
        let lzma2 = |p: &[u8]| code(check_coder(&[0x21], p));
        assert_eq!(lzma2(&[32]), None); // 256 MiB
        assert_eq!(lzma2(&[33]), Some(ErrorCode::ArchiveTooDemanding)); // 384 MiB
        assert_eq!(lzma2(&[40]), Some(ErrorCode::ArchiveTooDemanding)); // 4 GiB - 1
        assert_eq!(lzma2(&[41]), Some(ErrorCode::CorruptArchive));
        assert_eq!(lzma2(&[]), Some(ErrorCode::CorruptArchive));
    }

    #[test]
    fn lzma_dictionary_limits() {
        let lzma = |p: &[u8]| code(check_coder(&[0x03, 0x01, 0x01], p));
        assert_eq!(lzma(&[0x5D, 0, 0, 0, 0x10]), None);
        assert_eq!(lzma(&[0x5D, 0, 0, 0, 0x20]), Some(ErrorCode::ArchiveTooDemanding));
        assert_eq!(lzma(&[0x5D, 0, 0]), Some(ErrorCode::CorruptArchive));
        // PPMd: memoria nello stesso punto; gli altri metodi passano.
        assert_eq!(code(check_coder(&[0x03, 0x04, 0x01], &[6, 0, 0, 0, 0x20])), Some(ErrorCode::ArchiveTooDemanding));
        assert_eq!(code(check_coder(&[0x04, 0x02, 0x02], &[])), None);
    }

    fn crc(b: &[u8]) -> u32 {
        let mut c = flate2::Crc::new();
        c.update(b);
        c.sum()
    }

    /// 7z con intestazione codificata da un solo coder LZMA con dizionario `dict` su 5 byte di dati finti, con le
    /// somme di controllo giuste perché il crate arrivi fino al decoder.
    fn crafted(dict: u32) -> Vec<u8> {
        // kEncodedHeader, PackInfo (pos 0, 1 flusso da 5 byte), UnpackInfo (1 folder, 1 coder LZMA con proprietà).
        let mut header = vec![0x17, 0x06, 0x00, 0x01, 0x09, 0x05, 0x00, 0x07, 0x0B, 0x01, 0x00, 0x01, 0x23, 0x03, 0x01, 0x01, 0x05, 0x5D];
        header.extend(dict.to_le_bytes());
        // Dimensione decompressa 10, fine di UnpackInfo e dello StreamsInfo.
        header.extend([0x0C, 0x0A, 0x00, 0x00]);
        let mut start = 5u64.to_le_bytes().to_vec();
        start.extend((header.len() as u64).to_le_bytes());
        start.extend(crc(&header).to_le_bytes());
        let mut out = b"7z\xBC\xAF\x27\x1C\x00\x04".to_vec();
        out.extend(crc(&start).to_le_bytes());
        out.extend(start);
        out.extend([0u8; 5]);
        out.extend(header);
        out
    }

    fn build(p: &Path) -> Result<(crate::archive::index::Index, sevenz_rust2::Archive), AppError> {
        build_7z(open_local(p).unwrap(), &AtomicBool::new(false))
    }

    #[test]
    fn crafted_encoded_header_is_refused_before_the_crate() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("bomb.7z");
        std::fs::write(&p, crafted(u32::MAX)).unwrap();
        assert_eq!(build(&p).unwrap_err().code, ErrorCode::ArchiveTooDemanding);
        // Lo stesso archivio con un dizionario piccolo arriva al crate, che non trova un'intestazione valida.
        std::fs::write(&p, crafted(1 << 20)).unwrap();
        assert_eq!(build(&p).unwrap_err().code, ErrorCode::CorruptArchive);
        // Intestazione di avvio vuota: il crate cercherebbe da solo l'intestazione finale e la decodificherebbe.
        let mut empty = crafted(1 << 20);
        empty[8..32].fill(0);
        std::fs::write(&p, empty).unwrap();
        assert_eq!(build(&p).unwrap_err().code, ErrorCode::CorruptArchive);
    }

    #[test]
    fn normal_7z_passes() {
        use sevenz_rust2::{ArchiveEntry, ArchiveWriter, SourceReader};
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("solid.7z");
        let mut w = ArchiveWriter::create(&p).unwrap();
        let files = vec![ArchiveEntry::new_file("a.txt"), ArchiveEntry::new_file("d/b.txt")];
        w.push_archive_entries(files, vec![SourceReader::new(&b"abc"[..]), SourceReader::new(&b"de"[..])]).unwrap();
        w.finish().unwrap();
        let (ix, archive) = build(&p).unwrap();
        assert!(ix.solid);
        assert!(check_blocks(&archive).is_ok());
        let a = ix.get("/a.txt").unwrap();
        assert_eq!((a.size, a.at), (3, EntryAt::SevenZ { file: 0, block: Some(0) }));
        assert_eq!(ix.get("/d/b.txt").unwrap().at, EntryAt::SevenZ { file: 1, block: Some(0) });
        assert!(ix.stat("/d").unwrap().is_dir);
    }
}

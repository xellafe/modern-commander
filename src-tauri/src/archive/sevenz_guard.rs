//! Protezione dei 7z: `sevenz-rust2` crea i decoder senza limite di memoria (dizionario LZMA/LZMA2 e memoria PPMd
//! fino a 4 GiB) e alloca le strutture dell'intestazione per i conteggi che vi trova, limitati solo dalla lunghezza
//! dell'intestazione (che una intestazione codificata dichiara a piacere); con `panic = "abort"` un'allocazione
//! fallita chiude l'app. Prima di `Archive::read`, `check_header` legge l'intestazione come farà il crate (decodificando
//! quella codificata, con un tetto) e rifiuta coder troppo esigenti e conteggi oltre `MAX_ENTRIES`; i blocchi dei
//! dati, decodificati all'estrazione, passano da `check_blocks`.
//!
//! Conteggi del crate che diventano allocazioni (`reader.rs` di 0.24.1) e dove si controllano qui: voci (`numFiles`,
//! con i loro vettori di bit), folder, flussi compressi (con le loro CRC), sottoflussi (dimensioni, CRC, digest) con
//! `count`; coder di un folder e loro flussi (coppie di collegamento, flussi compressi, dimensioni) con
//! `MAX_FOLDER_STREAMS`, coder in tutto con `MAX_CODERS`; proprietà dei coder, nomi e intestazione stessa con
//! `MAX_HEADER` (un byte per byte).

use super::index::MAX_ENTRIES;
use super::list::io_err;
use crate::error::{AppError, ErrorCode};
use lzma_rust2::{Lzma2Reader, LzmaReader};
use sevenz_rust2::EncoderMethod;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

/// Memoria per decoder oltre cui il 7z si rifiuta: i 7z normali usano dizionari fino a 64 MiB, "Ultra" fino a 192.
const MAX_MEMORY: u64 = 256 << 20;

/// Intestazione codificata: contiene solo la descrizione di un blocco, qualche decina di byte.
const MAX_ENCODED_HEADER: u64 = 64 << 10;

/// Intestazione in chiaro (letta o decodificata) oltre cui il 7z si rifiuta: il crate la tiene tutta in memoria. Come
/// il tetto dei nomi di un TAR: un milione di voci con nomi lunghi ci sta.
const MAX_HEADER: u64 = 128 << 20;

/// Coder e flussi di un folder: 7-Zip non ne scrive più di 64, e il crate prealloca per il numero dichiarato.
const MAX_FOLDER_STREAMS: u64 = 64;

/// Coder in tutto l'archivio (~64 byte l'uno nel crate): un milione di blocchi con filtro e compressore ci stanno.
const MAX_CODERS: u64 = 4 * MAX_ENTRIES as u64;

/// Gli errori non hanno percorso: lo aggiunge `index::build_7z`.
fn corrupt(detail: &str) -> AppError {
    AppError { detail: Some(detail.into()), ..AppError::new(ErrorCode::CorruptArchive, None) }
}

fn unsupported(detail: Option<&str>) -> AppError {
    AppError { detail: detail.map(Into::into), ..AppError::new(ErrorCode::Unsupported, None) }
}

fn too_demanding() -> AppError {
    AppError::new(ErrorCode::ArchiveTooDemanding, None)
}

/// Un conteggio che il crate userà per allocare: oltre `MAX_ENTRIES` l'archivio non si apre.
fn count(n: u64) -> Result<u64, AppError> {
    if n > MAX_ENTRIES as u64 { Err(unsupported(Some("too_many_entries"))) } else { Ok(n) }
}

/// Rifiuta un coder LZMA, LZMA2 o PPMd che chiederebbe oltre `MAX_MEMORY`; gli altri metodi passano.
pub fn check_coder(method_id: &[u8], props: &[u8]) -> Result<(), AppError> {
    let le32 = || props.get(1..5).map(|b| u64::from(u32::from_le_bytes([b[0], b[1], b[2], b[3]])));
    let memory = match method_id {
        // LZMA: dizionario; PPMd: memoria del modello. Entrambi dopo il primo byte.
        [0x03, 0x01, 0x01] | [0x03, 0x04, 0x01] => le32(),
        [0x21] => props.first().and_then(|&p| lzma2_dict(p)).map(u64::from),
        _ => return Ok(()),
    };
    match memory {
        None => Err(corrupt("proprietà del coder 7z non valide")),
        Some(m) if m > MAX_MEMORY => Err(too_demanding()),
        Some(_) => Ok(()),
    }
}

/// Dizionario LZMA2 dal byte delle proprietà, come `get_lzma2_dic_size` del crate.
fn lzma2_dict(p: u8) -> Option<u32> {
    match p {
        40 => Some(u32::MAX),
        p if p < 40 => Some((2 | u32::from(p & 1)) << (p / 2 + 11)),
        _ => None,
    }
}

/// Le parti del file che `Archive::read` legge (intestazione di avvio, intestazione successiva e, se è codificata, il
/// suo flusso compresso), lette una volta sola da `check_header`: il crate analizza esattamente i byte controllati,
/// anche se il contenitore (o il server) cambia tra una lettura e l'altra. Fuori dalle parti non si legge.
pub struct Image {
    len: u64,
    pos: u64,
    parts: Vec<(u64, Vec<u8>)>,
}

impl Read for Image {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let pos = self.pos;
        let Some((at, data)) = self.parts.iter().find(|(at, d)| (*at..*at + d.len() as u64).contains(&pos)) else {
            return if pos >= self.len { Ok(0) } else { Err(io::Error::other("lettura 7z fuori dall'intestazione controllata")) };
        };
        let start = (pos - at) as usize;
        let n = buf.len().min(data.len() - start);
        buf[..n].copy_from_slice(&data[start..start + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for Image {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let to = match pos {
            SeekFrom::Start(n) => Some(n),
            SeekFrom::End(d) => self.len.checked_add_signed(d),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
        };
        self.pos = to.ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
        Ok(self.pos)
    }
}

/// Fino a `n` byte da `at` (meno a fine file).
fn read_part(r: &mut (impl Read + Seek), at: u64, n: u64) -> Result<Vec<u8>, AppError> {
    let io = |e: io::Error| AppError { path: None, ..io_err(&e, Path::new("")) };
    let mut out = Vec::new();
    r.seek(SeekFrom::Start(at)).map_err(io)?;
    r.take(n).read_to_end(&mut out).map_err(io)?;
    Ok(out)
}

/// Controlla l'intestazione prima del crate e ne restituisce le parti, da passare ad `Archive::read` al posto del
/// contenitore.
pub fn check_header(r: &mut (impl Read + Seek)) -> Result<Image, AppError> {
    let len = r.seek(SeekFrom::End(0)).map_err(|e| AppError { path: None, ..io_err(&e, Path::new("")) })?;
    let start = read_part(r, 0, 32)?;
    // Senza firma non è un 7z: i byte a caso non devono sembrare un dizionario enorme. La versione la controlla il crate.
    if start.len() < 32 || start[..6] != *b"7z\xBC\xAF\x27\x1C" {
        return Err(corrupt("firma 7z assente"));
    }
    // Con l'intestazione di avvio a zero (archivio scritto a metà) il crate cercherebbe da solo l'intestazione
    // finale e la decodificherebbe senza passare da qui.
    // ponytail: questi archivi si rifiutano; se servono, ripetere qui la stessa ricerca.
    if start[8..].iter().all(|&b| b == 0) {
        return Err(corrupt("intestazione di avvio 7z vuota"));
    }
    let le64 = |i: usize| u64::from_le_bytes(start[i..i + 8].try_into().expect("8 byte"));
    let (at, size) = (le64(12).checked_add(32), le64(20));
    let at = at.filter(|at| at.checked_add(size).is_some_and(|end| end <= len)).ok_or_else(|| corrupt("intestazione 7z fuori dal file"))?;
    if size > MAX_HEADER {
        return Err(too_demanding());
    }
    let header = read_part(r, at, size)?;
    let mut parts = vec![(0, start)];
    if header.first() == Some(&0x17) {
        if size > MAX_ENCODED_HEADER {
            return Err(corrupt("intestazione codificata 7z troppo lunga"));
        }
        let (pack_at, packed, plain) = decode_header(r, &header[1..])?;
        if pack_at < at + size && at < pack_at + packed.len() as u64 {
            return Err(corrupt("intestazione 7z sovrapposta ai suoi dati"));
        }
        check_counts(&plain)?;
        parts.push((pack_at, packed));
    } else {
        check_counts(&header)?;
    }
    parts.push((at, header));
    Ok(Image { len, pos: 0, parts })
}

/// `check_coder` su tutti i coder di tutti i blocchi: da chiamare prima di decodificarne uno.
pub fn check_blocks(a: &sevenz_rust2::Archive) -> Result<(), AppError> {
    a.blocks.iter().flat_map(|b| &b.coders).try_for_each(|c| check_coder(c.encoder_method_id(), c.properties()))
}

/// Il blocco dell'intestazione codificata (`encoded` senza il primo byte), decodificato con un tetto alla dimensione
/// dichiarata. Solo un coder Copy, LZMA o LZMA2: 7-Zip la comprime con LZMA (e la cifra con AES, che senza password
/// non si legge). Restituisce posizione e byte del flusso compresso e l'intestazione in chiaro.
fn decode_header(r: &mut (impl Read + Seek), encoded: &[u8]) -> Result<(u64, Vec<u8>, Vec<u8>), AppError> {
    let s = streams_info(&mut Bytes(encoded))?;
    let (coders, sizes) = s.first.ok_or_else(|| corrupt("intestazione codificata 7z senza folder"))?;
    if coders.iter().any(|c| c.0 == EncoderMethod::ID_AES256_SHA256) {
        return Err(unsupported(Some("encrypted")));
    }
    let ([(id, props, true)], [unpack]) = (&coders[..], &sizes[..]) else { return Err(unsupported(None)) };
    let packed_len = s.first_pack.ok_or_else(|| corrupt("intestazione codificata 7z senza flusso"))?;
    if *unpack > MAX_HEADER || packed_len > MAX_HEADER {
        return Err(too_demanding());
    }
    let at = s.pack_pos.checked_add(32).ok_or_else(|| corrupt("intestazione 7z fuori dal file"))?;
    let packed = read_part(r, at, packed_len)?;
    let bad = |e: io::Error| corrupt(&format!("intestazione codificata 7z non valida: {e}"));
    let input = packed.as_slice();
    let mut decoder: Box<dyn Read + '_> = match *id {
        EncoderMethod::ID_COPY => Box::new(input),
        // Proprietà già controllate da `check_coder` (in `folder`).
        EncoderMethod::ID_LZMA => {
            let dict = u32::from_le_bytes(props[1..5].try_into().expect("4 byte"));
            Box::new(LzmaReader::new_with_props(input, *unpack, props[0], dict, None).map_err(bad)?)
        }
        EncoderMethod::ID_LZMA2 => Box::new(Lzma2Reader::new(input, lzma2_dict(props[0]).expect("controllato"), None)),
        _ => return Err(unsupported(None)),
    };
    let mut plain = Vec::new();
    decoder.by_ref().take(*unpack).read_to_end(&mut plain).map_err(bad)?;
    drop(decoder);
    if plain.len() as u64 != *unpack {
        return Err(corrupt("intestazione codificata 7z troncata"));
    }
    Ok((at, packed, plain))
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

    /// Vettore di `n` bit preceduto dal byte "tutti definiti", come `read_all_or_bits` del crate.
    fn defined(&mut self, n: u64) -> Result<Vec<bool>, AppError> {
        let n = count(n)? as usize;
        if self.u8()? != 0 {
            return Ok(vec![true; n]);
        }
        let bits = self.take(n.div_ceil(8) as u64)?;
        Ok((0..n).map(|i| bits[i / 8] & (0x80 >> (i % 8)) != 0).collect())
    }
}

/// Coder di un folder: metodo, proprietà, un solo ingresso e una sola uscita.
type Coder<'a> = (&'a [u8], &'a [u8], bool);

#[derive(Default)]
struct Streams<'a> {
    pack_pos: u64,
    first_pack: Option<u64>,
    /// Coder del primo folder e dimensioni delle sue uscite: quanto serve a decodificare l'intestazione.
    first: Option<(Vec<Coder<'a>>, Vec<u64>)>,
}

/// L'intestazione in chiaro (`kHeader`), letta come `read_header` del crate fino al numero di voci.
fn check_counts(header: &[u8]) -> Result<(), AppError> {
    let h = &mut Bytes(header);
    if h.u8()? != 0x01 {
        return Err(corrupt("intestazione 7z assente"));
    }
    let mut nid = h.u8()?;
    if nid == 0x02 {
        // Proprietà dell'archivio: tipo, dimensione e byte, fino allo zero.
        while h.u8()? != 0x00 {
            let n = h.num()?;
            h.take(n)?;
        }
        nid = h.u8()?;
    }
    // Flussi aggiuntivi: il crate li rifiuta subito.
    if nid == 0x03 {
        return Ok(());
    }
    if nid == 0x04 {
        streams_info(h)?;
        nid = h.u8()?;
    }
    if nid == 0x05 {
        count(h.num()?)?;
    }
    Ok(())
}

/// `StreamsInfo`, letto come `read_streams_info` del crate; ogni conteggio che il crate usa per allocare passa da
/// `count` (o dai tetti dei coder). Ogni ciclo consuma almeno un byte: un conteggio assurdo finisce contro la fine.
fn streams_info<'a>(h: &mut Bytes<'a>) -> Result<Streams<'a>, AppError> {
    let mut s = Streams::default();
    let mut nid = h.u8()?;
    if nid == 0x06 {
        // PackInfo: posizione, flussi, dimensioni e somme di controllo facoltative.
        s.pack_pos = h.num()?;
        let streams = count(h.num()?)?;
        nid = h.u8()?;
        if nid == 0x09 {
            for i in 0..streams {
                let n = h.num()?;
                if i == 0 {
                    s.first_pack = Some(n);
                }
            }
            nid = h.u8()?;
        }
        if nid == 0x0A {
            let crcs = h.defined(streams)?.into_iter().filter(|&d| d).count() as u64;
            h.take(crcs * 4)?;
            nid = h.u8()?;
        }
        if nid != 0x00 {
            return Err(corrupt("PackInfo 7z non terminato"));
        }
        nid = h.u8()?;
    }
    // Uscite e somma di controllo di ogni folder.
    let mut folders: Vec<(u64, bool)> = Vec::new();
    if nid == 0x07 {
        if h.u8()? != 0x0B {
            return Err(corrupt("UnpackInfo 7z senza folder"));
        }
        let n = count(h.num()?)?;
        // Folder "esterni": nemmeno il crate li gestisce.
        if h.u8()? != 0 {
            return Err(corrupt("folder 7z esterni"));
        }
        let mut coders = 0;
        for i in 0..n {
            let (list, outs) = folder(h)?;
            coders += list.len() as u64;
            if coders > MAX_CODERS {
                return Err(count(u64::MAX).unwrap_err());
            }
            if i == 0 {
                s.first = Some((list, Vec::new()));
            }
            folders.push((outs, false));
        }
        if h.u8()? != 0x0C {
            return Err(corrupt("UnpackInfo 7z senza dimensioni"));
        }
        for (i, &(outs, _)) in folders.iter().enumerate() {
            for _ in 0..outs {
                let size = h.num()?;
                if let Some((_, sizes)) = s.first.as_mut().filter(|_| i == 0) {
                    sizes.push(size);
                }
            }
        }
        nid = h.u8()?;
        if nid == 0x0A {
            for (f, defined) in folders.iter_mut().zip(h.defined(n)?) {
                if defined {
                    f.1 = true;
                    h.take(4)?;
                }
            }
            nid = h.u8()?;
        }
        if nid != 0x00 {
            return Err(corrupt("UnpackInfo 7z non terminato"));
        }
        nid = h.u8()?;
    }
    if nid == 0x08 {
        substreams(h, &folders)?;
        nid = h.u8()?;
    }
    if nid != 0x00 {
        return Err(corrupt("StreamsInfo 7z non terminato"));
    }
    Ok(s)
}

/// Un folder: i coder (controllati), poi le coppie di collegamento e i flussi compressi per arrivare al successivo.
/// Restituisce i coder e il numero di uscite.
fn folder<'a>(h: &mut Bytes<'a>) -> Result<(Vec<Coder<'a>>, u64), AppError> {
    let bad = || corrupt("folder 7z non valido");
    let n = h.num()?;
    if n == 0 || n > MAX_FOLDER_STREAMS {
        return Err(bad());
    }
    let (mut ins, mut outs, mut coders) = (0u64, 0u64, Vec::new());
    for _ in 0..n {
        let flags = h.u8()?;
        let id = h.take(u64::from(flags & 0x0F))?;
        let simple = flags & 0x10 == 0;
        let (i, o) = if simple { (1, 1) } else { (h.num()?, h.num()?) };
        ins = ins.checked_add(i).ok_or_else(bad)?;
        outs = outs.checked_add(o).ok_or_else(bad)?;
        if ins > MAX_FOLDER_STREAMS || outs > MAX_FOLDER_STREAMS {
            return Err(bad());
        }
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
        coders.push((id, props, simple));
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
    Ok((coders, outs))
}

/// `SubStreamsInfo`, come `read_sub_streams_info` del crate: il totale dei sottoflussi passa da `count`.
fn substreams(h: &mut Bytes, folders: &[(u64, bool)]) -> Result<(), AppError> {
    let mut subs = vec![1u64; folders.len()];
    let mut nid = h.u8()?;
    if nid == 0x0D {
        let mut total = 0u64;
        for s in subs.iter_mut() {
            *s = h.num()?;
            total = count(total.saturating_add(*s))?;
        }
        nid = h.u8()?;
    }
    if nid == 0x09 {
        for &s in &subs {
            for _ in 1..s {
                h.num()?;
            }
        }
        nid = h.u8()?;
    }
    // Una CRC per sottoflusso, tranne per i folder con un solo sottoflusso e la CRC del folder.
    let digests = folders.iter().zip(&subs).filter(|&(&(_, crc), &s)| !(s == 1 && crc)).map(|(_, &s)| s).sum();
    if nid == 0x0A {
        let crcs = h.defined(digests)?.into_iter().filter(|&d| d).count() as u64;
        h.take(crcs * 4)?;
        nid = h.u8()?;
    }
    if nid != 0x00 {
        return Err(corrupt("SubStreamsInfo 7z non terminato"));
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

    /// 7z con `packed` subito dopo l'intestazione di avvio e `header` come intestazione successiva, con le somme di
    /// controllo giuste perché il crate arrivi fino in fondo.
    fn seven_z(packed: &[u8], header: &[u8]) -> Vec<u8> {
        let mut start = (packed.len() as u64).to_le_bytes().to_vec();
        start.extend((header.len() as u64).to_le_bytes());
        start.extend(crc(header).to_le_bytes());
        let mut out = b"7z\xBC\xAF\x27\x1C\x00\x04".to_vec();
        out.extend(crc(&start).to_le_bytes());
        out.extend(start);
        out.extend(packed);
        out.extend(header);
        out
    }

    /// Numero 7z nella forma lunga (`0xFF` e 8 byte): sempre valida.
    fn num(v: u64) -> Vec<u8> {
        [&[0xFF][..], &v.to_le_bytes()].concat()
    }

    /// Intestazione codificata da un solo coder (`coder`: flag, metodo e proprietà) su `packed` byte dall'inizio dei
    /// dati, che si decomprimono in `unpack` byte.
    fn encoded(coder: &[u8], packed: u64, unpack: u64) -> Vec<u8> {
        // kEncodedHeader, PackInfo (pos 0, 1 flusso), UnpackInfo (1 folder, 1 coder), dimensione, fine.
        let parts: [&[u8]; 7] = [&[0x17, 0x06, 0x00, 0x01, 0x09], &num(packed), &[0x00, 0x07, 0x0B, 0x01, 0x00, 0x01], coder, &[0x0C], &num(unpack), &[0x00, 0x00]];
        parts.concat()
    }

    /// 7z con intestazione codificata da un solo coder LZMA con dizionario `dict` su 5 byte di dati finti.
    fn crafted(dict: u32) -> Vec<u8> {
        let coder = [&[0x23, 0x03, 0x01, 0x01, 0x05, 0x5D][..], &dict.to_le_bytes()].concat();
        seven_z(&[0u8; 5], &encoded(&coder, 5, 10))
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
        // Senza la firma non è un 7z: danneggiato, non troppo esigente.
        let mut unsigned = crafted(u32::MAX);
        unsigned[0] = b'X';
        std::fs::write(&p, unsigned).unwrap();
        assert_eq!(build(&p).unwrap_err().code, ErrorCode::CorruptArchive);
    }

    fn refused(bytes: Vec<u8>) -> (ErrorCode, Option<String>) {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x.7z");
        std::fs::write(&p, bytes).unwrap();
        let e = build(&p).unwrap_err();
        (e.code, e.detail)
    }

    #[test]
    fn encoded_header_declaring_a_huge_unpack_size_is_refused() {
        // Come il 7z di prova della revisione: intestazione Deflate che dichiara 400 MB (in chiaro, tante voci quanti
        // byte: il crate allocava ~96 byte per voce dichiarata e l'app terminava).
        let deflate = [0x03, 0x04, 0x01, 0x08];
        assert_eq!(refused(seven_z(&[0u8; 64], &encoded(&deflate, 64, 400_000_000))).0, ErrorCode::ArchiveTooDemanding);
        // Sotto il tetto un metodo che il controllo non decodifica si rifiuta: 7-Zip comprime l'intestazione con LZMA.
        assert_eq!(refused(seven_z(&[0u8; 64], &encoded(&deflate, 64, 1000))), (ErrorCode::Unsupported, None));
        // Intestazione cifrata: senza password non si legge.
        let aes = [0x04, 0x06, 0xF1, 0x07, 0x01];
        assert_eq!(refused(seven_z(&[0u8; 64], &encoded(&aes, 64, 1000))), (ErrorCode::Unsupported, Some("encrypted".into())));
    }

    #[test]
    fn header_counts_above_the_entry_cap_are_refused_before_the_crate() {
        let many = 1_000_001;
        let cases: [Vec<u8>; 4] = [
            // Voci (FilesInfo).
            [&[0x01, 0x05][..], &num(many)].concat(),
            // Folder (UnpackInfo).
            [&[0x01, 0x04, 0x07, 0x0B][..], &num(many)].concat(),
            // Flussi compressi (PackInfo).
            [&[0x01, 0x04, 0x06][..], &num(0), &num(many)].concat(),
            // Sottoflussi (SubStreamsInfo) di un folder con un coder senza metodo.
            [&[0x01, 0x04, 0x07, 0x0B][..], &num(1), &[0x00], &num(1), &[0x00, 0x0C], &num(5), &[0x00, 0x08, 0x0D], &num(many)].concat(),
        ];
        let too_many = (ErrorCode::Unsupported, Some("too_many_entries".to_owned()));
        for (i, mut h) in cases.into_iter().enumerate() {
            h.extend([0u8; 16]);
            assert_eq!(refused(seven_z(&[], &h)), too_many, "caso {i}");
        }
        // L'intestazione delle voci, lunga quanto basta al limite del crate e codificata con Copy: il crate allocava le
        // voci prima di accorgersi dell'errore.
        let mut plain = [&[0x01, 0x05][..], &num(many)].concat();
        plain.resize(many as usize + 64, 0);
        let header = encoded(&[0x01, 0x00], plain.len() as u64, plain.len() as u64);
        assert_eq!(refused(seven_z(&plain, &header)), too_many);
    }

    #[test]
    fn normal_7z_passes() {
        use sevenz_rust2::{ArchiveEntry, ArchiveWriter, SourceReader};
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("solid.7z");
        let mut w = ArchiveWriter::create(&p).unwrap();
        // Con molti nomi lunghi il writer comprime l'intestazione (con poche voci la scrive in chiaro): il controllo
        // analizza un'intestazione codificata vera, con la CRC in PackInfo.
        let mut files = vec![ArchiveEntry::new_file("a.txt"), ArchiveEntry::new_file("d/b.txt")];
        let mut data = vec![SourceReader::new(&b"abc"[..]), SourceReader::new(&b"de"[..])];
        for i in 0..40 {
            files.push(ArchiveEntry::new_file(&format!("d/un-nome-piuttosto-lungo-e-ripetuto-{i:02}.txt")));
            data.push(SourceReader::new(&b"x"[..]));
        }
        w.push_archive_entries(files, data).unwrap();
        w.finish().unwrap();
        let bytes = std::fs::read(&p).unwrap();
        let next = 32 + u64::from_le_bytes(bytes[12..20].try_into().unwrap()) as usize;
        assert_eq!(bytes[next], 0x17);
        let (ix, archive) = build(&p).unwrap();
        assert!(ix.solid);
        assert!(check_blocks(&archive).is_ok());
        let a = ix.get("/a.txt").unwrap();
        assert_eq!((a.size, a.at), (3, EntryAt::SevenZ { file: 0, block: Some(0) }));
        assert_eq!(ix.get("/d/b.txt").unwrap().at, EntryAt::SevenZ { file: 1, block: Some(0) });
        assert!(ix.stat("/d").unwrap().is_dir);
    }
}

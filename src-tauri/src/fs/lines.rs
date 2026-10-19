//! Ricerca degli inizi riga a blocchi, per il visualizzatore a righe lunghe intere: l'LF successivo e quello
//! precedente a un offset, senza caricare mai una riga in memoria. Annullabile con il flag del registro.

use super::content::open_shared;
use crate::error::{AppError, ErrorCode};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// Byte letti per volta: annullamento e avanzamento si controllano a ogni blocco.
const BLOCK: usize = 1 << 20;

/// `unit` byte sono un LF: `0A` (unit 1), `0A 00` (UTF-16LE), `00 0A` (UTF-16BE).
fn is_lf(unit: &[u8], big_endian: bool) -> bool {
    match unit {
        [b] => *b == 0x0A,
        [a, b] => (*a, *b) == if big_endian { (0, 0x0A) } else { (0x0A, 0) },
        _ => false,
    }
}

/// Legge fino a `len` byte da `offset` in `buf` (svuotato prima); più corto solo a fine file.
fn read_block(f: &mut File, offset: u64, len: usize, buf: &mut Vec<u8>, path: &Path) -> Result<(), AppError> {
    let err = |e: std::io::Error| AppError::from_io(&e, path);
    buf.clear();
    f.seek(SeekFrom::Start(offset)).map_err(err)?;
    f.take(len as u64).read_to_end(buf).map_err(err)?;
    Ok(())
}

fn check(path: &Path, unit: u8, cancel: &AtomicBool) -> Result<usize, AppError> {
    if unit != 1 && unit != 2 {
        return Err(AppError::new(ErrorCode::InvalidRequest, Some(path)));
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(AppError::new(ErrorCode::Cancelled, Some(path)));
    }
    Ok(unit as usize)
}

/// Byte dopo il primo LF in posizione `p >= from` (multiplo di `unit`); senza LF, la dimensione del file.
/// `progress(p)` riceve l'offset fino a cui si è cercato senza trovarlo.
pub fn next_line_start(
    path: &Path,
    from: u64,
    unit: u8,
    big_endian: bool,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> Result<u64, AppError> {
    next_line_start_in(path, from, unit, big_endian, cancel, progress, BLOCK)
}

/// Inizio della riga che contiene `at`: `p + unit` per l'ultimo LF con `p + unit <= at`; senza LF, 0.
/// `progress(p)` riceve l'offset più basso già esaminato.
pub fn line_start(
    path: &Path,
    at: u64,
    unit: u8,
    big_endian: bool,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> Result<u64, AppError> {
    line_start_in(path, at, unit, big_endian, cancel, progress, BLOCK)
}

// `block` è sempre multiplo di `unit`: gli offset dei blocchi restano allineati e un LF non li attraversa mai.
fn next_line_start_in(
    path: &Path,
    from: u64,
    unit: u8,
    big_endian: bool,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
    block: usize,
) -> Result<u64, AppError> {
    let u = check(path, unit, cancel)?;
    let mut f = open_shared(path).map_err(|e| AppError::from_io(&e, path))?;
    let size = f.metadata().map_err(|e| AppError::from_io(&e, path))?.len();
    let mut pos = from.next_multiple_of(u as u64);
    let mut buf = Vec::with_capacity(block);
    loop {
        check(path, unit, cancel)?;
        read_block(&mut f, pos, block, &mut buf, path)?;
        if let Some(i) = buf.chunks_exact(u).position(|c| is_lf(c, big_endian)) {
            return Ok(pos + ((i + 1) * u) as u64);
        }
        if buf.is_empty() {
            return Ok(pos.min(size)); // `from` oltre la fine
        }
        pos += buf.len() as u64;
        progress(pos);
        if buf.len() < block {
            return Ok(pos);
        }
    }
}

fn line_start_in(
    path: &Path,
    at: u64,
    unit: u8,
    big_endian: bool,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
    block: usize,
) -> Result<u64, AppError> {
    let u = check(path, unit, cancel)?;
    let mut f = open_shared(path).map_err(|e| AppError::from_io(&e, path))?;
    let mut buf = Vec::with_capacity(block);
    // Un LF a `p` conta se `p + unit <= at`, cioè `p < hi` con `hi` = `at` allineato verso il basso.
    let mut hi = at - at % u as u64;
    while hi > 0 {
        check(path, unit, cancel)?;
        let lo = hi.saturating_sub(block as u64);
        read_block(&mut f, lo, (hi - lo) as usize, &mut buf, path)?;
        if let Some(i) = buf.chunks_exact(u).rposition(|c| is_lf(c, big_endian)) {
            return Ok(lo + ((i + 1) * u) as u64);
        }
        hi = lo;
        progress(lo);
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn file(bytes: &[u8]) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(bytes).unwrap();
        f
    }

    fn next(bytes: &[u8], from: u64, unit: u8, be: bool, block: usize) -> u64 {
        let f = file(bytes);
        next_line_start_in(f.path(), from, unit, be, &AtomicBool::new(false), &mut |_| {}, block).unwrap()
    }

    fn back(bytes: &[u8], at: u64, block: usize) -> u64 {
        let f = file(bytes);
        line_start_in(f.path(), at, 1, false, &AtomicBool::new(false), &mut |_| {}, block).unwrap()
    }

    #[test]
    fn next_finds_lf_across_block_boundary() {
        assert_eq!(next(b"abc\r\ndef", 0, 1, false, 4), 5);
    }

    #[test]
    fn next_without_lf_returns_size() {
        assert_eq!(next(b"abcdef", 0, 1, false, 4), 6);
        assert_eq!(next(b"abcdef", 6, 1, false, 4), 6);
    }

    #[test]
    fn line_start_backward_across_blocks() {
        assert_eq!(back(b"ab\ncdefghij", 10, 4), 3);
    }

    #[test]
    fn line_start_at_a_line_start_returns_it() {
        assert_eq!(back(b"ab\ncd", 3, 4), 3);
        assert_eq!(back(b"ab\ncd", 0, 4), 0);
    }

    #[test]
    fn line_start_without_lf_is_zero() {
        assert_eq!(back(b"abcdef", 5, 4), 0);
    }

    #[test]
    fn utf16_lf_only_on_unit_boundary() {
        // Lo `0A 00` alle posizioni 1-2 è a cavallo di due unità: non è un LF.
        let le = [0x41, 0x0A, 0x00, 0x42, 0x0A, 0x00, 0x43, 0x00];
        assert_eq!(next(&le, 0, 2, false, 4), 6);
        let be = [0x00, 0x41, 0x00, 0x0A, 0x00, 0x42];
        assert_eq!(next(&be, 0, 2, true, 4), 4);
        // All'indietro: stesso criterio.
        let f = file(&le);
        let r = line_start_in(f.path(), 8, 2, false, &AtomicBool::new(false), &mut |_| {}, 4).unwrap();
        assert_eq!(r, 6);
    }

    #[test]
    fn progress_reports_scanned_offsets() {
        let f = file(b"abcdefghij");
        let mut seen = vec![];
        next_line_start_in(f.path(), 0, 1, false, &AtomicBool::new(false), &mut |p| seen.push(p), 4).unwrap();
        assert_eq!(seen, [4, 8, 10]);
        let mut seen = vec![];
        line_start_in(f.path(), 10, 1, false, &AtomicBool::new(false), &mut |p| seen.push(p), 4).unwrap();
        assert_eq!(seen, [6, 2, 0]);
    }

    #[test]
    fn cancel_returns_cancelled() {
        let f = file(b"abcdef");
        let cancel = AtomicBool::new(true);
        let e = next_line_start(f.path(), 0, 1, false, &cancel, &mut |_| {}).unwrap_err();
        assert_eq!(e.code, ErrorCode::Cancelled);
        let e = line_start(f.path(), 5, 1, false, &cancel, &mut |_| {}).unwrap_err();
        assert_eq!(e.code, ErrorCode::Cancelled);
    }
}

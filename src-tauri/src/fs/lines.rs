//! Ricerca degli inizi riga a blocchi, per il visualizzatore a righe lunghe intere: l'LF successivo e quello
//! precedente a un offset, senza caricare mai una riga in memoria. Annullabile con il flag del registro.

use super::source::BlockSource;
use crate::error::{AppError, ErrorCode};
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

fn check(unit: u8, cancel: &AtomicBool) -> Result<usize, AppError> {
    if unit != 1 && unit != 2 {
        return Err(AppError::new(ErrorCode::InvalidRequest, None));
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(AppError::new(ErrorCode::Cancelled, None));
    }
    Ok(unit as usize)
}

/// Byte dopo il primo LF in posizione `p >= from` (multiplo di `unit`); senza LF, la dimensione del file.
/// `progress(p)` riceve l'offset fino a cui si è cercato senza trovarlo.
pub fn next_line_start(
    src: &mut dyn BlockSource,
    from: u64,
    unit: u8,
    big_endian: bool,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> Result<u64, AppError> {
    next_line_start_in(src, from, unit, big_endian, cancel, progress, BLOCK)
}

/// Inizio della riga che contiene `at`: `p + unit` per l'ultimo LF con `p + unit <= at`; senza LF, 0.
/// `progress(p)` riceve l'offset più basso già esaminato.
pub fn line_start(
    src: &mut dyn BlockSource,
    at: u64,
    unit: u8,
    big_endian: bool,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> Result<u64, AppError> {
    line_start_in(src, at, unit, big_endian, cancel, progress, BLOCK)
}

// `block` è sempre multiplo di `unit`: gli offset dei blocchi restano allineati e un LF non li attraversa mai.
fn next_line_start_in(
    src: &mut dyn BlockSource,
    from: u64,
    unit: u8,
    big_endian: bool,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
    block: usize,
) -> Result<u64, AppError> {
    let u = check(unit, cancel)?;
    let size = src.len();
    let mut pos = from.next_multiple_of(u as u64);
    loop {
        check(unit, cancel)?;
        let buf = src.read_at(pos, block as u32)?;
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
    src: &mut dyn BlockSource,
    at: u64,
    unit: u8,
    big_endian: bool,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
    block: usize,
) -> Result<u64, AppError> {
    let u = check(unit, cancel)?;
    // Un LF a `p` conta se `p + unit <= at`, cioè `p < hi` con `hi` = `at` allineato verso il basso.
    let mut hi = at - at % u as u64;
    while hi > 0 {
        check(unit, cancel)?;
        let lo = hi.saturating_sub(block as u64);
        let buf = src.read_at(lo, (hi - lo) as u32)?;
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
    use crate::fs::source::LocalFile;
    use std::io::Write;

    fn file(bytes: &[u8]) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(bytes).unwrap();
        f
    }

    fn next(bytes: &[u8], from: u64, unit: u8, be: bool, block: usize) -> u64 {
        let f = file(bytes);
        next_line_start_in(&mut LocalFile::open(f.path()).unwrap(), from, unit, be, &AtomicBool::new(false), &mut |_| {}, block).unwrap()
    }

    fn back(bytes: &[u8], at: u64, block: usize) -> u64 {
        let f = file(bytes);
        line_start_in(&mut LocalFile::open(f.path()).unwrap(), at, 1, false, &AtomicBool::new(false), &mut |_| {}, block).unwrap()
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
        let r = line_start_in(&mut LocalFile::open(f.path()).unwrap(), 8, 2, false, &AtomicBool::new(false), &mut |_| {}, 4).unwrap();
        assert_eq!(r, 6);
    }

    #[test]
    fn progress_reports_scanned_offsets() {
        let f = file(b"abcdefghij");
        let mut seen = vec![];
        next_line_start_in(&mut LocalFile::open(f.path()).unwrap(), 0, 1, false, &AtomicBool::new(false), &mut |p| seen.push(p), 4).unwrap();
        assert_eq!(seen, [4, 8, 10]);
        let mut seen = vec![];
        line_start_in(&mut LocalFile::open(f.path()).unwrap(), 10, 1, false, &AtomicBool::new(false), &mut |p| seen.push(p), 4).unwrap();
        assert_eq!(seen, [6, 2, 0]);
    }

    #[test]
    fn cancel_returns_cancelled() {
        let f = file(b"abcdef");
        let cancel = AtomicBool::new(true);
        let e = next_line_start(&mut LocalFile::open(f.path()).unwrap(), 0, 1, false, &cancel, &mut |_| {}).unwrap_err();
        assert_eq!(e.code, ErrorCode::Cancelled);
        let e = line_start(&mut LocalFile::open(f.path()).unwrap(), 5, 1, false, &cancel, &mut |_| {}).unwrap_err();
        assert_eq!(e.code, ErrorCode::Cancelled);
    }
}

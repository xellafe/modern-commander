//! Miniature dei file dalla shell di Windows (le stesse di Explorer: immagini, video, PDF, Office, ...).

use super::{check_shell_name, detail_error, hr_error};
use crate::error::{AppError, ErrorCode};
use std::ffi::c_void;
use std::path::Path;
use tauri::ipc::Response;
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits,
    GetObjectW, HBITMAP, HDC, HGDIOBJ, ReleaseDC,
};
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
};
use windows::Win32::UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_RESIZETOFIT};
use windows::core::HSTRING;

/// Lato massimo richiesto: sotto i 16 px la shell non produce niente di utile, sopra i 1024 si spreca memoria.
const SIZE_RANGE: std::ops::RangeInclusive<u32> = 16..=1024;

/// Miniatura di `path` (forma normale, non `\?\`: la shell non la accetta) come RGBA con il lato maggiore
/// ≤ `size`. Per i file senza miniatura la shell restituisce l'icona del tipo.
///
/// Inizializza COM STA sul thread chiamante e lo rilascia in uscita: va chiamata da un thread di
/// `spawn_blocking`, mai da quello della UI (un thumbnailer di video o di rete può metterci secondi).
pub fn thumbnail(path: &Path, size: u32) -> Result<(u32, u32, Vec<u8>), AppError> {
    let invalid = || AppError::new(ErrorCode::InvalidRequest, Some(path));
    if !SIZE_RANGE.contains(&size) {
        return Err(invalid());
    }
    let p = path.to_str().ok_or_else(invalid)?;
    check_shell_name(p)?;
    unsafe {
        // S_FALSE (già inizializzato) va comunque bilanciato; con un errore (modalità diversa) no.
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).is_ok();
        let res = render(p, path, size);
        if com {
            CoUninitialize();
        }
        res
    }
}

/// HBITMAP della shell: si libera su ogni percorso, errori compresi.
struct Bitmap(HBITMAP);

impl Drop for Bitmap {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(HGDIOBJ(self.0.0));
        }
    }
}

struct ScreenDc(HDC);

impl Drop for ScreenDc {
    fn drop(&mut self) {
        unsafe {
            ReleaseDC(None, self.0);
        }
    }
}

fn render(p: &str, path: &Path, size: u32) -> Result<(u32, u32, Vec<u8>), AppError> {
    let err = |e: windows::core::Error| hr_error(&e, Some(path));
    unsafe {
        let factory: IShellItemImageFactory =
            SHCreateItemFromParsingName(&HSTRING::from(p), None).map_err(err)?;
        let side = size as i32;
        // RESIZETOFIT: miniatura (o icona se non c'è) ridimensionata a stare in `size` × `size`.
        let bmp = Bitmap(factory.GetImage(SIZE { cx: side, cy: side }, SIIGBF_RESIZETOFIT).map_err(err)?);

        let mut obj = BITMAP::default();
        let got = GetObjectW(HGDIOBJ(bmp.0.0), size_of::<BITMAP>() as i32, Some(&mut obj as *mut _ as *mut c_void));
        if got == 0 || obj.bmWidth <= 0 || obj.bmHeight <= 0 {
            return Err(detail_error("GetObject"));
        }
        let (w, h) = (obj.bmWidth, obj.bmHeight);
        // Altezza negativa = righe dall'alto verso il basso, come le vuole `ImageData`.
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut px = vec![0u8; w as usize * h as usize * 4];
        let dc = ScreenDc(GetDC(None));
        if dc.0.is_invalid() {
            return Err(detail_error("GetDC"));
        }
        let lines = GetDIBits(dc.0, bmp.0, 0, h as u32, Some(px.as_mut_ptr().cast()), &mut info, DIB_RGB_COLORS);
        if lines == 0 {
            return Err(detail_error("GetDIBits"));
        }
        bgra_to_rgba(&mut px);
        Ok((w as u32, h as u32, px))
    }
}

/// Da BGRA (GDI) a RGBA (canvas). Un bitmap senza canale alfa (tutti zeri, come le icone a 24 bit
/// convertite) è opaco: gli alfa passano a 255, altrimenti la miniatura risulterebbe trasparente.
pub(crate) fn bgra_to_rgba(px: &mut [u8]) {
    let opaque_missing = px.chunks_exact(4).all(|p| p[3] == 0);
    for p in px.chunks_exact_mut(4) {
        p.swap(0, 2);
        if opaque_missing {
            p[3] = 255;
        }
    }
}

/// Miniatura come `[larghezza u32 LE][altezza u32 LE][RGBA]`: arriva al frontend come `ArrayBuffer`.
#[tauri::command]
pub async fn get_thumbnail(path: String, size: u32) -> Result<Response, AppError> {
    let path = crate::fs::local_only(&path)?;
    let (w, h, rgba) = crate::fs::blocking(move || thumbnail(&path, size)).await??;
    let mut out = Vec::with_capacity(8 + rgba.len());
    out.extend_from_slice(&w.to_le_bytes());
    out.extend_from_slice(&h.to_le_bytes());
    out.extend_from_slice(&rgba);
    Ok(Response::new(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;

    #[test]
    fn swaps_channels_and_fixes_missing_alpha() {
        let mut a = vec![1, 2, 3, 0, 4, 5, 6, 0];
        bgra_to_rgba(&mut a);
        assert_eq!(a, [3, 2, 1, 255, 6, 5, 4, 255]);
        let mut b = vec![1, 2, 3, 128];
        bgra_to_rgba(&mut b);
        assert_eq!(b, [3, 2, 1, 128]);
    }

    /// BMP 24 bit 8×8 scritto a mano: intestazione di file (14 byte) + `BITMAPINFOHEADER` (40) + righe
    /// da 24 byte (già multiple di 4, nessun padding). Lo converte il thumbnailer vero della shell.
    fn bmp_8x8() -> Vec<u8> {
        let (w, h, pixels) = (8u32, 8u32, 8u32 * 8 * 3);
        let mut b = Vec::new();
        b.extend_from_slice(b"BM");
        b.extend_from_slice(&(54 + pixels).to_le_bytes()); // dimensione del file
        b.extend_from_slice(&[0; 4]);
        b.extend_from_slice(&54u32.to_le_bytes()); // offset dei pixel
        b.extend_from_slice(&40u32.to_le_bytes()); // dimensione dell'intestazione
        b.extend_from_slice(&w.to_le_bytes());
        b.extend_from_slice(&h.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes()); // piani
        b.extend_from_slice(&24u16.to_le_bytes()); // bit per pixel
        b.extend_from_slice(&[0; 24]); // BI_RGB, dimensione immagine, risoluzioni, palette
        b.extend((0..w * h).flat_map(|i| [(i * 4) as u8, 200, 255 - (i * 4) as u8]));
        b
    }

    #[test]
    fn thumbnail_of_a_bitmap() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("pic.bmp");
        std::fs::write(&p, bmp_8x8()).unwrap();
        let (w, h, data) = thumbnail(&p, 32).unwrap();
        assert!((1..=32).contains(&w) && (1..=32).contains(&h), "{w}x{h}");
        assert_eq!(data.len(), (w * h * 4) as usize);
        assert!(data.chunks_exact(4).any(|p| p[..3] != [0, 0, 0]), "miniatura tutta nera"); // la shell ha davvero disegnato
    }

    #[test]
    fn thumbnail_errors() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope.png");
        let e = thumbnail(&missing, 96).err().unwrap();
        assert_eq!((e.code, e.path.as_deref()), (ErrorCode::NotFound, missing.to_str()));
        let p = dir.path().join("pic.bmp");
        std::fs::write(&p, bmp_8x8()).unwrap();
        for size in [4, 15, 1025] {
            assert_eq!(thumbnail(&p, size).err().unwrap().code, ErrorCode::InvalidRequest, "{size}");
        }
    }
}

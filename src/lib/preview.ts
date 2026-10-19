// Tipo di anteprima di un file e riconoscimento dei file binari.
import { fileKind, splitName } from "./format";

export type PreviewKind = "image" | "pdf" | "video" | "audio" | "markdown" | "archive";

// Solo ciò che la WebView mostra o riproduce da sola (niente tif/heic, mkv/avi, wma).
const KINDS: Record<string, PreviewKind> = {};
const add = (kind: PreviewKind, exts: string) => exts.split(" ").forEach((e) => (KINDS[e] = kind));
add("image", "png jpg jpeg gif webp bmp ico svg avif");
add("video", "mp4 m4v webm mov ogv");
add("audio", "mp3 m4a aac wav ogg oga opus flac");
add("markdown", "md markdown");
add("archive", "zip 7z tar tgz");

export function previewKind(name: string): PreviewKind | null {
  const lower = name.toLowerCase();
  if (lower.endsWith(".tar.gz")) return "archive"; // `x.gz` da solo non è un archivio
  if (fileKind(name, false) === "pdf") return "pdf";
  return KINDS[splitName(lower, false)[1]] ?? null;
}

/** Un byte 0 nel campione e nessun BOM UTF-16 (che di zeri ne ha per natura). */
export function looksBinary(sample: Uint8Array): boolean {
  const utf16Bom = sample.length >= 2 && ((sample[0] === 0xff && sample[1] === 0xfe) || (sample[0] === 0xfe && sample[1] === 0xff));
  return !utf16Bom && sample.indexOf(0) >= 0;
}

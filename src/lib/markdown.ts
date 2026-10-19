// Markdown dei file → HTML. Il contenuto non è fidato: niente HTML grezzo (`html: false`), e markdown-it
// scarta da solo i link `javascript:`, `vbscript:`, `file:` e `data:`. Le immagini sono disattivate: con la CSP
// nulla della WebView un `![](http://…)` farebbe partire una richiesta di rete a ogni anteprima.
import type { MarkdownIt } from "markdown-it";

let parser: Promise<MarkdownIt> | undefined;

/** HTML del Markdown. markdown-it si importa al primo uso (chunk separato dal principale). */
export async function renderMarkdown(text: string): Promise<string> {
  parser ??= import("markdown-it").then(({ default: Ctor }) => new Ctor({ html: false }).disable("image"));
  return (await parser).render(text);
}

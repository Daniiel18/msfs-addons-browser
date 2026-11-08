/**
 * (07/11) Etapa F — Markdown de los manuales de addons, sin dependencias.
 * Convierte el texto en bloques; la vista los pinta como elementos de React y
 * nunca inserta HTML del archivo. Lo que no reconoce queda como texto.
 */

export type MdInline =
  | { t: "text" | "bold" | "em" | "code"; v: string }
  | { t: "link"; v: string; href: string };

export type MdBlock =
  | { t: "h"; level: 1 | 2 | 3; text: MdInline[] }
  | { t: "p"; text: MdInline[] }
  | { t: "ul" | "ol"; items: MdInline[][] }
  | { t: "quote"; text: MdInline[] }
  | { t: "code"; v: string }
  | { t: "hr" };

const INLINE =
  /(`[^`\n]+`)|(\*\*[^*\n]+\*\*|__[^_\n]+__)|(\*[^*\s][^*\n]*\*|(?<![A-Za-z0-9])_[^_\s][^_\n]*_(?![A-Za-z0-9]))|!\[([^\]]*)\]\([^)\n]*\)|\[([^\]\n]+)\]\(([^)\s]+)[^)\n]*\)|(https?:\/\/[^\s<>)\]]+)/g;

export function parseInline(line: string): MdInline[] {
  const out: MdInline[] = [];
  const push = (part: MdInline) => {
    const last = out[out.length - 1];
    if (part.t === "text" && last?.t === "text") last.v += part.v;
    else if (part.v) out.push(part);
  };
  let at = 0;
  for (const m of line.matchAll(INLINE)) {
    const index = m.index ?? 0;
    push({ t: "text", v: line.slice(at, index) });
    at = index + m[0].length;
    if (m[1]) push({ t: "code", v: m[1].slice(1, -1) });
    else if (m[2]) push({ t: "bold", v: m[2].slice(2, -2) });
    else if (m[3]) push({ t: "em", v: m[3].slice(1, -1) });
    else if (m[4] !== undefined) push({ t: "text", v: m[4] });
    else if (m[5]) {
      // Solo direcciones web: nada de `file:` ni `javascript:` desde un manual.
      if (/^https?:\/\//i.test(m[6])) push({ t: "link", v: m[5], href: m[6] });
      else push({ t: "text", v: m[5] });
    } else if (m[7]) push({ t: "link", v: m[7], href: m[7] });
  }
  push({ t: "text", v: line.slice(at) });
  return out;
}

export function parseMarkdown(source: string): MdBlock[] {
  const lines = source.replace(/\r\n?/g, "\n").split("\n");
  const blocks: MdBlock[] = [];
  let para: string[] = [];
  const flush = () => {
    if (para.length === 0) return;
    blocks.push({ t: "p", text: parseInline(para.join(" ")) });
    para = [];
  };
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const trimmed = line.trim();
    if (trimmed === "") {
      flush();
      continue;
    }
    if (/^(```|~~~)/.test(trimmed)) {
      flush();
      const fence = trimmed.slice(0, 3);
      const code: string[] = [];
      for (i++; i < lines.length && !lines[i].trim().startsWith(fence); i++) code.push(lines[i]);
      blocks.push({ t: "code", v: code.join("\n") });
      continue;
    }
    const heading = /^(#{1,6})\s+(.*?)\s*#*$/.exec(trimmed);
    if (heading) {
      flush();
      const level = Math.min(heading[1].length, 3) as 1 | 2 | 3;
      blocks.push({ t: "h", level, text: parseInline(heading[2]) });
      continue;
    }
    if (/^([-*_])(\s*\1){2,}$/.test(trimmed)) {
      flush();
      blocks.push({ t: "hr" });
      continue;
    }
    // Las tablas no se dibujan: en monoespaciado se siguen leyendo.
    if (trimmed.startsWith("|")) {
      flush();
      const table: string[] = [];
      for (; i < lines.length && lines[i].trim().startsWith("|"); i++) table.push(lines[i].trim());
      i--;
      blocks.push({ t: "code", v: table.join("\n") });
      continue;
    }
    if (trimmed.startsWith(">")) {
      flush();
      const quote: string[] = [];
      for (; i < lines.length && lines[i].trim().startsWith(">"); i++) quote.push(lines[i].trim().replace(/^>\s?/, ""));
      i--;
      blocks.push({ t: "quote", text: parseInline(quote.join(" ")) });
      continue;
    }
    const item = /^\s*(?:([-*+])|(\d+)[.)])\s+(.*)$/.exec(line);
    if (item) {
      flush();
      const ordered = !!item[2];
      const pattern = ordered ? /^\s*\d+[.)]\s+(.*)$/ : /^\s*[-*+]\s+(.*)$/;
      const items: MdInline[][] = [];
      for (; i < lines.length; i++) {
        const m = pattern.exec(lines[i]);
        if (!m) break;
        items.push(parseInline(m[1]));
      }
      i--;
      blocks.push({ t: ordered ? "ol" : "ul", items });
      continue;
    }
    para.push(trimmed);
  }
  flush();
  return blocks;
}

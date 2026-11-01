// The markdown of lessons, questions, instructions and feedback, parsed into a small tree that
// renders as React elements, never as HTML: headings, paragraphs, lists (nested), quotes, code
// blocks, rules, bold, italics, code, links to safe addresses and pictures stored in this
// workspace. Anything else is shown as the text it is, so an author's pasted markup cannot run.

export type Inline =
  | { t: "text"; v: string }
  | { t: "b" | "i"; c: Inline[] }
  | { t: "code"; v: string }
  | { t: "a"; href: string; c: Inline[] }
  | { t: "img"; src: string; alt: string };

export type ListBlock = { t: "list"; ordered: boolean; start: number; items: Item[] };

export interface Item {
  c: Inline[];
  /** A list nested under this item. */
  sub?: ListBlock;
}

export type Block =
  | { t: "h"; level: 1 | 2 | 3 | 4; c: Inline[] }
  | { t: "p"; c: Inline[] }
  | { t: "quote"; c: Inline[] }
  | ListBlock
  | { t: "code"; lang: string; v: string }
  | { t: "hr" };

/** http(s) and mailto addresses, and paths of this site; anything else (javascript:, data:) is refused. */
export function safeHref(href: string): string | null {
  const h = href.trim();
  if (/^https?:\/\/[^\s]+$/i.test(h) || /^mailto:[^\s]+$/i.test(h)) return h;
  if (h.startsWith("/") && !h.startsWith("//") && !/\s/.test(h)) return h;
  return null;
}

/** Pictures come from the workspace's own file store only: the page may load nothing else, and a
 *  reader's address should not go to whatever host an author pasted. */
export function safeImage(src: string): string | null {
  const s = src.trim();
  return /^\/files\/(public\/)?[0-9a-zA-Z-]+$/.test(s) ? s : null;
}

const TOKEN =
  /(`+)([\s\S]+?)\1|!\[([^\]]*)\]\(([^)\s]+)\)|\[([^\]]+)\]\(([^)\s]+)\)|\*\*([\s\S]+?)\*\*|__([\s\S]+?)__|\*([^*\s](?:[^*]*[^*\s])?)\*|(?<![\p{L}\p{N}_])_([^_\s](?:[^_]*[^_\s])?)_(?![\p{L}\p{N}_])/gu;

export function parseInline(s: string): Inline[] {
  const out: Inline[] = [];
  const text = (v: string) => {
    if (!v) return;
    const last = out[out.length - 1];
    if (last?.t === "text") last.v += v;
    else out.push({ t: "text", v });
  };
  let at = 0;
  for (const m of s.matchAll(TOKEN)) {
    text(s.slice(at, m.index));
    at = m.index + m[0].length;
    if (m[1]) out.push({ t: "code", v: m[2] });
    else if (m[4] !== undefined) {
      const src = safeImage(m[4]);
      const href = safeHref(m[4]);
      if (src) out.push({ t: "img", src, alt: m[3] });
      // A picture kept elsewhere is a link to it, named by its description.
      else if (href) out.push({ t: "a", href, c: [{ t: "text", v: m[3] || href }] });
      else text(m[3]);
    } else if (m[6] !== undefined) {
      const href = safeHref(m[6]);
      if (href) out.push({ t: "a", href, c: parseInline(m[5]) });
      else text(m[5]);
    } else if (m[7] ?? m[8]) out.push({ t: "b", c: parseInline(m[7] ?? m[8]) });
    else out.push({ t: "i", c: parseInline(m[9] ?? m[10]) });
  }
  text(s.slice(at));
  return out;
}

const BULLET = /^(\s*)(?:([-*+])|(\d{1,9})[.)])\s+(.*)$/;

/** Reads the list that starts at `lines[from]`; returns it and the line after it. */
function readList(lines: string[], from: number): [ListBlock, number] {
  const first = BULLET.exec(lines[from])!;
  const base = first[1].length;
  const ordered = first[3] !== undefined;
  const raws: string[] = [];
  const subs = new Map<number, ListBlock>();
  let i = from;
  while (i < lines.length) {
    if (!lines[i].trim()) {
      // A blank line between items does not end the list; one before anything else does.
      let j = i;
      while (j < lines.length && !lines[j].trim()) j++;
      const next = j < lines.length ? BULLET.exec(lines[j]) : null;
      if (next && (next[1].length >= base + 2 || (next[1].length === base && (next[3] !== undefined) === ordered))) {
        i = j;
        continue;
      }
      break;
    }
    const m = BULLET.exec(lines[i]);
    if (m && m[1].length >= base + 2 && raws.length) {
      // Deeper than this list: the last item's own list.
      const [sub, next] = readList(lines, i);
      subs.set(raws.length - 1, sub);
      i = next;
    } else if (m && m[1].length === base && (m[3] !== undefined) === ordered) {
      raws.push(m[4]);
      i++;
    } else if (!m && /^\s{2,}\S/.test(lines[i]) && raws.length) {
      // An indented line goes on with the last item.
      raws[raws.length - 1] += ` ${lines[i].trim()}`;
      i++;
    } else break;
  }
  const items = raws.map((raw, k): Item => ({ c: parseInline(raw), ...(subs.has(k) ? { sub: subs.get(k) } : {}) }));
  return [{ t: "list", ordered, start: ordered ? Number(first[3]) : 1, items }, i];
}

export function parseMarkdown(src: string): Block[] {
  const lines = src.replace(/\r\n?/g, "\n").split("\n");
  const out: Block[] = [];
  let para: string[] = [];
  let quote: string[] = [];
  const flush = () => {
    if (para.length) out.push({ t: "p", c: parseInline(para.join(" ")) });
    if (quote.length) out.push({ t: "quote", c: parseInline(quote.join(" ")) });
    para = [];
    quote = [];
  };
  for (let i = 0; i < lines.length; ) {
    const raw = lines[i];
    const line = raw.trim();
    let m: RegExpExecArray | null;
    if ((m = /^(`{3,}|~{3,})\s*([\w+#.-]*)\s*$/.exec(line))) {
      flush();
      const fence = m[1];
      const body: string[] = [];
      i++;
      while (i < lines.length && !lines[i].trim().startsWith(fence)) body.push(lines[i++]);
      i++; // the closing fence (an unclosed block runs to the end)
      out.push({ t: "code", lang: m[2], v: body.join("\n") });
    } else if (!line) {
      flush();
      i++;
    } else if ((m = /^(#{1,4})\s+(.+?)(?:\s+#+)?$/.exec(line))) {
      flush();
      out.push({ t: "h", level: m[1].length as 1 | 2 | 3 | 4, c: parseInline(m[2]) });
      i++;
    } else if (/^(?:-{3,}|\*{3,}|_{3,})$/.test(line)) {
      flush();
      out.push({ t: "hr" });
      i++;
    } else if (BULLET.test(raw)) {
      flush();
      const [list, next] = readList(lines, i);
      out.push(list);
      i = next;
    } else if ((m = /^>\s?(.*)$/.exec(line))) {
      if (para.length) flush();
      quote.push(m[1]);
      i++;
    } else {
      if (quote.length) flush();
      para.push(line);
      i++;
    }
  }
  flush();
  return out;
}

/** The words of some inlines, without their marks. */
export function inlineText(c: Inline[]): string {
  return c
    .map((n) => {
      switch (n.t) {
        case "text":
        case "code":
          return n.v;
        case "img":
          return n.alt;
        default:
          return inlineText(n.c);
      }
    })
    .join("");
}

/** Markdown as one line of plain words, for a summary under a title. */
export function plainText(src: string, max = 140): string {
  const text = parseMarkdown(src)
    .map((b) => {
      switch (b.t) {
        case "h":
        case "p":
        case "quote":
          return inlineText(b.c);
        case "list":
          return b.items.map((i) => inlineText(i.c)).join(" ");
        case "code":
          return b.v;
        case "hr":
          return "";
      }
    })
    .filter(Boolean)
    .join(" ")
    .replace(/\s+/g, " ")
    .trim();
  return text.length > max ? `${text.slice(0, max - 1).trimEnd()}…` : text;
}

import { describe, expect, it } from "vitest";
import { type Block, inlineText, parseInline, parseMarkdown, plainText, safeHref, safeImage } from "./markdown";

const words = (b: Block) => inlineText((b as Extract<Block, { t: "p" }>).c);

describe("a lesson's markdown", () => {
  it("reads headings, paragraphs and rules", () => {
    const blocks = parseMarkdown("# Ownership\n\nEvery value has **one** owner.\nIt moves.\n\n---\n\n#### Small");
    expect(blocks.map((b) => b.t)).toEqual(["h", "p", "hr", "h"]);
    expect(blocks[0]).toMatchObject({ level: 1 });
    expect(blocks[3]).toMatchObject({ level: 4 });
    // A line break inside a paragraph is a space.
    expect(words(blocks[1])).toBe("Every value has one owner. It moves.");
  });

  it("keeps code blocks as they were typed, marks and all", () => {
    const blocks = parseMarkdown("Run it:\n\n```rust\nlet x = *y; // **not bold**\n\n    indented\n```\nDone.");
    expect(blocks.map((b) => b.t)).toEqual(["p", "code", "p"]);
    expect(blocks[1]).toEqual({ t: "code", lang: "rust", v: "let x = *y; // **not bold**\n\n    indented" });
  });

  it("lets an unclosed code block run to the end", () => {
    const blocks = parseMarkdown("```\nfn main() {}");
    expect(blocks).toEqual([{ t: "code", lang: "", v: "fn main() {}" }]);
  });

  it("reads lists, ordered and not, with lists inside them", () => {
    const [list] = parseMarkdown("- one\n- two\n  - two a\n  - two b\n- three") as [Extract<ReturnType<typeof parseMarkdown>[number], { t: "list" }>];
    expect(list.ordered).toBe(false);
    expect(list.items.map((i) => inlineText(i.c))).toEqual(["one", "two", "three"]);
    expect(list.items[1].sub?.items.map((i) => inlineText(i.c))).toEqual(["two a", "two b"]);
    const [steps] = parseMarkdown("3. third\n4. fourth") as [Extract<ReturnType<typeof parseMarkdown>[number], { t: "list" }>];
    expect(steps).toMatchObject({ ordered: true, start: 3 });
  });

  it("does not split a list at a blank line between its items", () => {
    const blocks = parseMarkdown("1. First\n\n2. Second\n\nThen a paragraph.");
    expect(blocks.map((b) => b.t)).toEqual(["list", "p"]);
    expect((blocks[0] as { items: unknown[] }).items).toHaveLength(2);
  });

  it("goes on with an item over an indented line", () => {
    const [list] = parseMarkdown("- a long item\n  that goes on") as [Extract<ReturnType<typeof parseMarkdown>[number], { t: "list" }>];
    expect(inlineText(list.items[0].c)).toBe("a long item that goes on");
  });

  it("reads quotes", () => {
    expect(parseMarkdown("> Fear the borrow\n> checker")).toEqual([{ t: "quote", c: [{ t: "text", v: "Fear the borrow checker" }] }]);
  });

  it("reads bold, italics, code and links", () => {
    const c = parseInline("**bold** and *italic* and `code` and [docs](https://doc.rust-lang.org)");
    expect(c.map((n) => n.t)).toEqual(["b", "text", "i", "text", "code", "text", "a"]);
    expect(c[6]).toMatchObject({ t: "a", href: "https://doc.rust-lang.org" });
  });

  it("keeps snake_case words as they are", () => {
    expect(parseInline("use snake_case_names here")).toEqual([{ t: "text", v: "use snake_case_names here" }]);
  });

  it("refuses addresses that run script, and shows their words", () => {
    expect(safeHref("javascript:alert(1)")).toBeNull();
    expect(safeHref("data:text/html,x")).toBeNull();
    expect(safeHref("//evil.test/x")).toBeNull();
    expect(safeHref("/lms/course/rust")).toBe("/lms/course/rust");
    expect(safeHref("mailto:teacher@school.test")).toBe("mailto:teacher@school.test");
    expect(parseInline("[click](javascript:alert)")).toEqual([{ t: "text", v: "click" }]);
  });

  it("shows pictures of this workspace and links to any other", () => {
    expect(safeImage("/files/0d9f2a3c-1b2c-4d5e-8f90-123456789abc")).toBe("/files/0d9f2a3c-1b2c-4d5e-8f90-123456789abc");
    expect(safeImage("/files/public/0d9f2a3c")).toBe("/files/public/0d9f2a3c");
    expect(safeImage("https://tracker.test/pixel.png")).toBeNull();
    expect(safeImage("/files/../etc/passwd")).toBeNull();
    expect(parseInline("![the borrow checker](/files/abc-123)")).toEqual([{ t: "img", src: "/files/abc-123", alt: "the borrow checker" }]);
    expect(parseInline("![diagram](https://cdn.test/d.png)")).toEqual([{ t: "a", href: "https://cdn.test/d.png", c: [{ t: "text", v: "diagram" }] }]);
  });

  it("shows markup it does not know as text", () => {
    const [p] = parseMarkdown("<script>alert(1)</script> <b>x</b>");
    expect(p).toEqual({ t: "p", c: [{ t: "text", v: "<script>alert(1)</script> <b>x</b>" }] });
  });

  it("makes a one-line summary", () => {
    expect(plainText("# Title\n\nSome **bold** text with a [link](https://x.test).\n\n- a\n- b")).toBe("Title Some bold text with a link. a b");
    expect(plainText("x".repeat(300), 20)).toHaveLength(20);
  });
});

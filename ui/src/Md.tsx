// Markdown on screen (./markdown.ts): React elements from a tree, so nothing an author wrote can
// become markup. Reading text gets a comfortable measure and its own rhythm (styles.css, .lms-md).

import { Fragment, useMemo } from "react";
import { type Block, type Inline, type Item, type ListBlock, parseMarkdown } from "./markdown";

function Inlines({ nodes }: { nodes: Inline[] }) {
  return (
    <>
      {nodes.map((n, i) => {
        switch (n.t) {
          case "text":
            return <Fragment key={i}>{n.v}</Fragment>;
          case "code":
            return <code key={i}>{n.v}</code>;
          case "b":
            return (
              <strong key={i}>
                <Inlines nodes={n.c} />
              </strong>
            );
          case "i":
            return (
              <em key={i}>
                <Inlines nodes={n.c} />
              </em>
            );
          case "img":
            return <img key={i} src={n.src} alt={n.alt} loading="lazy" />;
          case "a": {
            const outside = !n.href.startsWith("/");
            return (
              <a key={i} href={n.href} {...(outside ? { target: "_blank", rel: "noopener noreferrer" } : {})}>
                <Inlines nodes={n.c} />
              </a>
            );
          }
        }
      })}
    </>
  );
}

function ListView({ list }: { list: ListBlock }) {
  const items = list.items.map((it: Item, i) => (
    <li key={i}>
      <Inlines nodes={it.c} />
      {it.sub && <ListView list={it.sub} />}
    </li>
  ));
  return list.ordered ? <ol start={list.start === 1 ? undefined : list.start}>{items}</ol> : <ul>{items}</ul>;
}

function BlockView({ block }: { block: Block }) {
  switch (block.t) {
    case "h": {
      // The lesson's title is the page's heading: the text's own start one level down.
      const H = (["h2", "h3", "h4", "h5"] as const)[block.level - 1];
      return (
        <H>
          <Inlines nodes={block.c} />
        </H>
      );
    }
    case "p":
      return (
        <p>
          <Inlines nodes={block.c} />
        </p>
      );
    case "quote":
      return (
        <blockquote>
          <Inlines nodes={block.c} />
        </blockquote>
      );
    case "list":
      return <ListView list={block} />;
    case "code":
      return (
        <pre tabIndex={0} aria-label={block.lang ? `${block.lang} code` : "Code"}>
          <code>{block.v}</code>
        </pre>
      );
    case "hr":
      return <hr />;
  }
}

export function Md({ source, className }: { source: string | null | undefined; className?: string }) {
  const blocks = useMemo(() => parseMarkdown(source ?? ""), [source]);
  if (!blocks.length) return null;
  return (
    <div className={`lms-md ${className ?? ""}`}>
      {blocks.map((b, i) => (
        <BlockView key={i} block={b} />
      ))}
    </div>
  );
}

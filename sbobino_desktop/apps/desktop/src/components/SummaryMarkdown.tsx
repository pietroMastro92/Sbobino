import React from "react";

type SummaryBlock =
  | { kind: "heading"; level: 2 | 3 | 4; text: string }
  | { kind: "list"; items: string[]; ordered: boolean }
  | { kind: "paragraph"; text: string };

function flushParagraph(lines: string[], blocks: SummaryBlock[]): void {
  const text = lines.join(" ").trim();
  if (text) {
    blocks.push({ kind: "paragraph", text });
  }
  lines.length = 0;
}

export function parseSummaryMarkdown(markdown: string): SummaryBlock[] {
  const blocks: SummaryBlock[] = [];
  const paragraphLines: string[] = [];
  const lines = markdown.replace(/\r\n/g, "\n").split("\n");

  for (let index = 0; index < lines.length; index += 1) {
    const rawLine = lines[index] ?? "";
    const line = rawLine.trim();
    if (!line) {
      flushParagraph(paragraphLines, blocks);
      continue;
    }

    const heading = /^(#{1,6})\s+(.+?)\s*#*$/.exec(line);
    if (heading) {
      flushParagraph(paragraphLines, blocks);
      const level = Math.min(4, Math.max(2, heading[1].length)) as 2 | 3 | 4;
      blocks.push({ kind: "heading", level, text: heading[2].trim() });
      continue;
    }

    const unordered = /^[-*]\s+(.+)$/.exec(line);
    const ordered = /^\d+[.)]\s+(.+)$/.exec(line);
    if (unordered || ordered) {
      flushParagraph(paragraphLines, blocks);
      const orderedList = Boolean(ordered);
      const items = [unordered?.[1] ?? ordered?.[1] ?? ""];
      while (index + 1 < lines.length) {
        const next = (lines[index + 1] ?? "").trim();
        const nextUnordered = /^[-*]\s+(.+)$/.exec(next);
        const nextOrdered = /^\d+[.)]\s+(.+)$/.exec(next);
        if (orderedList ? !nextOrdered : !nextUnordered) {
          break;
        }
        items.push(nextUnordered?.[1] ?? nextOrdered?.[1] ?? "");
        index += 1;
      }
      blocks.push({
        kind: "list",
        items: items.map((item) => item.trim()).filter(Boolean),
        ordered: orderedList,
      });
      continue;
    }

    paragraphLines.push(line);
  }

  flushParagraph(paragraphLines, blocks);
  return blocks;
}

export function SummaryMarkdown({ markdown }: { markdown: string }): JSX.Element {
  const blocks = parseSummaryMarkdown(markdown);

  return (
    <div className="summary-markdown">
      {blocks.map((block, index) => {
        if (block.kind === "heading") {
          const HeadingTag = `h${block.level}` as keyof JSX.IntrinsicElements;
          return <HeadingTag key={index}>{block.text}</HeadingTag>;
        }

        if (block.kind === "list") {
          const ListTag = block.ordered ? "ol" : "ul";
          return (
            <ListTag key={index}>
              {block.items.map((item, itemIndex) => (
                <li key={itemIndex}>{item}</li>
              ))}
            </ListTag>
          );
        }

        return <p key={index}>{block.text}</p>;
      })}
    </div>
  );
}

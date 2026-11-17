// A scanner for Resolve's project XML that records where each element sits in the text. Edits are
// splices into the original text, so everything an edit doesn't touch stays exactly as Resolve wrote it.

export interface XmlElement {
  name: string;
  /** DbId attribute, which every Resolve object has. */
  id: string | null;
  /** Offsets into the text: the whole element, and what's between its tags. */
  start: number;
  end: number;
  innerStart: number;
  innerEnd: number;
  children: XmlElement[];
}

export interface Splice {
  start: number;
  end: number;
  text: string;
}

export function parseXml(text: string): XmlElement {
  const document: XmlElement = { name: "", id: null, start: 0, end: text.length, innerStart: 0, innerEnd: text.length, children: [] };
  const open: XmlElement[] = [document];
  let at = text.indexOf("<");
  while (at !== -1) {
    if (text.startsWith("<?", at)) at = skip(text, at, "?>");
    else if (text.startsWith("<!--", at)) at = skip(text, at, "-->");
    else if (text.startsWith("<![CDATA[", at)) at = skip(text, at, "]]>");
    else if (text.startsWith("<!", at)) at = skip(text, at, ">");
    else if (text[at + 1] === "/") {
      const close = skip(text, at, ">");
      const element = open.pop();
      const name = text.slice(at + 2, close - 1).trim();
      if (!element || element === document || element.name !== name) throw new Error(`Unexpected </${name}> at ${at}.`);
      element.innerEnd = at;
      element.end = close;
      at = close;
    } else {
      const close = tagEnd(text, at);
      const selfClosing = text[close - 2] === "/";
      const tag = text.slice(at + 1, selfClosing ? close - 2 : close - 1);
      const name = /^[^\s/>]+/.exec(tag)?.[0] ?? "";
      const element: XmlElement = {
        name,
        id: /\sDbId="([^"]*)"/.exec(tag)?.[1] ?? null,
        start: at,
        end: close,
        innerStart: close,
        innerEnd: close,
        children: [],
      };
      open.at(-1)!.children.push(element);
      if (!selfClosing) open.push(element);
      at = close;
    }
    at = text.indexOf("<", at);
  }
  if (open.length !== 1) throw new Error(`<${open.at(-1)!.name}> is never closed.`);
  const root = document.children[0];
  if (!root) throw new Error("The XML has no root element.");
  return root;
}

function skip(text: string, at: number, terminator: string): number {
  const end = text.indexOf(terminator, at);
  if (end === -1) throw new Error(`Unterminated markup at ${at}.`);
  return end + terminator.length;
}

/** The index after a start tag's `>`, stepping over quoted attribute values. */
function tagEnd(text: string, at: number): number {
  let quote = "";
  for (let i = at + 1; i < text.length; i++) {
    const char = text[i];
    if (quote) {
      if (char === quote) quote = "";
    } else if (char === '"' || char === "'") quote = char;
    else if (char === ">") return i + 1;
  }
  throw new Error(`Unterminated tag at ${at}.`);
}

export function child(element: XmlElement, name: string): XmlElement | undefined {
  return element.children.find((candidate) => candidate.name === name);
}

/** Every element below `element`, depth first. */
export function* descendants(element: XmlElement): Generator<XmlElement> {
  for (const next of element.children) {
    yield next;
    yield* descendants(next);
  }
}

export function findById(root: XmlElement, id: string): XmlElement | undefined {
  if (root.id === id) return root;
  for (const element of descendants(root)) if (element.id === id) return element;
  return undefined;
}

/** The element's text with entities decoded, or "" when it has children or is empty. */
export function textOf(text: string, element: XmlElement | undefined): string {
  if (!element || element.children.length) return "";
  return unescape(text.slice(element.innerStart, element.innerEnd));
}

/** The text of `element`'s child called `name`. */
export function childText(text: string, element: XmlElement, name: string): string {
  return textOf(text, child(element, name));
}

/** A splice that sets a leaf element's text, keeping the tag. An empty value writes `<Name/>`, like Resolve. */
export function setText(text: string, element: XmlElement, value: string): Splice {
  if (element.children.length) throw new Error(`<${element.name}> isn't a text element.`);
  const selfClosing = element.innerStart === element.end;
  const tag = selfClosing ? text.slice(element.start + 1, element.end - 2).trimEnd() : text.slice(element.start + 1, element.innerStart - 1);
  return {
    start: element.start,
    end: element.end,
    text: value === "" ? `<${tag}/>` : `<${tag}>${escape(value)}</${element.name}>`,
  };
}

/** `text` with the splices applied. They mustn't overlap. */
export function applySplices(text: string, splices: Splice[]): string {
  const ordered = [...splices].sort((a, b) => b.start - a.start);
  let result = text;
  let limit = Infinity;
  for (const splice of ordered) {
    if (splice.end > limit) throw new Error("Two edits touch the same part of the XML.");
    result = result.slice(0, splice.start) + splice.text + result.slice(splice.end);
    limit = splice.start;
  }
  return result;
}

const ENTITIES: Record<string, string> = { amp: "&", lt: "<", gt: ">", quot: '"', apos: "'" };

function unescape(value: string): string {
  return value.replace(/&(#x[0-9a-f]+|#\d+|\w+);/gi, (whole, entity: string) => {
    if (entity[0] === "#") {
      const code = entity[1] === "x" || entity[1] === "X" ? parseInt(entity.slice(2), 16) : parseInt(entity.slice(1), 10);
      return String.fromCodePoint(code);
    }
    return ENTITIES[entity] ?? whole;
  });
}

function escape(value: string): string {
  return value.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

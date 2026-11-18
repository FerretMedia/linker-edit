import { Marked } from "marked";

const escapeHtml = (text: string) =>
  text.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;").replaceAll('"', "&quot;");

// Agent output is untrusted: raw HTML is shown as text, and links open outside the app.
const marked = new Marked({
  gfm: true,
  breaks: true,
  renderer: {
    html: ({ text }) => escapeHtml(text),
    link({ href, tokens }) {
      const safe = /^(https?:|mailto:)/i.test(href) ? href : "#";
      return `<a href="${escapeHtml(safe)}" target="_blank" rel="noreferrer">${this.parser.parseInline(tokens)}</a>`;
    },
    image: ({ text }) => escapeHtml(text),
  },
});

// The chat redraws its rows on every new item, so each reply would be parsed again and again: a 1,000-item thread
// parsed about 230 replies per streamed message. Bounded, since a streaming reply passes through many texts.
const CACHE_SIZE = 1000;
const cache = new Map<string, string>();

export function renderMarkdown(text: string): string {
  let html = cache.get(text);
  if (html === undefined) {
    html = marked.parse(text, { async: false });
    if (cache.size >= CACHE_SIZE) cache.delete(cache.keys().next().value!);
  } else {
    // Most recently used last, so the oldest goes first.
    cache.delete(text);
  }
  cache.set(text, html);
  return html;
}

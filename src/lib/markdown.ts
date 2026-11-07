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

export const renderMarkdown = (text: string) => marked.parse(text, { async: false });

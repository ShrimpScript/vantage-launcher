/**
 * A deliberately small markdown subset for Modrinth project bodies.
 *
 * The body is written by whoever published the mod, so it is untrusted: everything is HTML-
 * escaped *first*, then a fixed set of transforms is applied to the escaped text. Nothing an
 * author writes can introduce a tag.
 *
 * Images are dropped rather than rendered. Bodies are full of huge banners, and an <img> with
 * an author-controlled URL is an outbound request to an arbitrary host from inside the app.
 * The gallery comes from the API's own `gallery` field instead, which is Modrinth-hosted.
 */

const esc = (s: string) =>
  s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

/** Only http(s) links survive, so `javascript:` and friends cannot appear in an href. */
function safeHref(url: string): string | null {
  const u = url.trim();
  return /^https?:\/\//i.test(u) ? u.replace(/"/g, "%22") : null;
}

function inline(s: string): string {
  return s
    // images first, so their URLs are removed before the link rule sees them
    .replace(/!\[[^\]]*\]\([^)]*\)/g, "")
    .replace(/\[([^\]]+)\]\(([^)\s]+)[^)]*\)/g, (_m, text: string, url: string) => {
      const href = safeHref(url);
      return href ? `<a href="${href}" target="_blank" rel="noreferrer noopener">${text}</a>` : text;
    })
    .replace(/`([^`]+)`/g, "<code>$1</code>")
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
    .replace(/(^|[^*])\*([^*]+)\*/g, "$1<em>$2</em>");
}

export function render(md: string): string {
  const lines = esc(md).replace(/\r/g, "").split("\n");
  const out: string[] = [];
  let inList = false;
  let inCode = false;

  const closeList = () => {
    if (inList) { out.push("</ul>"); inList = false; }
  };

  for (const raw of lines) {
    const line = raw.trimEnd();

    if (/^```/.test(line)) {
      closeList();
      out.push(inCode ? "</pre>" : "<pre>");
      inCode = !inCode;
      continue;
    }
    if (inCode) { out.push(line); continue; }

    if (!line.trim()) { closeList(); continue; }

    const h = /^(#{1,4})\s+(.*)$/.exec(line);
    if (h) {
      closeList();
      const level = Math.min(h[1]!.length + 2, 6);
      out.push(`<h${level}>${inline(h[2]!)}</h${level}>`);
      continue;
    }
    if (/^\s*([-*+]|\d+\.)\s+/.test(line)) {
      if (!inList) { out.push("<ul>"); inList = true; }
      out.push(`<li>${inline(line.replace(/^\s*([-*+]|\d+\.)\s+/, ""))}</li>`);
      continue;
    }
    if (/^\s*(---+|===+|\*\*\*+)\s*$/.test(line)) { closeList(); out.push("<hr />"); continue; }

    closeList();
    out.push(`<p>${inline(line)}</p>`);
  }
  closeList();
  if (inCode) out.push("</pre>");
  return out.join("\n");
}

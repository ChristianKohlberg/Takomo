// Shared SPA code — the single source for the markdown renderer both
// /board and /inbox render agent- and human-written text with (takomo-ftix).
//
// This file is NOT fetched by the browser. It is inlined into each page at a
// marker comment when the response is built (src/api/mod.rs), so
// each page stays the one self-contained document CLAUDE.md and api/mod.rs
// promise — no second request, no new route, and the existing `script-src
// 'self'` CSP is untouched.
//
// That marker is the LAST line of each page's script on purpose. Appending
// rather than splicing means every line of page-specific code keeps the line
// number it has in its own file, so a stack trace still points somewhere you
// can go. Inlining mid-file would silently shift ~2000 lines.
//
// It was two byte-identical copies before this, and nothing guarded them: CI's
// "Duplicated files stay in sync" job only diffs the two SKILL.md files. Two
// PRs (#92, #95) edited this renderer and stayed correct only because their
// authors hand-checked byte-identity. The renderer parses attacker-influenced
// ticket bodies, comments and question text, and `mdHref` below is the
// allowlist that refuses javascript:/data:/vbscript:, so a fix landing in one
// copy and not the other is a security bug that shows up on one page only.
//
// Depends on exactly one thing from the host page: `el(tag, cls, text)`, which
// scripts/spa-eslint.config.mjs declares for this file so it lints on its own. It
// touches no state, no locale table and no API. Keep it that way — anything
// needing `L()`/`t()` or `state` belongs in the page, not here.

  var MD_INLINE = /(`[^`]+`|\*\*[^*]+\*\*|\*[^*\n]+\*|_[^_\n]+_|\[[^\]]*\]\()/g;
  var MD_HEADING = /^(#{1,6})\s+(.*)$/;
  var MD_RULE = /^(-{3,}|\*{3,}|_{3,})$/;
  var MD_OL = /^\d+[.)]\s+/;
  var MD_UL = /^[-*+]\s+/;
  var MD_QUOTE = /^>\s?/;
  var MD_FENCE = /^```/;

  // Only http(s) and mailto survive. Anything else (javascript:, data:) renders
  // as its literal markdown, so a hostile link cannot hide behind link text.
  function mdHref(u) { return /^(https?:\/\/|mailto:)/i.test(u) ? u : null; }

  // Scan a link target — everything after `](` — and return {url, title, end},
  // `end` being just past the closing `)`; null when it is not a well-formed
  // target, which makes the caller print the source verbatim. Hand-rolled with a
  // depth counter rather than a regex: balanced parens are not a regular
  // language, and the regexes that approximate them are exactly the ones that
  // backtrack catastrophically on hostile input. This is one left-to-right pass
  // that never rewinds, so its cost is linear in the target's length.
  function mdLinkTarget(s, from) {
    var n = s.length, i = from, depth = 0, url = "", ch;
    while (i < n && (s.charAt(i) === " " || s.charAt(i) === "\t")) i++;
    while (i < n) {
      ch = s.charAt(i);
      if (ch === "\n") return null;                    // a target never spans lines
      if (ch === "\\" && (s.charAt(i + 1) === "(" || s.charAt(i + 1) === ")")) {
        url += s.charAt(i + 1); i += 2; continue;      // \( and \) are literals
      }
      if (ch === "(") { depth++; url += ch; i++; continue; }
      if (ch === ")") {
        if (depth === 0) break;                        // the closing paren
        depth--; url += ch; i++; continue;
      }
      if (ch === " " || ch === "\t") break;            // a title may follow
      url += ch; i++;
    }
    if (depth !== 0) return null;                      // unbalanced, so not a link
    while (i < n && (s.charAt(i) === " " || s.charAt(i) === "\t")) i++;
    var title = "", q = s.charAt(i);
    if (q === '"' || q === "'") {
      i++;
      while (i < n && s.charAt(i) !== q) {
        if (s.charAt(i) === "\n") return null;
        title += s.charAt(i); i++;
      }
      if (i >= n) return null;                         // unterminated title
      i++;
      while (i < n && (s.charAt(i) === " " || s.charAt(i) === "\t")) i++;
    }
    if (s.charAt(i) !== ")") return null;
    return { url: url, title: title, end: i + 1 };
  }

  function mdInline(parent, s) {
    s = String(s == null ? "" : s);
    var last = 0, m;
    MD_INLINE.lastIndex = 0;
    while ((m = MD_INLINE.exec(s)) !== null) {
      if (m.index > last) parent.appendChild(document.createTextNode(s.slice(last, m.index)));
      var tok = m[0], c0 = tok.charAt(0);
      if (c0 === "`") {
        parent.appendChild(el("code", null, tok.slice(1, -1)));
      } else if (tok.indexOf("**") === 0) {
        parent.appendChild(el("b", null, tok.slice(2, -2)));
      } else if (c0 === "*" || c0 === "_") {
        parent.appendChild(el("i", null, tok.slice(1, -1)));
      } else {
        // `tok` is `[text](`, so the target starts just past it. mdHref is still
        // the only thing that decides a scheme is allowed — the wider scan feeds
        // it more URLs, it does not let more of them through.
        var tgt = mdLinkTarget(s, m.index + tok.length);
        var href = tgt ? mdHref(tgt.url) : null;
        // Where this construct's source ends. On a refused scheme we still skip
        // the whole parsed target, so `[x](javascript:alert(1))` prints entire
        // rather than having its tail rescanned for inline tokens.
        var end = tgt ? tgt.end : m.index + tok.length;
        if (href) {
          var a = el("a", null, tok.slice(1, -2));
          a.href = href; a.target = "_blank"; a.rel = "noopener noreferrer";
          if (tgt.title) a.title = tgt.title;          // an attribute value, never parsed as markup
          parent.appendChild(a);
        } else {
          parent.appendChild(document.createTextNode(s.slice(m.index, end)));
        }
        MD_INLINE.lastIndex = end;
      }
      last = MD_INLINE.lastIndex;
    }
    if (last < s.length) parent.appendChild(document.createTextNode(s.slice(last)));
  }

  // `| a | b |` -> ["a","b"], tolerating the optional outer pipes.
  function mdCells(line) {
    return line.trim().replace(/^\|/, "").replace(/\|$/, "").split("|").map(function (c) { return c.trim(); });
  }
  // A table is a pipe row whose NEXT line is a |---|---| separator. Requiring the
  // separator keeps prose that merely contains a pipe from becoming a table.
  function mdIsTable(lines, i) {
    if (lines[i].indexOf("|") === -1 || i + 1 >= lines.length) return false;
    var sep = lines[i + 1].trim();
    return sep.indexOf("|") !== -1 && /^\|?[\s:|-]+$/.test(sep) && sep.indexOf("-") !== -1;
  }
  // Does line `i` open a block? Used both to dispatch and to end a paragraph, so
  // the two can never disagree about where a paragraph stops.
  function mdIsBlock(lines, i) {
    var t = lines[i].trim();
    return t === "" || MD_FENCE.test(t) || MD_HEADING.test(t) || MD_RULE.test(t) ||
      MD_OL.test(t) || MD_UL.test(t) || MD_QUOTE.test(t) || mdIsTable(lines, i);
  }

  function mdNode(s, cls) {
    var wrap = el("div", cls ? "md " + cls : "md");
    var lines = String(s == null ? "" : s).replace(/\r\n?/g, "\n").split("\n");
    var i = 0;
    while (i < lines.length) {
      var t = lines[i].trim();
      if (t === "") { i++; continue; }

      if (MD_FENCE.test(t)) {
        var lang = t.slice(3).trim();
        var buf = [];
        i++;
        while (i < lines.length && !MD_FENCE.test(lines[i].trim())) { buf.push(lines[i]); i++; }
        if (i < lines.length) i++;                       // closing fence
        var pre = el("pre"), code = el("code", null, buf.join("\n"));
        if (lang) code.setAttribute("data-lang", lang);
        pre.appendChild(code); wrap.appendChild(pre);
        continue;
      }

      var h = MD_HEADING.exec(t);
      if (h) {
        // Start at h3: these render inside panels that own h1/h2 already.
        var head = el("h" + Math.min(6, h[1].length + 2));
        mdInline(head, h[2]); wrap.appendChild(head); i++;
        continue;
      }

      if (MD_RULE.test(t)) { wrap.appendChild(el("hr")); i++; continue; }

      if (mdIsTable(lines, i)) {
        var head2 = mdCells(lines[i]);
        i += 2;                                          // header + separator
        var scroll = el("div", "md-table");              // owns the x-overflow
        var table = el("table"), thead = el("thead"), htr = el("tr");
        head2.forEach(function (c) { var th = el("th"); mdInline(th, c); htr.appendChild(th); });
        thead.appendChild(htr); table.appendChild(thead);
        var tbody = el("tbody");
        while (i < lines.length && lines[i].trim() !== "" && lines[i].indexOf("|") !== -1) {
          var tr = el("tr");
          mdCells(lines[i]).forEach(function (c) { var td = el("td"); mdInline(td, c); tr.appendChild(td); });
          tbody.appendChild(tr); i++;
        }
        table.appendChild(tbody); scroll.appendChild(table); wrap.appendChild(scroll);
        continue;
      }

      if (MD_QUOTE.test(t)) {
        var bq = el("blockquote");
        var firstQ = true;
        while (i < lines.length && MD_QUOTE.test(lines[i].trim())) {
          if (!firstQ) bq.appendChild(el("br"));
          mdInline(bq, lines[i].trim().replace(MD_QUOTE, ""));
          firstQ = false; i++;
        }
        wrap.appendChild(bq);
        continue;
      }

      if (MD_OL.test(t) || MD_UL.test(t)) {
        var ordered = MD_OL.test(t);
        var list = el(ordered ? "ol" : "ul");
        var re = ordered ? MD_OL : MD_UL;
        while (i < lines.length && re.test(lines[i].trim())) {
          var li = el("li");
          mdInline(li, lines[i].trim().replace(re, ""));
          list.appendChild(li); i++;
        }
        wrap.appendChild(list);
        continue;
      }

      // Paragraph. A single newline becomes a <br> rather than being reflowed:
      // agents hand-format these, and silently joining their lines changes what
      // they wrote.
      var p = el("p"), first = true;
      while (i < lines.length && !mdIsBlock(lines, i)) {
        if (!first) p.appendChild(el("br"));
        mdInline(p, lines[i].trim());
        first = false; i++;
      }
      wrap.appendChild(p);
    }
    return wrap;
  }

// Fast drop-in replacement for csv-reader 1.0.0.
// Single left-to-right pass. Unquoted fields are taken as one slice
// between delimiters (stray CRs are the rare exception and rebuild the
// field from parts); quoted fields jump to their closing quote with
// indexOf, with a slower part-by-part path only when the field holds
// doubled quotes or CR characters that must be read as LF.
export function run(input) {
  const text = typeof input.text === "string" ? input.text : String(input.text);
  const header = input.header === true;
  const n = text.length;
  const errors = [];
  const rows = [];
  let columns = null;
  let width = -1;
  let pos = text.charCodeAt(0) === 0xFEFF ? 1 : 0;
  let line = 1;

  while (true) {
    while (pos < n) {
      const c = text.charCodeAt(pos);
      if (c === 10) { pos += 1; line += 1; }
      else if (c === 13 && text.charCodeAt(pos + 1) === 10) { pos += 2; line += 1; }
      else break;
    }
    if (pos >= n) break;

    const recordLine = line;
    const recordRow = header && columns === null ? null : rows.length;
    const fields = [];
    let lastQuoted = false;

    for (;;) {
      let value = "";
      let ended = 2; // 0 comma, 1 line break, 2 end
      let needScan = false;
      let junkFrom = -1;

      if (pos < n && text.charCodeAt(pos) === 34) {
        lastQuoted = true;
        const open = pos;
        const openLine = line;
        const firstQ = text.indexOf('"', open + 1);
        let crIdx = -1;
        if (firstQ === -1) {
          errors.push({ code: "unterminated_quote", line: recordLine, row: recordRow });
          value = '"';
          pos = open + 1;
          needScan = true;
        } else if (text.charCodeAt(firstQ + 1) !== 34 && ((crIdx = text.indexOf("\r", open + 1)) === -1 || crIdx >= firstQ)) {
          // Simple quoted field: first quote closes it, no CR inside.
          value = text.slice(open + 1, firstQ);
          let lf = text.indexOf("\n", open + 1);
          while (lf !== -1 && lf < firstQ) { line += 1; lf = text.indexOf("\n", lf + 1); }
          pos = firstQ + 1;
          if (pos >= n) {
            ended = 2;
          } else {
            const c = text.charCodeAt(pos);
            if (c === 44) { pos += 1; ended = 0; }
            else if (c === 10) { pos += 1; line += 1; ended = 1; }
            else if (c === 13 && text.charCodeAt(pos + 1) === 10) { pos += 2; line += 1; ended = 1; }
            else { junkFrom = value.length; needScan = true; }
          }
        } else {
          // General quoted field: doubled quotes and/or CRs inside.
          const parts = [];
          let segStart = open + 1;
          let cur = firstQ;
          let closed = false;
          for (;;) {
            if (cur === -1) break;
            const isDoubled = text.charCodeAt(cur + 1) === 34;
            if (segStart < cur) {
              let seg = text.slice(segStart, cur);
              if (seg.indexOf("\r") !== -1) {
                let cnt = 0;
                for (let k = 0; k < seg.length; k++) {
                  const cc = seg.charCodeAt(k);
                  if (cc === 13) { cnt += 1; if (seg.charCodeAt(k + 1) === 10) k += 1; }
                  else if (cc === 10) cnt += 1;
                }
                line += cnt;
                seg = seg.replace(/\r\n|\r/g, "\n");
              } else {
                let lf = seg.indexOf("\n");
                while (lf !== -1) { line += 1; lf = seg.indexOf("\n", lf + 1); }
              }
              parts.push(seg);
            }
            if (isDoubled) {
              parts.push('"');
              segStart = cur + 2;
              cur = text.indexOf('"', segStart);
              continue;
            }
            pos = cur + 1;
            closed = true;
            break;
          }
          if (!closed) {
            line = openLine;
            errors.push({ code: "unterminated_quote", line: recordLine, row: recordRow });
            value = '"';
            pos = open + 1;
            needScan = true;
          } else {
            value = parts.join("");
            if (pos >= n) {
              ended = 2;
            } else {
              const c = text.charCodeAt(pos);
              if (c === 44) { pos += 1; ended = 0; }
              else if (c === 10) { pos += 1; line += 1; ended = 1; }
              else if (c === 13 && text.charCodeAt(pos + 1) === 10) { pos += 2; line += 1; ended = 1; }
              else { junkFrom = value.length; needScan = true; }
            }
          }
        }
      } else {
        lastQuoted = false;
        needScan = true;
      }

      if (needScan) {
        const start0 = pos;
        let i = pos;
        let segStart = pos;
        let parts = null;
        let how = 2;
        while (i < n) {
          const c = text.charCodeAt(i);
          if (c === 44) { how = 0; break; }
          if (c === 10) { how = 1; break; }
          if (c === 13) {
            if (text.charCodeAt(i + 1) === 10) { how = 1; break; }
            errors.push({ code: "stray_cr", line: recordLine, row: recordRow });
            if (parts === null) parts = [value];
            parts.push(text.slice(segStart, i));
            i += 1;
            segStart = i;
            continue;
          }
          i += 1;
        }
        if (parts === null) {
          value += text.slice(start0, i);
        } else {
          parts.push(text.slice(segStart, i));
          value = parts.join("");
        }
        if (how === 0) { pos = i + 1; ended = 0; }
        else if (how === 1) {
          pos = text.charCodeAt(i) === 13 ? i + 2 : i + 1;
          line += 1;
          ended = 1;
        } else { pos = n; ended = 2; }
        if (junkFrom !== -1 && value.length > junkFrom) {
          errors.push({ code: "quote_junk", line: recordLine, row: recordRow });
        }
      }

      fields.push(value);
      if (ended !== 0) break;
    }

    if (fields.length === 1 && fields[0] === "" && !lastQuoted) continue;

    if (width < 0) width = fields.length;
    if (header && columns === null) { columns = fields; continue; }
    if (fields.length < width) {
      errors.push({ code: "short_row", line: recordLine, row: recordRow, fields: fields.length });
      while (fields.length < width) fields.push("");
    } else if (fields.length > width) {
      errors.push({ code: "long_row", line: recordLine, row: recordRow, extra: fields.splice(width) });
    }
    rows.push(fields);
  }
  return { columns: header ? (columns ?? []) : null, rows, errors };
}

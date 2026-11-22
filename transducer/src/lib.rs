// JSON-to-JSON CSV transducer: reads /input/cases.json bytes and writes
// /output/answers.json bytes, answering each case exactly as
// csv-reader 1.0.0 does, without materialising JS values.
// Case texts are decoded to UTF-8 bytes (lone surrogates from \uXXXX
// escapes are kept as CESU-8 3-byte sequences and re-escaped on output).

use std::slice;

// ---------- fixed output sink ----------
struct FixedOut {
    ptr: *mut u8,
    cap: usize,
    len: usize,
    overflow: bool,
}
impl FixedOut {
    fn b(&mut self, byte: u8) {
        if self.len >= self.cap {
            self.overflow = true;
            return;
        }
        unsafe { *self.ptr.add(self.len) = byte; }
        self.len += 1;
    }
    fn s(&mut self, bytes: &[u8]) {
        if self.len + bytes.len() > self.cap {
            self.overflow = true;
            return;
        }
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), self.ptr.add(self.len), bytes.len());
        }
        self.len += bytes.len();
    }
    fn lit(&mut self, s: &str) {
        self.s(s.as_bytes());
    }
}

// ---------- growable byte sink helpers ----------
fn vs(v: &mut Vec<u8>, bytes: &[u8]) {
    v.extend_from_slice(bytes);
}
fn vlit(v: &mut Vec<u8>, s: &str) {
    v.extend_from_slice(s.as_bytes());
}
fn vuint(v: &mut Vec<u8>, mut x: usize) {
    let mut tmp = [0u8; 20];
    let mut i = tmp.len();
    if x == 0 {
        v.push(b'0');
        return;
    }
    while x > 0 {
        i -= 1;
        tmp[i] = b'0' + (x % 10) as u8;
        x /= 10;
    }
    v.extend_from_slice(&tmp[i..]);
}

fn push_utf8(dst: &mut Vec<u8>, cp: u32) {
    if cp < 0x80 {
        dst.push(cp as u8);
    } else if cp < 0x800 {
        dst.push(0xC0 | (cp >> 6) as u8);
        dst.push(0x80 | (cp & 0x3F) as u8);
    } else if cp < 0x10000 {
        dst.push(0xE0 | (cp >> 12) as u8);
        dst.push(0x80 | ((cp >> 6) & 0x3F) as u8);
        dst.push(0x80 | (cp & 0x3F) as u8);
    } else {
        dst.push(0xF0 | (cp >> 18) as u8);
        dst.push(0x80 | ((cp >> 12) & 0x3F) as u8);
        dst.push(0x80 | ((cp >> 6) & 0x3F) as u8);
        dst.push(0x80 | (cp & 0x3F) as u8);
    }
}

// ---------- JSON string emission from UTF-8 bytes ----------
fn emit_json_str(dst: &mut Vec<u8>, bytes: &[u8]) {
    dst.push(b'"');
    let mut i = 0usize;
    let n = bytes.len();
    while i < n {
        // Fast path: bulk-copy a run of bytes needing no escaping
        // (0xED is excluded: it may start a CESU-8 lone surrogate).
        let rest = &bytes[i..];
        let run = rest
            .iter()
            .position(|&b| b < 0x20 || b == 0x22 || b == 0x5C || b == 0xED)
            .unwrap_or(rest.len());
        if run > 0 {
            dst.extend_from_slice(&rest[..run]);
            i += run;
            continue;
        }
        let b = bytes[i];
        match b {
            0x22 => { vs(dst, b"\\\""); i += 1; }
            0x5C => { vs(dst, b"\\\\"); i += 1; }
            0x0A => { vs(dst, b"\\n"); i += 1; }
            0x0D => { vs(dst, b"\\r"); i += 1; }
            0x09 => { vs(dst, b"\\t"); i += 1; }
            0x08 => { vs(dst, b"\\b"); i += 1; }
            0x0C => { vs(dst, b"\\f"); i += 1; }
            0x00..=0x1F => {
                const HEX: &[u8; 16] = b"0123456789abcdef";
                vs(dst, b"\\u00");
                dst.push(HEX[(b >> 4) as usize]);
                dst.push(HEX[(b & 15) as usize]);
                i += 1;
            }
            0xED => {
                // CESU-8 lone surrogate? (ED A0..BF 80..BF)
                if i + 2 < n && (bytes[i + 1] & 0xC0) == 0x80 && (bytes[i + 2] & 0xC0) == 0x80 {
                    let cp = (((b & 0x0F) as u32) << 12)
                        | (((bytes[i + 1] & 0x3F) as u32) << 6)
                        | ((bytes[i + 2] & 0x3F) as u32);
                    if (0xD800..=0xDFFF).contains(&cp) {
                        const HEX: &[u8; 16] = b"0123456789abcdef";
                        vs(dst, b"\\u");
                        dst.push(HEX[(cp >> 12) as usize]);
                        dst.push(HEX[((cp >> 8) & 15) as usize]);
                        dst.push(HEX[((cp >> 4) & 15) as usize]);
                        dst.push(HEX[(cp & 15) as usize]);
                        i += 3;
                        continue;
                    }
                }
                dst.push(b);
                i += 1;
            }
            _ => { dst.push(b); i += 1; }
        }
    }
    dst.push(b'"');
}

// ---------- JSON input walking ----------
struct Json<'a> {
    b: &'a [u8],
    p: usize,
}
impl<'a> Json<'a> {
    fn ws(&mut self) {
        while self.p < self.b.len() && matches!(self.b[self.p], b' ' | b'\t' | b'\n' | b'\r') {
            self.p += 1;
        }
    }
    fn peek(&mut self) -> u8 {
        self.ws();
        if self.p < self.b.len() {
            self.b[self.p]
        } else {
            0
        }
    }
    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == c {
            self.p += 1;
            true
        } else {
            false
        }
    }
    fn hex4(&self, at: usize) -> Option<u32> {
        if at + 4 > self.b.len() {
            return None;
        }
        let mut v: u32 = 0;
        for k in 0..4 {
            let h = self.b[at + k];
            let d = match h {
                b'0'..=b'9' => (h - b'0') as u32,
                b'a'..=b'f' => (h - b'a' + 10) as u32,
                b'A'..=b'F' => (h - b'A' + 10) as u32,
                _ => return None,
            };
            v = (v << 4) | d;
        }
        Some(v)
    }
    // Parse a JSON string, decoding it to UTF-8 bytes appended to `out`.
    fn string_bytes(&mut self, out: &mut Vec<u8>) {
        self.ws();
        if self.p >= self.b.len() || self.b[self.p] != b'"' {
            return;
        }
        self.p += 1;
        while self.p < self.b.len() {
            let c = self.b[self.p];
            if c == b'"' {
                self.p += 1;
                return;
            }
            if c == b'\\' && self.p + 1 < self.b.len() {
                let e = self.b[self.p + 1];
                match e {
                    b'"' => { out.push(0x22); self.p += 2; }
                    b'\\' => { out.push(0x5C); self.p += 2; }
                    b'/' => { out.push(0x2F); self.p += 2; }
                    b'b' => { out.push(0x08); self.p += 2; }
                    b'f' => { out.push(0x0C); self.p += 2; }
                    b'n' => { out.push(0x0A); self.p += 2; }
                    b'r' => { out.push(0x0D); self.p += 2; }
                    b't' => { out.push(0x09); self.p += 2; }
                    b'u' => {
                        let v = self.hex4(self.p + 2).unwrap_or(0);
                        self.p += 6;
                        if (0xD800..=0xDBFF).contains(&v) {
                            // possible surrogate pair via a following \uXXXX
                            if self.p + 1 < self.b.len()
                                && self.b[self.p] == b'\\'
                                && self.b[self.p + 1] == b'u'
                            {
                                if let Some(lo) = self.hex4(self.p + 2) {
                                    if (0xDC00..=0xDFFF).contains(&lo) {
                                        let cp = 0x10000 + ((v - 0xD800) << 10) + (lo - 0xDC00);
                                        push_utf8(out, cp);
                                        self.p += 6;
                                        continue;
                                    }
                                }
                            }
                            push_utf8(out, v); // lone: CESU-8 bytes
                        } else {
                            push_utf8(out, v);
                        }
                    }
                    _ => { out.push(e); self.p += 2; }
                }
                continue;
            }
            if c < 0x80 {
                // Fast path: bulk-copy a run of plain ASCII bytes.
                let rest = &self.b[self.p..];
                let run = rest
                    .iter()
                    .position(|&x| x == b'"' || x == b'\\' || x >= 0x80)
                    .unwrap_or(rest.len());
                let run = if run == 0 { 1 } else { run };
                out.extend_from_slice(&rest[..run]);
                self.p += run;
                continue;
            }
            // Raw UTF-8 multibyte sequence: copy verbatim when well-formed.
            let adv = if c >= 0xF0 { 4 } else if c >= 0xE0 { 3 } else { 2 };
            let mut ok = self.p + adv <= self.b.len();
            if ok {
                for k in 1..adv {
                    if (self.b[self.p + k] & 0xC0) != 0x80 {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                out.extend_from_slice(&self.b[self.p..self.p + adv]);
                self.p += adv;
            } else {
                out.push(c);
                self.p += 1;
            }
        }
    }
    fn skip_string(&mut self) {
        self.ws();
        if self.p >= self.b.len() || self.b[self.p] != b'"' {
            return;
        }
        self.p += 1;
        while self.p < self.b.len() {
            let c = self.b[self.p];
            if c == b'\\' {
                self.p += 2;
                continue;
            }
            self.p += 1;
            if c == b'"' {
                return;
            }
        }
    }
    fn skip_value(&mut self) {
        match self.peek() {
            b'"' => self.skip_string(),
            b'{' => {
                self.p += 1;
                if self.eat(b'}') { return; }
                loop {
                    self.skip_string();
                    self.eat(b':');
                    self.skip_value();
                    if self.eat(b',') { continue; }
                    self.eat(b'}');
                    return;
                }
            }
            b'[' => {
                self.p += 1;
                if self.eat(b']') { return; }
                loop {
                    self.skip_value();
                    if self.eat(b',') { continue; }
                    self.eat(b']');
                    return;
                }
            }
            _ => {
                while self.p < self.b.len()
                    && !matches!(self.b[self.p], b',' | b'}' | b']' | b' ' | b'\t' | b'\n' | b'\r')
                {
                    self.p += 1;
                }
            }
        }
    }
    fn bool_value(&mut self) -> bool {
        self.ws();
        if self.b[self.p..].starts_with(b"true") {
            self.p += 4;
            true
        } else if self.b[self.p..].starts_with(b"false") {
            self.p += 5;
            false
        } else {
            self.skip_value();
            false
        }
    }
}

// ---------- error records ----------
fn err_open(errs: &mut Vec<u8>, code: &str, line: usize, row: Option<usize>) {
    if !errs.is_empty() {
        errs.push(b',');
    }
    vlit(errs, "{\"code\":\"");
    vlit(errs, code);
    vlit(errs, "\",\"line\":");
    vuint(errs, line);
    vlit(errs, ",\"row\":");
    match row {
        Some(r) => vuint(errs, r),
        None => vlit(errs, "null"),
    }
}
fn err_simple(errs: &mut Vec<u8>, code: &str, line: usize, row: Option<usize>) {
    err_open(errs, code, line, row);
    errs.push(b'}');
}

// ---------- unquoted scan ----------
// Appends the field's bytes to `scr` (dropping stray CRs), returns
// (ended, new_pos): ended 0 = comma, 1 = line break, 2 = end of text.
fn scan_unquoted(
    dec: &[u8],
    start: usize,
    line: &mut usize,
    scr: &mut Vec<u8>,
    errs: &mut Vec<u8>,
    record_line: usize,
    record_row: Option<usize>,
) -> (u8, usize) {
    let n = dec.len();
    let mut seg = start;
    loop {
        let rel = dec[seg..]
            .iter()
            .position(|&u| u == 44 || u == 10 || u == 13);
        let i = match rel {
            Some(r) => seg + r,
            None => {
                scr.extend_from_slice(&dec[seg..n]);
                return (2, n);
            }
        };
        let u = dec[i];
        if u == 44 {
            scr.extend_from_slice(&dec[seg..i]);
            return (0, i + 1);
        }
        if u == 10 {
            scr.extend_from_slice(&dec[seg..i]);
            *line += 1;
            return (1, i + 1);
        }
        // u == 13
        if i + 1 < n && dec[i + 1] == 10 {
            scr.extend_from_slice(&dec[seg..i]);
            *line += 1;
            return (1, i + 2);
        }
        scr.extend_from_slice(&dec[seg..i]);
        err_simple(errs, "stray_cr", record_line, record_row);
        seg = i + 1;
    }
}

// Fused unquoted scan + JSON emission: emits the field straight into
// `rec` when it contains no stray CR (the common case); only stray CRs
// route the value through `scr`. Returns (ended, new_pos, value_len).
fn scan_unquoted_emit(
    dec: &[u8],
    start: usize,
    line: &mut usize,
    rec: &mut Vec<u8>,
    scr: &mut Vec<u8>,
    errs: &mut Vec<u8>,
    record_line: usize,
    record_row: Option<usize>,
) -> (u8, usize, usize) {
    let n = dec.len();
    let mut seg = start;
    let mut stray = false;
    loop {
        let rel = dec[seg..]
            .iter()
            .position(|&u| u == 44 || u == 10 || u == 13);
        let i = match rel {
            Some(r) => seg + r,
            None => {
                if stray {
                    scr.extend_from_slice(&dec[seg..n]);
                    let vl = scr.len();
                    emit_json_str(rec, scr);
                    return (2, n, vl);
                }
                let vl = n - start;
                emit_json_str(rec, &dec[start..n]);
                return (2, n, vl);
            }
        };
        let u = dec[i];
        let (ended, np): (u8, usize) = if u == 44 {
            (0, i + 1)
        } else if u == 10 {
            *line += 1;
            (1, i + 1)
        } else if i + 1 < n && dec[i + 1] == 10 {
            *line += 1;
            (1, i + 2)
        } else {
            // stray CR
            if !stray {
                scr.clear();
                stray = true;
            }
            scr.extend_from_slice(&dec[seg..i]);
            err_simple(errs, "stray_cr", record_line, record_row);
            seg = i + 1;
            continue;
        };
        if stray {
            scr.extend_from_slice(&dec[seg..i]);
            let vl = scr.len();
            emit_json_str(rec, scr);
            return (ended, np, vl);
        }
        let vl = i - start;
        emit_json_str(rec, &dec[start..i]);
        return (ended, np, vl);
    }
}

// ---------- one case ----------
fn process_case(dec: &[u8], header: bool, out: &mut FixedOut) {
    let n = dec.len();
    let mut cols: Option<Vec<u8>> = None;
    let mut rows: Vec<u8> = Vec::new();
    let mut errs: Vec<u8> = Vec::new();
    let mut rec: Vec<u8> = Vec::new();
    let mut scr: Vec<u8> = Vec::new();
    let mut foff: Vec<usize> = Vec::new();
    let mut rows_count: usize = 0;
    let mut width: usize = 0;
    let mut first_pending = true;
    let mut pos: usize = if n >= 3 && dec[0] == 0xEF && dec[1] == 0xBB && dec[2] == 0xBF {
        3
    } else {
        0
    };
    let mut line: usize = 1;

    loop {
        // skip empty lines
        while pos < n {
            let c = dec[pos];
            if c == 10 {
                pos += 1;
                line += 1;
            } else if c == 13 && pos + 1 < n && dec[pos + 1] == 10 {
                pos += 2;
                line += 1;
            } else {
                break;
            }
        }
        if pos >= n {
            break;
        }
        let record_line = line;
        let record_row: Option<usize> = if header && first_pending {
            None
        } else {
            Some(rows_count)
        };
        rec.clear();
        foff.clear();
        let mut last_quoted = false;
        let mut last_value_len: usize = 0;

        // fields
        loop {
            if !foff.is_empty() {
                rec.push(b',');
            }
            let field_start = rec.len();
            foff.push(field_start);
            let ended: u8;
            if pos < n && dec[pos] == 0x22 {
                last_quoted = true;
                let open = pos;
                let open_line = line;
                // Probe: a quoted field whose first special char is a
                // lone closing quote needs no materialisation.
                let mut probe_clean: Option<usize> = None;
                {
                    let mut i = open + 1;
                    loop {
                        if i >= n {
                            break;
                        }
                        match dec[i..]
                            .iter()
                            .position(|&u| u == 0x22 || u == 0x0D || u == 0x0A)
                        {
                            Some(r) => {
                                let j = i + r;
                                if dec[j] == 0x22 && !(j + 1 < n && dec[j + 1] == 0x22) {
                                    probe_clean = Some(j);
                                }
                                break;
                            }
                            None => break,
                        }
                    }
                }
                if let Some(close) = probe_clean {
                    let vlen = close - (open + 1);
                    pos = close + 1;
                    if pos >= n {
                        emit_json_str(&mut rec, &dec[open + 1..close]);
                        last_value_len = vlen;
                        ended = 2;
                    } else {
                        let c = dec[pos];
                        if c == 44 {
                            pos += 1;
                            emit_json_str(&mut rec, &dec[open + 1..close]);
                            last_value_len = vlen;
                            ended = 0;
                        } else if c == 10 {
                            pos += 1;
                            line += 1;
                            emit_json_str(&mut rec, &dec[open + 1..close]);
                            last_value_len = vlen;
                            ended = 1;
                        } else if c == 13 && pos + 1 < n && dec[pos + 1] == 10 {
                            pos += 2;
                            line += 1;
                            emit_json_str(&mut rec, &dec[open + 1..close]);
                            last_value_len = vlen;
                            ended = 1;
                        } else {
                            // junk after the closing quote
                            scr.clear();
                            scr.extend_from_slice(&dec[open + 1..close]);
                            let (e2, np) = scan_unquoted(dec, pos, &mut line, &mut scr, &mut errs, record_line, record_row);
                            if scr.len() > vlen {
                                err_simple(&mut errs, "quote_junk", record_line, record_row);
                            }
                            last_value_len = scr.len();
                            emit_json_str(&mut rec, &scr);
                            ended = e2;
                            pos = np;
                        }
                    }
                } else {
                    // Slow path: materialise into scr.
                    scr.clear();
                    let mut i = open + 1;
                    let mut closed = false;
                    loop {
                        if i >= n {
                            break;
                        }
                        let rel = dec[i..]
                            .iter()
                            .position(|&u| u == 0x22 || u == 0x0D || u == 0x0A);
                        let j = match rel {
                            Some(r) => i + r,
                            None => {
                                scr.extend_from_slice(&dec[i..n]);
                                i = n;
                                break;
                            }
                        };
                        scr.extend_from_slice(&dec[i..j]);
                        let u = dec[j];
                        if u == 0x22 {
                            if j + 1 < n && dec[j + 1] == 0x22 {
                                scr.push(0x22);
                                i = j + 2;
                                continue;
                            }
                            closed = true;
                            i = j + 1;
                            break;
                        }
                        if u == 0x0D {
                            scr.push(0x0A);
                            line += 1;
                            i = if j + 1 < n && dec[j + 1] == 0x0A { j + 2 } else { j + 1 };
                            continue;
                        }
                        // u == 0x0A
                        line += 1;
                        scr.push(0x0A);
                        i = j + 1;
                    }
                    if !closed {
                        line = open_line;
                        err_simple(&mut errs, "unterminated_quote", record_line, record_row);
                        scr.clear();
                        scr.push(0x22);
                        let (e2, np) = scan_unquoted(dec, open + 1, &mut line, &mut scr, &mut errs, record_line, record_row);
                        ended = e2;
                        pos = np;
                    } else {
                        pos = i;
                        if pos >= n {
                            ended = 2;
                        } else {
                            let c = dec[pos];
                            if c == 44 {
                                pos += 1;
                                ended = 0;
                            } else if c == 10 {
                                pos += 1;
                                line += 1;
                                ended = 1;
                            } else if c == 13 && pos + 1 < n && dec[pos + 1] == 10 {
                                pos += 2;
                                line += 1;
                                ended = 1;
                            } else {
                                let quoted_len = scr.len();
                                let (e2, np) = scan_unquoted(dec, pos, &mut line, &mut scr, &mut errs, record_line, record_row);
                                if scr.len() > quoted_len {
                                    err_simple(&mut errs, "quote_junk", record_line, record_row);
                                }
                                ended = e2;
                                pos = np;
                            }
                        }
                    }
                    last_value_len = scr.len();
                    emit_json_str(&mut rec, &scr);
                }
            } else {
                last_quoted = false;
                let (e2, np, vl) = scan_unquoted_emit(dec, pos, &mut line, &mut rec, &mut scr, &mut errs, record_line, record_row);
                ended = e2;
                pos = np;
                last_value_len = vl;
            }
            if ended != 0 {
                break;
            }
        }

        // a line left with no characters is no record
        if foff.len() == 1 && last_value_len == 0 && !last_quoted {
            continue;
        }

        if first_pending {
            first_pending = false;
            width = foff.len();
            if header {
                cols = Some(rec.clone());
                continue;
            }
        }
        let count = foff.len();
        if count < width {
            err_open(&mut errs, "short_row", record_line, record_row);
            vlit(&mut errs, ",\"fields\":");
            vuint(&mut errs, count);
            errs.push(b'}');
            for _ in count..width {
                rec.push(b',');
                vs(&mut rec, b"\"\"");
            }
        } else if count > width {
            let extra_start = foff[width];
            err_open(&mut errs, "long_row", record_line, record_row);
            vlit(&mut errs, ",\"extra\":[");
            let extra_ptr = rec.as_ptr();
            let extra_len = rec.len() - extra_start;
            unsafe {
                let extra = slice::from_raw_parts(extra_ptr.add(extra_start), extra_len);
                vs(&mut errs, extra);
            }
            vlit(&mut errs, "]}");
            rec.truncate(extra_start - 1);
        }
        if !rows.is_empty() {
            rows.push(b',');
        }
        rows.push(b'[');
        vs(&mut rows, &rec);
        rows.push(b']');
        rows_count += 1;
    }

    out.lit("{\"columns\":");
    if header {
        match &cols {
            Some(c) => {
                out.b(b'[');
                out.s(c);
                out.b(b']');
            }
            None => out.lit("[]"),
        }
    } else {
        out.lit("null");
    }
    out.lit(",\"rows\":[");
    out.s(&rows);
    out.lit("],\"errors\":[");
    out.s(&errs);
    out.lit("]}");
}

// ---------- top level ----------
fn run_transduce(input: &[u8], out: &mut FixedOut) -> i64 {
    let mut j = Json { b: input, p: 0 };
    let mut keybuf: Vec<u8> = Vec::new();
    let mut dec: Vec<u8> = Vec::new();
    out.b(b'[');
    j.eat(b'[');
    let mut first_case = true;
    loop {
        if j.eat(b']') {
            break;
        }
        if !j.eat(b'{') {
            break;
        }
        let mut header = false;
        let mut have_input = false;
        loop {
            if j.eat(b'}') {
                break;
            }
            keybuf.clear();
            j.string_bytes(&mut keybuf);
            j.eat(b':');
            if keybuf == b"input" {
                j.eat(b'{');
                loop {
                    if j.eat(b'}') {
                        break;
                    }
                    keybuf.clear();
                    j.string_bytes(&mut keybuf);
                    j.eat(b':');
                    if keybuf == b"text" {
                        dec.clear();
                        j.string_bytes(&mut dec);
                        have_input = true;
                    } else if keybuf == b"header" {
                        header = j.bool_value();
                    } else {
                        j.skip_value();
                    }
                    if j.eat(b',') {
                        continue;
                    }
                    j.eat(b'}');
                    break;
                }
            } else {
                j.skip_value();
            }
            if j.eat(b',') {
                continue;
            }
            j.eat(b'}');
            break;
        }
        if have_input {
            if !first_case {
                out.b(b',');
            }
            first_case = false;
            process_case(&dec, header, out);
        }
        if j.eat(b',') {
            continue;
        }
        j.eat(b']');
        break;
    }
    out.b(b']');
    if out.overflow {
        -1
    } else {
        out.len as i64
    }
}

#[no_mangle]
pub extern "C" fn buf_alloc(size: usize) -> *mut u8 {
    let mut v: Vec<u8> = Vec::with_capacity(size);
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

#[no_mangle]
pub extern "C" fn buf_free(ptr: *mut u8, size: usize) {
    unsafe {
        drop(Vec::from_raw_parts(ptr, 0, size));
    }
}

#[no_mangle]
pub extern "C" fn transduce(
    in_ptr: *const u8,
    in_len: usize,
    out_ptr: *mut u8,
    out_cap: usize,
) -> i64 {
    let input = unsafe { slice::from_raw_parts(in_ptr, in_len) };
    let mut out = FixedOut {
        ptr: out_ptr,
        cap: out_cap,
        len: 0,
        overflow: false,
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_transduce(input, &mut out)
    }));
    match r {
        Ok(v) => v,
        Err(_) => -3,
    }
}

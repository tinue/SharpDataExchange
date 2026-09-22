//! Calc-U-1600 CE-1600F floppy image files (`<name>.floppy.yaml`) ⇄ two 64 KB sides.
//!
//! The container format belongs to Calc-U-1600 (`docs/Floppy-Image-Format.md` there;
//! reader/writer in `Core/Connector/FloppyImageFile.hpp`). This is an independent,
//! dependency-free implementation of version 1 of that spec:
//!
//! - the reader accepts the YAML subset Calc-U-1600's own reader does for this file
//!   (comments, quoted or bare scalars, keys in any order, `bytes: |` block scalars) and
//!   rejects unknown keys and any `format-version` other than 1;
//! - the writer emits exactly what Calc-U-1600's `formatFloppyFile` emits, so a file
//!   rewritten by sde differs from one saved by the emulator only where the bytes differ
//!   (and in `saved`).
//!
//! The filesystem inside a side is not this module's business — see [`crate::diskfs`].

use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};

/// Bytes per side (16 tracks × 8 sectors × 512 bytes).
pub const SIDE_SIZE: usize = 0x10000;
pub const FORMAT_TAG: &str = "ce1600f-floppy";
pub const FORMAT_VERSION: i64 = 1;
/// File-name suffix Calc-U-1600 scans for.
pub const FILE_SUFFIX: &str = ".floppy.yaml";

/// One side of the diskette; each is its own volume (the drive is single-sided, the
/// disk is flipped by hand).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    A,
    B,
}

impl Side {
    pub const BOTH: [Side; 2] = [Side::A, Side::B];

    pub fn index(self) -> usize {
        match self {
            Side::A => 0,
            Side::B => 1,
        }
    }

    pub fn from_str_ci(s: &str) -> Option<Side> {
        match s {
            "A" | "a" => Some(Side::A),
            "B" | "b" => Some(Side::B),
            _ => None,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Side::A => "a",
            Side::B => "b",
        }
    }
}

impl std::fmt::Display for Side {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Side::A => "A",
            Side::B => "B",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FloppyImage {
    pub disk_name: String,
    /// `saved:` timestamp as read (ISO-8601 UTC), if present.
    pub saved: Option<String>,
    /// Side A and side B, [`SIDE_SIZE`] bytes each.
    pub sides: [Vec<u8>; 2],
}

impl FloppyImage {
    pub fn side(&self, side: Side) -> &[u8] {
        &self.sides[side.index()]
    }

    pub fn side_mut(&mut self, side: Side) -> &mut Vec<u8> {
        &mut self.sides[side.index()]
    }
}

// ── reader ──────────────────────────────────────────────────────────────

/// Parse a `.floppy.yaml` file.
pub fn parse(text: &str) -> Result<FloppyImage> {
    let root = parse_yaml_maps(text)?;
    root.require_only(&["format", "format-version", "disk-name", "saved", "sides"])?;

    let format = root.scalar("format")?;
    if format.as_deref() != Some(FORMAT_TAG) {
        bail!("not a floppy-disk file (expected 'format: {FORMAT_TAG}')");
    }
    let version = root.scalar("format-version")?.ok_or_else(|| anyhow!("missing 'format-version'"))?;
    let version = parse_int(&version).ok_or_else(|| anyhow!("'format-version' is not an integer: {version:?}"))?;
    if version != FORMAT_VERSION {
        bail!("unsupported floppy format-version {version} (sde reads {FORMAT_VERSION})");
    }
    let disk_name = root.scalar("disk-name")?.ok_or_else(|| anyhow!("missing 'disk-name'"))?;
    if disk_name.is_empty() {
        bail!("empty 'disk-name'");
    }
    let saved = root.scalar("saved")?;

    let sides = root.map("sides")?.ok_or_else(|| anyhow!("missing 'sides' mapping"))?;
    sides.require_only(&["a", "b"])?;
    let mut out = [Vec::new(), Vec::new()];
    for side in Side::BOTH {
        let key = side.key();
        let node = sides.map(key)?.ok_or_else(|| anyhow!("side '{key}': missing"))?;
        node.require_only(&["encoding", "bytes"])?;
        if node.scalar("encoding")?.as_deref() != Some("addressed-hex") {
            bail!("side '{key}': expected 'encoding: addressed-hex'");
        }
        let bytes = node.scalar("bytes")?.ok_or_else(|| anyhow!("side '{key}': missing 'bytes'"))?;
        out[side.index()] = parse_addressed_hex(&bytes, SIDE_SIZE).with_context(|| format!("side '{key}'"))?;
    }
    Ok(FloppyImage { disk_name, saved, sides: out })
}

/// Read and parse a `.floppy.yaml` file from disk.
pub fn read_path(path: &Path) -> Result<FloppyImage> {
    let text = std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    parse(&text).with_context(|| format!("{}", path.display()))
}

/// Decimal, or `0x…` hex (Calc-U-1600's `asInt`).
fn parse_int(s: &str) -> Option<i64> {
    let s = s.trim();
    let (neg, digits) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    let v = match digits.strip_prefix("0x").or_else(|| digits.strip_prefix("0X")) {
        Some(hex) => i64::from_str_radix(hex, 16).ok()?,
        None => digits.parse::<i64>().ok()?,
    };
    Some(if neg { -v } else { v })
}

/// A node of the tiny YAML subset this file uses: a scalar or a block mapping.
enum Node {
    Scalar(String),
    Map(Vec<(String, Node)>),
}

impl Node {
    fn entries(&self) -> &[(String, Node)] {
        match self {
            Node::Map(m) => m,
            Node::Scalar(_) => &[],
        }
    }

    fn get(&self, key: &str) -> Option<&Node> {
        self.entries().iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    fn require_only(&self, allowed: &[&str]) -> Result<()> {
        for (k, _) in self.entries() {
            if !allowed.contains(&k.as_str()) {
                bail!("unknown key '{k}'");
            }
        }
        Ok(())
    }

    fn scalar(&self, key: &str) -> Result<Option<String>> {
        match self.get(key) {
            None => Ok(None),
            Some(Node::Scalar(s)) => Ok(Some(s.clone())),
            Some(Node::Map(_)) => bail!("'{key}' must be a scalar"),
        }
    }

    fn map(&self, key: &str) -> Result<Option<&Node>> {
        match self.get(key) {
            None => Ok(None),
            Some(n @ Node::Map(_)) => Ok(Some(n)),
            Some(Node::Scalar(_)) => bail!("'{key}' must be a mapping"),
        }
    }
}

struct Line<'a> {
    no: usize,
    indent: usize,
    /// Content without indent; `None` for blank / whole-line comment lines.
    body: Option<&'a str>,
    raw: &'a str,
}

fn parse_yaml_maps(text: &str) -> Result<Node> {
    let mut lines = Vec::new();
    for (i, raw) in text.split('\n').enumerate() {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        let trimmed = raw.trim_start_matches(' ');
        let indent = raw.len() - trimmed.len();
        if trimmed.starts_with('\t') {
            bail!("line {}: tabs are not allowed for indentation", i + 1);
        }
        let body = trimmed.trim_end();
        let body = if body.is_empty() || body.starts_with('#') { None } else { Some(body) };
        lines.push(Line { no: i + 1, indent, body, raw });
    }
    let mut pos = 0;
    let root = parse_map(&lines, &mut pos, 0)?;
    if let Some(l) = lines[pos..].iter().find(|l| l.body.is_some()) {
        bail!("line {}: unexpected indentation", l.no);
    }
    Ok(root)
}

/// Parse a block mapping whose keys sit at exactly `indent`.
fn parse_map(lines: &[Line], pos: &mut usize, indent: usize) -> Result<Node> {
    let mut entries: Vec<(String, Node)> = Vec::new();
    while *pos < lines.len() {
        let line = &lines[*pos];
        let Some(body) = line.body else {
            *pos += 1;
            continue;
        };
        if line.indent < indent {
            break;
        }
        if line.indent > indent {
            bail!("line {}: unexpected indentation", line.no);
        }
        let (key, rest) = body
            .split_once(':')
            .ok_or_else(|| anyhow!("line {}: expected 'key: value'", line.no))?;
        let key = key.trim().to_string();
        if !rest.is_empty() && !rest.starts_with(' ') {
            bail!("line {}: expected a space after ':'", line.no);
        }
        if entries.iter().any(|(k, _)| *k == key) {
            bail!("line {}: duplicate key '{key}'", line.no);
        }
        let value = strip_comment(rest.trim());
        *pos += 1;
        let node = if value == "|" {
            // Literal block scalar: following lines indented deeper than the key, or blank.
            let mut block = Vec::new();
            let mut common: Option<usize> = None;
            while *pos < lines.len() {
                let l = &lines[*pos];
                let blank = l.raw.trim().is_empty();
                if !blank && l.indent <= indent {
                    break;
                }
                if !blank {
                    common = Some(common.map_or(l.indent, |c| c.min(l.indent)));
                }
                block.push(l);
                *pos += 1;
            }
            let common = common.unwrap_or(0);
            let mut s = String::new();
            for l in block {
                if l.raw.len() >= common {
                    s.push_str(l.raw[common..].trim_end());
                }
                s.push('\n');
            }
            Node::Scalar(s)
        } else if value.is_empty() {
            // Nested mapping on the following, deeper-indented lines.
            let child_indent = lines[*pos..]
                .iter()
                .find(|l| l.body.is_some())
                .map(|l| l.indent)
                .filter(|&i| i > indent);
            match child_indent {
                Some(ci) => parse_map(lines, pos, ci)?,
                None => Node::Scalar(String::new()),
            }
        } else {
            Node::Scalar(unquote(value, line.no)?)
        };
        entries.push((key, node));
    }
    Ok(Node::Map(entries))
}

/// Drop a ` #…` trailing comment outside quotes.
fn strip_comment(s: &str) -> &str {
    let mut quote: Option<char> = None;
    let mut prev_space = true;
    for (i, c) in s.char_indices() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == '#' && prev_space => return s[..i].trim_end(),
            None => {}
        }
        prev_space = c == ' ';
    }
    s
}

/// Single- or double-quoted scalars (no escape processing), else the bare value.
fn unquote(s: &str, line: usize) -> Result<String> {
    for q in ['"', '\''] {
        if let Some(rest) = s.strip_prefix(q) {
            return match rest.strip_suffix(q) {
                Some(inner) if !inner.contains(q) => Ok(inner.to_string()),
                _ => bail!("line {line}: unterminated or malformed quoted value"),
            };
        }
    }
    Ok(s.to_string())
}

/// Parse an `addressed-hex` block (Calc-U-1600 `docs/Memory-Card-Definition-Format.md`
/// §6) covering exactly `len` bytes: lines `$ADDR: XX XX …` or `$ADDR: XX...` (byte XX
/// repeated up to the next line's address or the end). Lines must partition `[0, len)`.
pub fn parse_addressed_hex(text: &str, len: usize) -> Result<Vec<u8>> {
    struct HexLine {
        addr: usize,
        run: Option<u8>,
        bytes: Vec<u8>,
    }
    let mut lines = Vec::new();
    for raw in text.split('\n') {
        let s = raw.trim_matches([' ', '\t', '\r']);
        if s.is_empty() {
            continue;
        }
        let Some(rest) = s.strip_prefix('$') else {
            bail!("addressed-hex line must start with '$': '{s}'");
        };
        let (addr, data) = rest.split_once(':').ok_or_else(|| anyhow!("addressed-hex line missing ':': '{s}'"))?;
        let addr = usize::from_str_radix(addr, 16).map_err(|_| anyhow!("bad address in addressed-hex line: '{s}'"))?;
        let toks: Vec<&str> = data.split_ascii_whitespace().collect();
        if toks.is_empty() {
            bail!("empty addressed-hex line: '{s}'");
        }
        if toks.len() == 1 && toks[0].len() == 5 && toks[0].ends_with("...") {
            let v = hex_pair(&toks[0][..2]).ok_or_else(|| anyhow!("bad run byte in addressed-hex line: '{s}'"))?;
            lines.push(HexLine { addr, run: Some(v), bytes: Vec::new() });
        } else {
            let mut bytes = Vec::with_capacity(toks.len());
            for t in toks {
                bytes.push(
                    hex_pair(t).ok_or_else(|| anyhow!("'{t}' is not a hex byte pair in addressed-hex line: '{s}'"))?,
                );
            }
            lines.push(HexLine { addr, run: None, bytes });
        }
    }
    if lines.is_empty() {
        bail!("addressed-hex block has no lines");
    }
    lines.sort_by_key(|l| l.addr);

    let mut out = vec![0u8; len];
    let mut cursor = 0usize;
    for i in 0..lines.len() {
        let line = &lines[i];
        if line.addr != cursor {
            bail!("addressed-hex lines have a gap or overlap at/near ${cursor:04X}");
        }
        match line.run {
            Some(b) => {
                let end = lines.get(i + 1).map_or(len, |n| n.addr);
                if end <= cursor || end > len {
                    bail!("addressed-hex run at ${cursor:04X} has an invalid length");
                }
                out[cursor..end].fill(b);
                cursor = end;
            }
            None => {
                let end = cursor + line.bytes.len();
                if end > len {
                    bail!("addressed-hex line at ${cursor:04X} overruns the block");
                }
                out[cursor..end].copy_from_slice(&line.bytes);
                cursor = end;
            }
        }
    }
    if cursor != len {
        bail!("addressed-hex block covers 0x{cursor:X} bytes but needs 0x{len:X}");
    }
    Ok(out)
}

fn hex_pair(t: &str) -> Option<u8> {
    if t.len() == 2 && t.bytes().all(|b| b.is_ascii_hexdigit()) {
        u8::from_str_radix(t, 16).ok()
    } else {
        None
    }
}

// ── writer ──────────────────────────────────────────────────────────────

/// Serialize with the given `saved` timestamp — byte-identical to Calc-U-1600's
/// `formatFloppyFile` for the same inputs.
pub fn format_with_saved(img: &FloppyImage, saved: &str) -> Result<String> {
    if img.disk_name.is_empty() || img.disk_name.contains(['"', '\n', '\r']) {
        bail!("disk name {:?} cannot be written (empty, or contains '\"' or a newline)", img.disk_name);
    }
    let mut out = String::with_capacity(4096);
    out.push_str("# Calc-U-1600 CE-1600F floppy disk image\n");
    out.push_str(&format!("format: {FORMAT_TAG}\n"));
    out.push_str(&format!("format-version: {FORMAT_VERSION}\n"));
    out.push_str(&format!("disk-name: \"{}\"\n", img.disk_name));
    out.push_str(&format!("saved: {saved}\n"));
    out.push_str("sides:\n");
    for side in Side::BOTH {
        let bytes = img.side(side);
        if bytes.len() != SIDE_SIZE {
            bail!("side {side} is {} bytes, expected {SIDE_SIZE}", bytes.len());
        }
        out.push_str(&format!("  {}:\n", side.key()));
        out.push_str("    encoding: addressed-hex\n");
        out.push_str("    bytes: |\n");
        for line in format_addressed_hex(bytes) {
            out.push_str("      ");
            out.push_str(&line);
            out.push('\n');
        }
    }
    Ok(out)
}

/// Serialize with `saved` set to the current UTC time.
pub fn format(img: &FloppyImage) -> Result<String> {
    format_with_saved(img, &now_iso8601_utc())
}

/// `addressed-hex` lines for `bytes`, mirroring Calc-U-1600's `formatAddressedHexLines`:
/// 16-byte rows (uppercase hex, two spaces before the 9th byte), and a run of whole
/// uniform rows collapsed to `$ADDR: XX...`.
pub fn format_addressed_hex(bytes: &[u8]) -> Vec<String> {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let n = bytes.len();
    let mut lines = Vec::new();
    let mut i = 0;
    while i < n {
        let row_end = (i + 16).min(n);
        let first = bytes[i];
        if bytes[i..row_end].iter().any(|&b| b != first) {
            let mut s = format!("${i:04X}: ");
            for (k, &b) in bytes[i..row_end].iter().enumerate() {
                if k != 0 {
                    s.push_str(if k == 8 { "  " } else { " " });
                }
                s.push(HEX[(b >> 4) as usize] as char);
                s.push(HEX[(b & 0x0F) as usize] as char);
            }
            lines.push(s);
            i = row_end;
            continue;
        }
        let mut run_end = row_end;
        while run_end + 16 <= n && bytes[run_end..run_end + 16].iter().all(|&b| b == first) {
            run_end += 16;
        }
        lines.push(format!("${i:04X}: {first:02X}..."));
        i = run_end;
    }
    lines
}

/// Current UTC time as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn now_iso8601_utc() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    iso8601_from_unix(secs)
}

fn iso8601_from_unix(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Days since 1970-01-01 → (year, month, day), proleptic Gregorian (H. Hinnant).
pub(crate) fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// Replace `path` with `text` atomically: write a temporary file next to it, then rename
/// over the original, so a failure never leaves a half-written image.
pub fn write_path_atomic(path: &Path, text: &str) -> Result<()> {
    use std::io::Write;
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("image");
    let tmp = dir.join(format!(".{name}.sde-{}.tmp", std::process::id()));
    let result = (|| -> Result<()> {
        let mut f = std::fs::File::create(&tmp).with_context(|| format!("cannot create {}", tmp.display()))?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path).with_context(|| format!("cannot replace {}", path.display()))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/floppy").join(name);
        std::fs::read_to_string(p).unwrap()
    }

    #[test]
    fn fixtures_parse_and_rewrite_byte_identical() {
        for name in ["blank.floppy.yaml", "formatted.floppy.yaml", "dw.floppy.yaml"] {
            let text = fixture(name);
            let img = parse(&text).unwrap_or_else(|e| panic!("{name}: {e:#}"));
            assert_eq!(img.sides[0].len(), SIDE_SIZE);
            assert_eq!(img.sides[1].len(), SIDE_SIZE);
            let again = format_with_saved(&img, img.saved.as_deref().unwrap()).unwrap();
            assert_eq!(again, text, "{name} not byte-identical");
            assert_eq!(parse(&again).unwrap(), img);
        }
    }

    #[test]
    fn formatted_fixture_contents() {
        let img = parse(&fixture("formatted.floppy.yaml")).unwrap();
        assert_eq!(img.disk_name, "Formatted");
        for side in Side::BOTH {
            assert_eq!(img.side(side)[0x200], 0xF2); // FAT media byte
            assert_eq!(img.side(side)[0x400], 0xF2); // FAT copy
        }
    }

    fn minimal(extra_top: &str, side_a: &str) -> String {
        format!(
            "format: ce1600f-floppy\nformat-version: 1\ndisk-name: \"X\"\n{extra_top}sides:\n  a:\n    encoding: addressed-hex\n    bytes: |\n{side_a}  b:\n    encoding: addressed-hex\n    bytes: |\n      $0000: 00...\n"
        )
    }

    #[test]
    fn reader_accepts_subset_variants() {
        let text = "# c\ndisk-name: 'Single' # trailing\nformat-version: 0x1\nformat: ce1600f-floppy\nsides:\n  b:\n    bytes: |\n      $0000: 00...\n    encoding: \"addressed-hex\"\n  a:\n    encoding: addressed-hex\n    bytes: |\n      $0010: 11...\n\n      $0000: 01 02 03 04 05 06 07 08  09 0A 0B 0C 0D 0E 0F 10\r\n";
        let img = parse(text).unwrap();
        assert_eq!(img.disk_name, "Single");
        assert_eq!(img.saved, None);
        assert_eq!(img.sides[0][0x0F], 0x10);
        assert_eq!(img.sides[0][0xFFFF], 0x11);
    }

    #[test]
    fn reader_rejects() {
        let ok_a = "      $0000: 00...\n";
        assert!(parse(&minimal("", ok_a)).is_ok());
        let cases: Vec<(String, &str)> = vec![
            (minimal("", ok_a).replace("format-version: 1", "format-version: 2"), "format-version 2"),
            (minimal("", ok_a).replace("format-version: 1\n", ""), "missing 'format-version'"),
            (minimal("", ok_a).replace("ce1600f-floppy", "ce1600m-card"), "not a floppy-disk file"),
            (minimal("", ok_a).replace("\"X\"", "\"\""), "empty 'disk-name'"),
            (minimal("colour: red\n", ok_a), "unknown key 'colour'"),
            (minimal("disk-name: \"Y\"\n", ok_a), "duplicate key"),
            (minimal("", ok_a).replace("  b:\n    encoding: addressed-hex\n    bytes: |\n      $0000: 00...\n", ""), "side 'b': missing"),
            (minimal("", ok_a).replacen("encoding: addressed-hex", "encoding: hex", 1), "expected 'encoding: addressed-hex'"),
            (minimal("", "      $0000: 00 01\n      $0004: 00...\n"), "gap or overlap"),
            (minimal("", "      $0000: 00...\n      $0000: 00...\n"), "invalid length"),
            (minimal("", "      $0000: 00 01\n      $0001: 00...\n"), "gap or overlap"),
            (minimal("", "      $0000: 00 01\n"), "covers 0x2 bytes"),
            (minimal("", "      $FFFF: 00...\n      $0000: 0G...\n"), "bad run byte"),
            (minimal("", "      $0000: 00 1\n"), "not a hex byte pair"),
            (minimal("", "      $0000: 00 FF...\n"), "not a hex byte pair"),
            (minimal("", "      $FFFF: 00 01\n      $0000: 00...\n"), "overruns"),
            (minimal("", ok_a).replace("  a:", "\ta:"), "tabs"),
        ];
        for (text, want) in cases {
            let err = format!("{:#}", parse(&text).unwrap_err());
            assert!(err.contains(want), "expected {want:?} in {err:?}");
        }
    }

    #[test]
    fn writer_rows_runs_and_partial_rows() {
        let mut b = vec![0u8; 64];
        b[16..32].copy_from_slice(&[0xFF; 16]);
        b[40] = 0xAB;
        let lines = format_addressed_hex(&b);
        assert_eq!(
            lines,
            vec![
                "$0000: 00...",
                "$0010: FF...",
                "$0020: 00 00 00 00 00 00 00 00  AB 00 00 00 00 00 00 00",
                "$0030: 00...",
            ]
        );
        assert_eq!(parse_addressed_hex(&lines.join("\n"), 64).unwrap(), b);
        // Partial trailing row: never merged into the previous run.
        let b = vec![7u8; 20];
        assert_eq!(format_addressed_hex(&b), vec!["$0000: 07...", "$0010: 07..."]);
    }

    #[test]
    fn writer_rejects_unwritable_disk_name() {
        let mut img = parse(&fixture("blank.floppy.yaml")).unwrap();
        img.disk_name = "a\"b".into();
        assert!(format_with_saved(&img, "x").is_err());
    }

    #[test]
    fn iso8601() {
        assert_eq!(iso8601_from_unix(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601_from_unix(1_709_210_096), "2024-02-29T12:34:56Z");
        assert_eq!(iso8601_from_unix(1_790_000_000), "2026-09-21T14:13:20Z");
    }

    #[test]
    fn atomic_write_replaces() {
        let dir = std::env::temp_dir().join(format!("sde_fi_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("x.floppy.yaml");
        std::fs::write(&p, "old").unwrap();
        write_path_atomic(&p, "new").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "new");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }
}

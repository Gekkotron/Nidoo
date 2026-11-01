//! Guard: user-facing text belongs in the FR/EN table (`src/i18n/`), not in a bare literal.
//! `bare_text_allow.txt` lists the files not translated yet (`file:`) and the literals that are
//! not words (`text:`); an entry nothing uses fails the test, so the list only shrinks.
//! `BARE_TEXT_BLESS=1` rewrites the `file:` lines from what the scan finds.
#![cfg(not(target_arch = "wasm32"))]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Attribute values that are always read by people, even in lower case.
const READ_ATTRS: &[&str] = &["aria-label=", "title=", "placeholder=", "alt="];

/// What a literal right after these is: a class, an id, a URL… never a sentence.
const TECH_ATTRS: &[&str] = &[
    "class=",
    "class:",
    "attr:class=",
    "id=",
    "href=",
    "style=",
    "style:",
    "src=",
    "type=",
    "key=",
    "for=",
    "role=",
    "inputmode=",
    "autocomplete=",
    "inner_html=",
    "viewBox=",
    "d=",
];

/// A line with any of these holds no user-facing literal.
const TECH_LINES: &[&str] = &[
    "log::",
    "web_sys::console",
    "expect(",
    "panic!",
    "unreachable!",
    "assert",
    "#[",
    "concat!",
    "include_",
    "query_selector",
    "serde",
    "rename",
    "set_property",
    "set_attribute",
    "get_item",
    "set_item",
];

/// The user-facing literals in `source`, in order.
fn flagged(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut skip_depth: Option<i32> = None;
    let mut skip_top = false;
    let mut pending_cfg_test = false;
    // Brace depth inside a `view!` block, where a literal alone on its line is view text.
    let mut view_depth = 0;
    for raw in source.lines() {
        let line = strip_comment(raw);
        if let Some(depth) = skip_depth.as_mut() {
            // A multi-line raw string (`br#"{ … }"#`) unbalances `braces`, so a top-level test
            // module ends at its `}` in the first column, not when the count reaches zero.
            *depth += braces(&line);
            if if skip_top {
                line.trim_end() == "}"
            } else {
                *depth <= 0
            } {
                skip_depth = None;
            }
            continue;
        }
        let trimmed = line.trim();
        if trimmed == "#[cfg(test)]" {
            pending_cfg_test = true;
            continue;
        }
        if pending_cfg_test {
            pending_cfg_test = false;
            if let Some(rest) = trimmed.strip_prefix("mod ") {
                let depth = braces(&line);
                if depth > 0 {
                    skip_depth = Some(depth);
                    skip_top = !raw.starts_with(char::is_whitespace);
                    continue;
                }
                // `#[cfg(test)]` then `mod tests { … }` on one line: nothing to scan.
                if rest.contains('{') {
                    continue;
                }
            }
        }
        let in_view = view_depth > 0;
        if view_depth > 0 {
            view_depth = (view_depth + braces(&line)).max(0);
        } else if let Some(at) = line.find("view!") {
            view_depth = braces(&line[at..]).max(0);
        }
        if TECH_LINES.iter().any(|t| line.contains(t)) {
            continue;
        }
        for (start, lit) in literals(&line) {
            let before = line[..start].trim_end();
            let after = line[start + lit.len() + 2..].trim();
            // View text (`>"now"`, or a literal alone on its line) is read even in lower case.
            let after_tag =
                before.ends_with('>') && !before.ends_with("=>") && !before.ends_with("->");
            let in_text =
                after_tag || (in_view && before.is_empty() && matches!(after, "" | "," | "}"));
            let read = in_text || READ_ATTRS.iter().any(|a| before.ends_with(a));
            if !read && TECH_ATTRS.iter().any(|a| ends_with_attr(before, a)) {
                continue;
            }
            if looks_like_text(&lit, read) {
                out.push(lit);
            }
        }
    }
    out
}

/// `before` ends with the attribute `attr` as a whole name: `d=` matches `d=` and ` d=` but not
/// `checked=`, and `id=` not `valid=`.
fn ends_with_attr(before: &str, attr: &str) -> bool {
    before.strip_suffix(attr).is_some_and(|head| {
        !head
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == ':')
    })
}

/// A literal people read: a word of two letters or more (placeholders like `{hour}` aside), and
/// either a space or a capital first letter, unless it sits in a read attribute. SVG paths and
/// markup are not text.
fn looks_like_text(lit: &str, read_attr: bool) -> bool {
    let bare = strip_placeholders(lit);
    if lit.starts_with(['<', '/', '.', '#']) || is_svg_path(lit) || is_css(lit) || is_classes(&bare)
    {
        return false;
    }
    let has_word = bare
        .as_bytes()
        .windows(2)
        .any(|w| w[0].is_ascii_alphabetic() && w[1].is_ascii_alphabetic());
    if !has_word {
        return false;
    }
    read_attr
        || bare.contains(' ')
        || bare
            .chars()
            .find(|c| !c.is_whitespace())
            .is_some_and(char::is_uppercase)
}

/// Path data ("M12 4L4 12 Z", "H{x}V{y}"): path commands and numbers only, with a number, and
/// never two letters in a row (placeholders count as numbers), so "Stats" or "Last 24 h" are words.
fn is_svg_path(lit: &str) -> bool {
    let mut s = String::new();
    let mut depth = 0;
    for c in lit.chars() {
        match c {
            '{' => {
                if depth == 0 {
                    s.push('0');
                }
                depth += 1;
            }
            '}' if depth > 0 => depth -= 1,
            _ if depth == 0 => s.push(c),
            _ => {}
        }
    }
    let letters_apart = !s
        .as_bytes()
        .windows(2)
        .any(|w| w[0].is_ascii_alphabetic() && w[1].is_ascii_alphabetic());
    s.chars().any(|c| c.is_ascii_digit())
        && letters_apart
        && s.chars()
            .all(|c| "MmLlHhVvCcSsQqTtAaZz0123456789 ,.-".contains(c))
}

/// The CSS properties the UI writes inline.
const PROPERTIES: &[&str] = &[
    "left",
    "right",
    "top",
    "bottom",
    "width",
    "height",
    "min-width",
    "max-width",
    "min-height",
    "max-height",
    "transform",
    "display",
    "background",
    "background-size",
    "background-position",
    "color",
    "opacity",
    "fill",
    "stop-color",
    "d",
    "stroke",
    "stroke-width",
    "stroke-dasharray",
    "animation-duration",
    "animation-delay",
    "transition",
    "margin",
    "padding",
    "gap",
    "flex",
    "grid-template-columns",
    "font-size",
    "z-index",
    "border",
];

/// A CSS declaration or value: "left: {}%", "hsl({} {}% 55%)", "var(--x)".
fn is_css(s: &str) -> bool {
    const FUNCS: &[&str] = &[
        "hsl(",
        "rgb(",
        "url(",
        "translate",
        "calc(",
        "var(--",
        "rotate(",
    ];
    // Only a real property name: "error: {e}" or "failed: {}" is a sentence people read.
    let property = s
        .split_once(':')
        .is_some_and(|(p, _)| PROPERTIES.contains(&p));
    property || FUNCS.iter().any(|f| s.contains(f))
}

/// CSS class names (BEM): lower case, and a `__` element or a `--` modifier somewhere.
fn is_classes(s: &str) -> bool {
    (s.contains("__") || s.contains("--"))
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || " -_".contains(c))
}

fn strip_placeholders(s: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in s.chars() {
        match c {
            '{' => depth += 1,
            '}' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// The line up to a `//` that is outside a string.
fn strip_comment(line: &str) -> String {
    let mut in_str = false;
    let mut escaped = false;
    let bytes = line.as_bytes();
    for i in 0..bytes.len() {
        let b = bytes[i];
        if in_str {
            match (escaped, b) {
                (true, _) => escaped = false,
                (false, b'\\') => escaped = true,
                (false, b'"') => in_str = false,
                _ => {}
            }
        } else if b == b'"' {
            in_str = true;
        } else if b == b'/' && bytes.get(i + 1) == Some(&b'/') {
            return line[..i].to_owned();
        }
    }
    line.to_owned()
}

/// `{` minus `}` outside strings.
fn braces(line: &str) -> i32 {
    let mut n = 0;
    let mut in_str = false;
    let mut escaped = false;
    for c in line.chars() {
        if in_str {
            match (escaped, c) {
                (true, _) => escaped = false,
                (false, '\\') => escaped = true,
                (false, '"') => in_str = false,
                _ => {}
            }
        } else {
            match c {
                '"' => in_str = true,
                '{' => n += 1,
                '}' => n -= 1,
                _ => {}
            }
        }
    }
    n
}

/// The plain `"…"` literals of a line with the byte offset of their opening quote. Raw strings
/// and char literals are not scanned.
fn literals(line: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' && (i == 0 || (bytes[i - 1] != b'#' && bytes[i - 1] != b'r')) {
            let start = i;
            let mut j = i + 1;
            let mut value = String::new();
            let mut closed = false;
            while j < bytes.len() {
                match bytes[j] {
                    b'\\' => {
                        j += 2;
                        value.push(' ');
                        continue;
                    }
                    b'"' => {
                        closed = true;
                        break;
                    }
                    _ => {}
                }
                j += 1;
            }
            if closed {
                value = line[start + 1..j].to_owned();
                out.push((start, value));
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
    out
}

fn src_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn allow_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/bare_text_allow.txt")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().collect();
    entries.sort_by_key(|e| e.path());
    for e in entries {
        let path = e.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "i18n") && path.parent() == Some(&src_dir()) {
                continue;
            }
            rust_files(&path, out);
        } else if path.extension().is_some_and(|x| x == "rs") {
            out.push(path);
        }
    }
}

/// `src/<path>` for a file under `src/`.
fn rel(path: &Path) -> String {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"));
    path.strip_prefix(base)
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/")
}

struct Allow {
    files: BTreeSet<String>,
    texts: BTreeSet<String>,
    /// The `text:` lines as written, kept when blessing.
    text_lines: Vec<String>,
}

fn read_allow() -> Allow {
    let body = std::fs::read_to_string(allow_path()).unwrap_or_default();
    let mut allow = Allow {
        files: BTreeSet::new(),
        texts: BTreeSet::new(),
        text_lines: Vec::new(),
    };
    for line in body.lines() {
        let value = |rest: &str| rest.split("  #").next().unwrap_or("").trim().to_owned();
        if let Some(rest) = line.strip_prefix("file:") {
            allow.files.insert(value(rest));
        } else if let Some(rest) = line.strip_prefix("text:") {
            allow.texts.insert(value(rest));
            allow.text_lines.push(line.to_owned());
        }
    }
    allow
}

#[test]
fn no_bare_text_outside_the_table() {
    let allow = read_allow();
    let mut files = Vec::new();
    rust_files(&src_dir(), &mut files);

    let mut hits: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut used_texts = BTreeSet::new();
    for path in &files {
        let source = std::fs::read_to_string(path).unwrap();
        for lit in flagged(&source) {
            if allow.texts.contains(&lit) {
                used_texts.insert(lit);
                continue;
            }
            hits.entry(rel(path)).or_default().push(lit);
        }
    }

    if std::env::var_os("BARE_TEXT_BLESS").is_some() {
        let mut body = String::from(
            "# Ratchet for tests/bare_text.rs: delete a file line once that file is translated.\n",
        );
        for file in hits.keys() {
            body.push_str(&format!("file: {file}  # not translated yet\n"));
        }
        for line in &allow.text_lines {
            body.push_str(line);
            body.push('\n');
        }
        std::fs::write(allow_path(), body).unwrap();
        return;
    }

    let mut problems = Vec::new();
    for (file, lits) in &hits {
        if !allow.files.contains(file) {
            for lit in lits {
                problems.push(format!("{file}: \"{lit}\" belongs in src/i18n/"));
            }
        }
    }
    for file in &allow.files {
        if !hits.contains_key(file) {
            problems.push(format!(
                "file: {file} has no bare text left; delete its line"
            ));
        }
    }
    for text in &allow.texts {
        if !used_texts.contains(text) {
            problems.push(format!("text: {text} is never used; delete its line"));
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn flags_view_text_and_attributes() {
    assert_eq!(flagged(r#"<button>"Save"</button>"#), ["Save"]);
    assert_eq!(flagged(r#"<input placeholder="search"/>"#), ["search"]);
    assert_eq!(flagged(r#"toasts.push("Flow saved");"#), ["Flow saved"]);
}

#[test]
fn flags_lowercase_words_people_read() {
    assert_eq!(
        flagged("view! {\n<span>\n    \"retry\"\n</span>\n}"),
        ["retry"]
    );
    assert_eq!(flagged(r#"<p>"now"</p>"#), ["now"]);
    assert!(flagged(r#"Category::Lights => "lights","#).is_empty());
    // Outside view!, a lone lower-case word reads like an id ("door_out", "on"): not flagged.
    // The area recipe greps for those by hand.
    assert!(flagged("const C: [&str; 2] = [\n    \"red\",\n    \"blue\",\n];").is_empty());
}

#[test]
fn words_made_of_path_letters_are_still_words() {
    assert_eq!(flagged(r#"let t = "Last 24 h";"#), ["Last 24 h"]);
    assert_eq!(flagged(r#"let s = "Stats";"#), ["Stats"]);
}

#[test]
fn ignores_technical_literals() {
    assert!(flagged(r#"<div class="card card--wide">"#).is_empty());
    assert!(flagged(r#"let id = "group_cover_living";"#).is_empty());
    assert!(flagged(r#"#[serde(rename = "Light")]"#).is_empty());
    assert!(flagged(r#"// "Not this""#).is_empty());
    assert!(flagged("#[cfg(test)]\nmod tests { const X: &str = \"Some text\"; }").is_empty());
    assert!(flagged(r#"format!("{hour} h {minute}")"#).is_empty());
    assert!(flagged(r#"const PATH: &str = "M12 4L4 12 Z";"#).is_empty());
    assert!(flagged(r#"let s = format!("left: {}%", x);"#).is_empty());
    assert!(flagged(r#"let c = format!("hsl({} {}% 55%)", h, s);"#).is_empty());
    assert!(flagged(r#"let c = "card__value card__value--long";"#).is_empty());
    assert!(flagged(r#"format!("robot robot--{}", state)"#).is_empty());
    assert!(flagged(r#"let d = format!("H{x_end}V{y_min}");"#).is_empty());
    assert!(flagged(r#"let links = host.closest(".sidebar a");"#).is_empty());
}

/// A top-level test module holding a multi-line raw string ends at its closing brace, even with
/// trailing blanks; what follows is scanned again.
#[test]
fn a_top_level_test_module_ends_at_its_closing_brace() {
    let source = [
        "#[cfg(test)]",
        "mod tests {",
        "    const JSON: &[u8] = br#\"{",
        "  \"id\": 1",
        "}\"#;",
        "    fn t() { let _ = \"Test only words\"; }",
        "}  ",
        "fn shown() { let _ = \"Shown words\"; }",
    ]
    .join("\n");
    let source = source.as_str();
    assert_eq!(flagged(source), ["Shown words"]);
}

/// `d=` must not match the end of `checked=`, nor `id=` the end of `valid=`.
#[test]
fn tech_attrs_need_a_word_boundary() {
    assert!(flagged(r#"<path d="Some words here"/>"#).is_empty());
    assert!(flagged(r#"<div id="Some words here">"#).is_empty());
    assert_eq!(
        flagged(r#"<input checked="Some words here"/>"#),
        ["Some words here"]
    );
    assert_eq!(
        flagged(r#"<input valid="Some words here"/>"#),
        ["Some words here"]
    );
    // `attr:class=` is its own entry; a colon before any other name is not a boundary.
    assert!(flagged(r#"<div attr:class="Some words here">"#).is_empty());
    assert_eq!(
        flagged(r#"<input data:id="Some words here"/>"#),
        ["Some words here"]
    );
}

/// "error: {e}" reads like a sentence; "left: {}%" is CSS.
#[test]
fn a_word_colon_literal_is_not_always_css() {
    assert_eq!(flagged(r#"let m = format!("error: {e}");"#), ["error: {e}"]);
    assert!(flagged(r#"let s = format!("left: {}%", x);"#).is_empty());
    assert!(flagged(r#"let s = "display: none".to_owned();"#).is_empty());
}

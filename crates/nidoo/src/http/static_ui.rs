//! The embedded web UI (`crates/nidoo-web/dist`, built by `trunk build`), served for every
//! path the API does not claim. Unknown paths get `index.html` so the client-side router can
//! resolve them. A release `trunk build` also writes `.br` and `.gz` copies of the big assets
//! (`crates/nidoo-web/compress.sh`); they are sent to browsers that accept them.

use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri, header};
use axum::response::{Html, IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../nidoo-web/dist/"]
struct Dist;

const INDEX: &str = "index.html";
/// The pre-compressed copies, best first: `Accept-Encoding` coding and file suffix. Browsers
/// offer `br` only over HTTPS, so on a plain-HTTP LAN it is gzip that gets used.
const ENCODINGS: [(&str, &str); 2] = [("br", ".br"), ("gzip", ".gz")];
const NOT_BUILT: &str = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
    <title>Nidoo</title></head><body><p>The Nidoo web UI has not been built. \
    Run <code>trunk build --release</code> in <code>crates/nidoo-web</code>.</p></body></html>";

pub(super) async fn index(uri: Uri, request_headers: HeaderMap) -> Response {
    let path = uri.path().trim_start_matches('/');
    if !path.is_empty()
        && path != INDEX
        && let Some(file) = Dist::get(path)
    {
        let mime = mime_guess::from_path(path).first_or_octet_stream();
        let cache = if is_hashed(path) {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        };
        let accept = request_headers
            .get(header::ACCEPT_ENCODING)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let copies: Vec<_> = ENCODINGS
            .iter()
            .filter_map(|(coding, suffix)| Some((*coding, Dist::get(&format!("{path}{suffix}"))?)))
            .collect();
        let has_copies = !copies.is_empty();
        let chosen = copies
            .into_iter()
            .find(|(coding, _)| accepts(accept, coding));
        let (coding, body) = match chosen {
            Some((coding, copy)) => (Some(coding), copy.data),
            None => (None, file.data),
        };
        let mut response = file_response(mime.as_ref(), cache, body.into_owned());
        let headers = response.headers_mut();
        if has_copies {
            headers.insert(header::VARY, HeaderValue::from_static("accept-encoding"));
        }
        if let Some(coding) = coding {
            headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static(coding));
        }
        return response;
    }
    match Dist::get(INDEX) {
        Some(file) => file_response(
            "text/html; charset=utf-8",
            "no-cache",
            file.data.into_owned(),
        ),
        None => (StatusCode::SERVICE_UNAVAILABLE, Html(NOT_BUILT)).into_response(),
    }
}

fn file_response(content_type: &str, cache: &'static str, body: Vec<u8>) -> Response {
    let mut response = body.into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(content_type)
            .unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    response
}

/// Whether an `Accept-Encoding` value lists `coding` without `q=0`.
fn accepts(accept: &str, coding: &str) -> bool {
    accept.split(',').any(|item| {
        let mut params = item.split(';').map(str::trim);
        params
            .next()
            .is_some_and(|c| c.eq_ignore_ascii_case(coding))
            && params.all(|p| {
                p.strip_prefix("q=")
                    .and_then(|q| q.trim().parse::<f32>().ok())
                    .is_none_or(|q| q > 0.0)
            })
    })
}

/// Whether the file name carries a Trunk content hash: a `-` followed by at least eight hex
/// digits (`nidoo-web-836a2c4c498c2ec4_bg.wasm`, `main-c920ca43256fdcb9.css`).
fn is_hashed(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.split('-')
        .skip(1)
        .any(|rest| rest.bytes().take_while(u8::is_ascii_hexdigit).count() >= 8)
}

#[cfg(test)]
mod tests {
    use super::{accepts, is_hashed};

    #[test]
    fn trunk_hashes_are_detected() {
        assert!(is_hashed("nidoo-web-836a2c4c498c2ec4_bg.wasm"));
        assert!(is_hashed("main-c920ca43256fdcb9.css"));
        assert!(!is_hashed("index.html"));
        assert!(!is_hashed("nidoo-web.wasm"));
        assert!(!is_hashed("my-favicon.ico"));
    }

    #[test]
    fn accept_encoding_lists_the_coding_with_a_non_zero_quality() {
        assert!(accepts("gzip, deflate", "gzip"));
        assert!(accepts("gzip, deflate, br, zstd", "br"));
        assert!(accepts("br;q=0.5, GZIP", "gzip"));
        assert!(accepts("br ; q=1.0", "br"));
        assert!(!accepts("gzip, deflate", "br"));
        assert!(!accepts("br;q=0, gzip", "br"));
        assert!(!accepts("br; q=0.000", "br"));
        assert!(!accepts("", "gzip"));
        assert!(!accepts("x-gzip", "gzip"));
    }
}

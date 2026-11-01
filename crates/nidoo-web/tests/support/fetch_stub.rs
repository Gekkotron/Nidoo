//! A stand-in for `window.fetch` in wasm tests, shared by the test files that pull it in with
//! `#[path = "support/fetch_stub.rs"] mod fetch_stub;`.
//!
//! `FetchStub::install()` swaps the browser's `fetch` for a function that records every request
//! (method, path with query, body) and answers from canned replies keyed by method and URL:
//! `reply` gives a status and a JSON body, `hang` gives a promise that never settles (to look at
//! a screen while its write is in flight). A request nothing was set up for answers `404`.
//! Dropping the stub puts the original `fetch` back, so a test cannot leak it into the next.
//!
//! Each test file uses only part of this, hence the `dead_code` allowance.
#![allow(dead_code)]

use js_sys::{Function, Reflect};
use wasm_bindgen::JsValue;

/// One request the page made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recorded {
    pub method: String,
    /// The path and query, e.g. `/api/v1/ambiances/good_night`.
    pub url: String,
    /// The body as text, empty for a GET.
    pub body: String,
}

/// The installed stub; see the module docs.
pub struct FetchStub {
    original: JsValue,
    /// `{ log: [...], routes: { "METHOD url": { status, body, hang } } }`, read by the stub.
    state: JsValue,
}

const STUB_SOURCE: &str = r#"
    return async function (input, init) {
        const req = input instanceof Request ? input : new Request(input, init);
        const u = new URL(req.url);
        const url = u.pathname + u.search;
        const body = await req.clone().text();
        state.log.push({ method: req.method, url, body });
        const route = state.routes[req.method + " " + url];
        if (!route) {
            return new Response(JSON.stringify({ error: "no stub for " + req.method + " " + url }), {
                status: 404,
                headers: { "content-type": "application/json" },
            });
        }
        if (route.hang) {
            return new Promise(() => {});
        }
        if (route.delay) {
            await new Promise((resolve) => setTimeout(resolve, route.delay));
        }
        return new Response(route.body, {
            status: route.status,
            headers: { "content-type": "application/json" },
        });
    };
"#;

fn set(target: &JsValue, key: &str, value: &JsValue) {
    Reflect::set(target, &JsValue::from_str(key), value).unwrap();
}

fn get(target: &JsValue, key: &str) -> JsValue {
    Reflect::get(target, &JsValue::from_str(key)).unwrap()
}

impl FetchStub {
    /// Replaces `window.fetch`; answers `404` to everything until `reply` or `hang` say otherwise.
    pub fn install() -> Self {
        let global = js_sys::global();
        let state = js_sys::Object::new();
        set(&state, "log", &js_sys::Array::new());
        set(&state, "routes", &js_sys::Object::new());
        let make = Function::new_with_args("state", STUB_SOURCE);
        let stub = make.call1(&JsValue::NULL, &state).unwrap();
        let original = get(&global, "fetch");
        set(&global, "fetch", &stub);
        Self {
            original,
            state: state.into(),
        }
    }

    fn route(&self, method: &str, url: &str, status: u16, body: &str, hang: bool) {
        self.route_after(method, url, status, body, hang, 0);
    }

    fn route_after(
        &self,
        method: &str,
        url: &str,
        status: u16,
        body: &str,
        hang: bool,
        delay_ms: u32,
    ) {
        let entry = js_sys::Object::new();
        set(&entry, "delay", &JsValue::from(delay_ms));
        set(&entry, "status", &JsValue::from(status));
        set(&entry, "body", &JsValue::from_str(body));
        set(&entry, "hang", &JsValue::from(hang));
        set(
            &get(&self.state, "routes"),
            &format!("{method} {url}"),
            &entry,
        );
    }

    /// Answers `method url` with `status` and the JSON text `body`.
    pub fn reply(&self, method: &str, url: &str, status: u16, body: &str) {
        self.route(method, url, status, body, false);
    }

    /// Like `reply`, but answers only after `delay_ms`.
    pub fn reply_after(&self, method: &str, url: &str, status: u16, body: &str, delay_ms: u32) {
        self.route_after(method, url, status, body, false, delay_ms);
    }

    /// Makes `method url` never answer.
    pub fn hang(&self, method: &str, url: &str) {
        self.route(method, url, 0, "", true);
    }

    /// Every request seen so far, oldest first.
    pub fn requests(&self) -> Vec<Recorded> {
        js_sys::Array::from(&get(&self.state, "log"))
            .iter()
            .map(|r| {
                let text = |key| get(&r, key).as_string().unwrap_or_default();
                Recorded {
                    method: text("method"),
                    url: text("url"),
                    body: text("body"),
                }
            })
            .collect()
    }
}

impl Drop for FetchStub {
    fn drop(&mut self) {
        set(&js_sys::global(), "fetch", &self.original);
    }
}

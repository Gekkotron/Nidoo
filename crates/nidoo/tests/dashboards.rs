//! The `/api/v1/dashboards` routes, served on an ephemeral port over a hub with no broker.

use std::net::SocketAddr;
use std::sync::Arc;

use nidoo::config::Config;
use nidoo::http::router;
use nidoo::hub::Hub;
use nidoo_api::ConnectionStatus;
use nidoo_core::command::Publish;
use reqwest::StatusCode;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

struct Server {
    hub: Arc<Hub>,
    publisher: mpsc::Receiver<Publish>,
    addr: SocketAddr,
    client: reqwest::Client,
}

impl Server {
    async fn start() -> Server {
        let (tx, publisher) = mpsc::channel(64);
        let hub = Hub::new(&Config::default(), tx);
        hub.set_connection(ConnectionStatus::Online);
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(
            axum::serve(
                listener,
                router(Arc::clone(&hub), nidoo::setup::SetupCtx::inert()),
            )
            .into_future(),
        );
        Server {
            hub,
            publisher,
            addr,
            client: reqwest::Client::new(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}/api/v1{path}", self.addr)
    }

    async fn put(&self, id: &str, body: &Value) -> reqwest::Response {
        self.client
            .put(self.url(&format!("/dashboards/{id}")))
            .json(body)
            .send()
            .await
            .expect("put")
    }

    async fn create(&self, id: &str, order: u32) {
        let response = self.put(id, &dash(id, order)).await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    async fn delete(&self, id: &str) -> reqwest::Response {
        self.client
            .delete(self.url(&format!("/dashboards/{id}")))
            .send()
            .await
            .expect("delete")
    }

    async fn reorder(&self, order: &[&str]) -> reqwest::Response {
        self.client
            .post(self.url("/dashboards/order"))
            .json(&json!({ "order": order }))
            .send()
            .await
            .expect("reorder")
    }

    async fn ids(&self) -> Vec<String> {
        let list: Value = self
            .client
            .get(self.url("/dashboards"))
            .send()
            .await
            .expect("list")
            .json()
            .await
            .expect("json");
        list.as_array()
            .expect("array")
            .iter()
            .map(|v| v["dashboard"]["id"].as_str().expect("id").to_owned())
            .collect()
    }

    fn drain(&mut self) {
        while self.publisher.try_recv().is_ok() {}
    }
}

fn dash(id: &str, order: u32) -> Value {
    json!({ "id": id, "name": id, "order": order, "widgets": [] })
}

#[tokio::test]
async fn get_dashboards_returns_house_only_initially() {
    let server = Server::start().await;
    let list: Value = server
        .client
        .get(server.url("/dashboards"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let list = list.as_array().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["dashboard"]["id"], "house");
    assert_eq!(list[0]["synthesized"], true);
}

#[tokio::test]
async fn put_dashboard_creates() {
    let server = Server::start().await;
    let response = server.put("kitchen", &dash("kitchen", 1)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["id"], "kitchen");
    assert_eq!(server.ids().await, ["house", "kitchen"]);
    let got = server
        .client
        .get(server.url("/dashboards/kitchen"))
        .send()
        .await
        .unwrap();
    assert_eq!(got.status(), StatusCode::OK);
}

#[tokio::test]
async fn put_dashboard_rejects_house() {
    let server = Server::start().await;
    let response = server.put("house", &dash("house", 1)).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn put_house_with_malformed_body_still_returns_403() {
    let server = Server::start().await;
    let response = server
        .client
        .put(server.url("/dashboards/house"))
        .body(&b"not-json"[..])
        .send()
        .await
        .expect("put");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn put_dashboard_rejects_bad_span() {
    let server = Server::start().await;
    let body = json!({
        "id": "kitchen", "name": "kitchen", "order": 1,
        "widgets": [
            { "id": "w1", "kind": "light", "span": "2x2", "config": { "entity_id": "light.a" } }
        ]
    });
    let response = server.put("kitchen", &body).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(server.ids().await, ["house"]);
}

#[tokio::test]
async fn put_dashboard_normalizes_colliding_order() {
    let server = Server::start().await;
    server.create("a", 1).await;
    server.create("b", 2).await;
    let response = server.put("c", &dash("c", 1)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["order"], 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_puts_never_share_an_order() {
    let server = Server::start().await;
    let puts = (0..32).map(|n| {
        let hub = Arc::clone(&server.hub);
        tokio::spawn(async move {
            let body: nidoo_api::Dashboard =
                serde_json::from_value(dash(&format!("d{n}"), 0)).unwrap();
            hub.set_dashboard(body).await.expect("put").order
        })
    });
    let mut orders = Vec::new();
    for put in puts.collect::<Vec<_>>() {
        orders.push(put.await.unwrap());
    }
    orders.sort_unstable();
    assert_eq!(orders, (1..=32).collect::<Vec<u32>>());
}

#[tokio::test]
async fn delete_dashboard_removes() {
    let server = Server::start().await;
    server.create("a", 1).await;
    assert_eq!(server.delete("a").await.status(), StatusCode::NO_CONTENT);
    assert_eq!(server.ids().await, ["house"]);
    assert_eq!(server.delete("a").await.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_house_forbidden() {
    let server = Server::start().await;
    assert_eq!(server.delete("house").await.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn reorder_drifted_list_rejected() {
    let mut server = Server::start().await;
    for (id, order) in [("a", 1), ("b", 2), ("c", 3)] {
        server.create(id, order).await;
    }
    server.drain();
    let response = server.reorder(&["a", "b", "d"]).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(server.publisher.try_recv().is_err());
}

#[tokio::test]
async fn reorder_happy_path() {
    let server = Server::start().await;
    for (id, order) in [("a", 1), ("b", 2), ("c", 3)] {
        server.create(id, order).await;
    }
    let response = server.reorder(&["c", "a", "b"]).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(server.ids().await, ["house", "c", "a", "b"]);
}

#[tokio::test]
async fn order_route_shadows_id_route() {
    let server = Server::start().await;
    let response = server.reorder(&[]).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

/// An undecodable retained payload is stored as Invalid: GET hides it (404) but DELETE must
/// still clear it, so a corrupted dashboard can be unstuck.
#[tokio::test]
async fn delete_invalid_dashboard() {
    let mut server = Server::start().await;
    let topic = server.hub.dashboard_topic("corrupt");
    server.hub.ingest(&topic, b"not-json");
    let got = server
        .client
        .get(server.url("/dashboards/corrupt"))
        .send()
        .await
        .unwrap();
    assert_eq!(got.status(), StatusCode::NOT_FOUND);
    server.drain();
    assert_eq!(
        server.delete("corrupt").await.status(),
        StatusCode::NO_CONTENT
    );
    let published = server.publisher.try_recv().expect("empty retained publish");
    assert_eq!(published.topic, topic);
    assert!(published.payload.is_empty() && published.retain);
    assert_eq!(
        server.delete("corrupt").await.status(),
        StatusCode::NOT_FOUND
    );
}

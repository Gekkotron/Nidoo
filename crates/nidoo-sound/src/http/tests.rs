use super::*;

fn prefs(client_id: &str, name: &str, allowed: bool) -> PlayerPrefs {
    PlayerPrefs {
        client_id: client_id.to_owned(),
        name: name.to_owned(),
        volume: 45,
        speech_volume: 45,
        muted: false,
        allowed,
    }
}

fn player(prefs: PlayerPrefs, connected: bool) -> Published {
    Published::Player {
        short_id: short_id(&prefs.client_id),
        prefs,
        connected,
        address: None,
    }
}

#[test]
fn list_keeps_only_approved_sorted_by_name() {
    let s = Satellites::default();
    s.apply(&player(prefs("living-id", "Living room", true), true));
    s.apply(&player(prefs("kitchen-id", "Kitchen", true), true));
    s.apply(&player(prefs("garage-id", "Garage", false), true));
    let list = s.list();
    let names: Vec<&str> = list.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, ["Kitchen", "Living room"]);
    assert_eq!(list[0].id, short_id("kitchen-id"));
}

#[test]
fn a_name_given_in_nidoo_wins_and_sorts() {
    let s = Satellites::default();
    s.apply(&player(prefs("living-id", "Living room", true), true));
    s.apply(&player(prefs("board-id", "Satellite", true), true));
    s.set_name(&short_id("board-id"), Some("Bedroom".into()));
    let names: Vec<String> = s.list().into_iter().map(|v| v.name).collect();
    assert_eq!(names, ["Bedroom", "Living room"]);
    s.set_name(&short_id("board-id"), None);
    let names: Vec<String> = s.list().into_iter().map(|v| v.name).collect();
    assert_eq!(names, ["Living room", "Satellite"]);
}

#[test]
fn a_name_given_before_the_satellite_shows_up_is_kept() {
    let s = Satellites::default();
    s.set_name(&short_id("board-id"), Some("Bedroom".into()));
    s.apply(&player(prefs("board-id", "Satellite", true), true));
    assert_eq!(s.list()[0].name, "Bedroom");
}

#[test]
fn player_update_replaces_the_entry() {
    let s = Satellites::default();
    s.apply(&player(prefs("kitchen-id", "Kitchen", true), true));
    let mut p = prefs("kitchen-id", "Kitchen", true);
    p.volume = 70;
    p.muted = true;
    s.apply(&player(p, false));
    let list = s.list();
    assert_eq!(list.len(), 1);
    assert_eq!(
        (list[0].volume, list[0].muted, list[0].connected),
        (70, true, false)
    );
}

#[test]
fn removed_and_unapproved_leave_the_list() {
    let s = Satellites::default();
    let kitchen = prefs("kitchen-id", "Kitchen", true);
    s.apply(&player(kitchen.clone(), true));
    s.apply(&Published::Removed {
        prefs: kitchen.clone(),
    });
    assert!(s.list().is_empty());

    s.apply(&player(kitchen.clone(), true));
    assert!(s.is_approved(&short_id("kitchen-id")));
    let mut off = kitchen;
    off.allowed = false;
    s.apply(&player(off, true));
    assert!(s.list().is_empty());
    assert!(!s.is_approved(&short_id("kitchen-id")));
}

async fn serve(router: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://{addr}")
}

/// Kitchen approved, Garage not; returns the base URL and the control receiver.
async fn fixture() -> (String, mpsc::Receiver<Control>) {
    let s = Satellites::default();
    s.apply(&player(prefs("kitchen-id", "Kitchen", true), true));
    s.apply(&player(prefs("garage-id", "Garage", false), true));
    let (tx, rx) = mpsc::channel(8);
    (serve(router(s, tx)).await, rx)
}

#[tokio::test]
async fn get_players_lists_approved() {
    let (base, _rx) = fixture().await;
    let res = reqwest::get(format!("{base}/players")).await.unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body.as_array().unwrap().len(), 1);
    assert_eq!(body[0]["name"], "Kitchen");
    assert_eq!(body[0]["id"], short_id("kitchen-id"));
    assert_eq!(body[0]["volume"], 45);
    assert_eq!(body[0]["muted"], false);
    assert_eq!(body[0]["connected"], true);
}

async fn put(base: &str, id: &str, body: &str) -> reqwest::Response {
    reqwest::Client::new()
        .put(format!("{base}/players/{id}"))
        .header("content-type", "application/json")
        .body(body.to_owned())
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn put_volume_sends_control() {
    let (base, mut rx) = fixture().await;
    let id = short_id("kitchen-id");
    assert_eq!(put(&base, &id, r#"{"volume":100}"#).await.status(), 204);
    assert_eq!(
        rx.try_recv(),
        Ok(Control::SetVolume {
            short_id: id,
            volume: 100
        })
    );
}

#[tokio::test]
async fn put_both_sends_two_controls() {
    let (base, mut rx) = fixture().await;
    let id = short_id("kitchen-id");
    let res = put(&base, &id, r#"{"volume":20,"muted":true}"#).await;
    assert_eq!(res.status(), 204);
    assert_eq!(
        rx.try_recv(),
        Ok(Control::SetVolume {
            short_id: id.clone(),
            volume: 20
        })
    );
    assert_eq!(
        rx.try_recv(),
        Ok(Control::SetMute {
            short_id: id,
            muted: true
        })
    );
}

#[tokio::test]
async fn put_rejects_bad_bodies() {
    let (base, mut rx) = fixture().await;
    let id = short_id("kitchen-id");
    for body in [r#"{"volume":101}"#, "{}", r#"{"volume":"loud"}"#] {
        let res = put(&base, &id, body).await;
        assert_eq!(res.status(), 400, "{body}");
        let json: serde_json::Value = res.json().await.unwrap();
        assert!(json["error"].is_string(), "{body}");
    }
    assert!(rx.try_recv().is_err());
}

#[test]
fn waiting_lists_connected_unallowed_only() {
    let s = Satellites::default();
    s.apply(&player(prefs("A", "Alpha", true), true));
    s.apply(&player(prefs("B", "Bravo", false), true));
    s.apply(&player(prefs("C", "Charlie", false), false));
    assert_eq!(
        s.waiting(),
        [WaitingView {
            id: short_id("B"),
            name: "Bravo".into(),
            connected: true
        }]
    );
    let names: Vec<String> = s.list().into_iter().map(|v| v.name).collect();
    assert_eq!(names, ["Alpha"]);
}

#[tokio::test]
async fn get_waiting_over_http() {
    let (base, _rx) = fixture().await;
    let res = reqwest::get(format!("{base}/players/waiting"))
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(
        body,
        serde_json::json!([{"id": short_id("garage-id"), "name": "Garage", "connected": true}])
    );
}

async fn send(base: &str, method: reqwest::Method, path: &str) -> reqwest::Response {
    reqwest::Client::new()
        .request(method, format!("{base}{path}"))
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn allow_sends_set_allowed() {
    let (base, mut rx) = fixture().await;
    // Garage is waiting; Kitchen is already allowed: both get the command.
    for id in [short_id("garage-id"), short_id("kitchen-id")] {
        let res = send(
            &base,
            reqwest::Method::POST,
            &format!("/players/{id}/allow"),
        )
        .await;
        assert_eq!(res.status(), 204);
        assert_eq!(
            rx.try_recv(),
            Ok(Control::SetAllowed {
                short_id: id,
                allowed: true
            })
        );
    }
}

#[tokio::test]
async fn forget_sends_remove() {
    let (base, mut rx) = fixture().await;
    let id = short_id("kitchen-id");
    let res = send(&base, reqwest::Method::DELETE, &format!("/players/{id}")).await;
    assert_eq!(res.status(), 204);
    assert_eq!(rx.try_recv(), Ok(Control::Remove { short_id: id }));
}

#[tokio::test]
async fn allow_and_forget_unknown_is_404() {
    let (base, mut rx) = fixture().await;
    for (method, path) in [
        (reqwest::Method::POST, "/players/ffffffffffff/allow"),
        (reqwest::Method::DELETE, "/players/ffffffffffff"),
    ] {
        let res = send(&base, method, path).await;
        assert_eq!(res.status(), 404);
        let json: serde_json::Value = res.json().await.unwrap();
        assert_eq!(json["error"], "no speaker ffffffffffff");
    }
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn put_unknown_or_unapproved_is_404() {
    let (base, mut rx) = fixture().await;
    let res = put(&base, "ffffffffffff", r#"{"volume":10}"#).await;
    assert_eq!(res.status(), 404);
    let garage = short_id("garage-id");
    assert_eq!(put(&base, &garage, r#"{"volume":10}"#).await.status(), 404);
    assert!(rx.try_recv().is_err());
}

use super::*;

fn names(l: Lookup) -> Option<String> {
    match l {
        Lookup::Dashboard("d1") => Some("Wall tablet".into()),
        Lookup::Room("r1") => Some("Kitchen".into()),
        Lookup::Flow("f1") => Some("Night lights".into()),
        Lookup::Ambiance("good_night") => Some("Good night".into()),
        Lookup::Entity("light.a") => Some("Lamp".into()),
        Lookup::Room("Séjour") => Some("Séjour".into()),
        Lookup::Room("Salle de bain") => Some("Salle de bain".into()),
        _ => None,
    }
}
fn labels(c: &[Crumb]) -> Vec<&str> {
    c.iter().map(|c| c.label.as_str()).collect()
}

#[test]
fn sections() {
    assert_eq!(section_for("/", &[]), Section::Overview);
    assert_eq!(section_for("/dashboards/house", &[]), Section::Overview);
    assert_eq!(section_for("/rooms/r1", &[]), Section::Room("r1".into()));
    assert_eq!(section_for("/flows/f1", &[]), Section::Automations);
    assert_eq!(section_for("/group/g1", &[]), Section::Groups);
    assert_eq!(section_for("/groups/g1/edit", &[]), Section::Groups);
    assert_eq!(section_for("/alerts/a1", &[]), Section::Alerts);
    assert_eq!(section_for("/devices", &[]), Section::Settings);
    let back = vec!["/rooms/r1".to_string(), "/entity/light.a".to_string()];
    assert_eq!(
        section_for("/entity/light.a", &back),
        Section::Room("r1".into())
    );
    assert_eq!(section_for("/history/light.a", &[]), Section::None);
}

#[test]
fn breadcrumbs() {
    assert_eq!(
        labels(&crumbs("/rooms/r1", Lang::Fr, names)),
        ["Ma maison", "Kitchen"]
    );
    assert_eq!(
        labels(&crumbs("/flows/f1", Lang::Fr, names)),
        ["Ma maison", "Automatisations", "Night lights"]
    );
    assert_eq!(
        labels(&crumbs("/flows/new", Lang::En, names)),
        ["My home", "Automations", "New flow"]
    );
    assert_eq!(
        labels(&crumbs("/dashboards/d1", Lang::En, names)),
        ["My home", "Wall tablet"]
    );
    assert_eq!(
        labels(&crumbs("/history/light.a", Lang::En, names)),
        ["My home", "Lamp", "History"]
    );
    assert_eq!(
        labels(&crumbs("/rooms/zz", Lang::En, names)),
        ["My home", "zz"]
    );
    let c = crumbs("/flows/f1", Lang::En, names);
    assert_eq!(c[0].href.as_deref(), Some("/"));
    assert_eq!(c[1].href.as_deref(), Some("/flows"));
    assert_eq!(c[2].href, None);
}

#[test]
fn crumbs_for_ambiance_routes() {
    assert_eq!(section_for("/ambiances", &[]), Section::Ambiances);
    assert_eq!(section_for("/ambiances/about", &[]), Section::Ambiances);
    assert_eq!(
        section_for("/ambiances/good_night/report", &[]),
        Section::Ambiances
    );
    assert_eq!(
        labels(&crumbs("/ambiances", Lang::En, names)),
        ["My home", "My moods"]
    );
    assert_eq!(
        labels(&crumbs("/ambiances/new", Lang::En, names)),
        ["My home", "My moods", "New mood"]
    );
    assert_eq!(
        labels(&crumbs("/ambiances/about", Lang::Fr, names)),
        ["Ma maison", "Mes ambiances", "Comment ça marche"]
    );
    let c = crumbs("/ambiances/good_night", Lang::En, names);
    assert_eq!(labels(&c), ["My home", "My moods", "Good night"]);
    assert_eq!(c[1].href.as_deref(), Some("/ambiances"));
    assert_eq!(c[2].href, None);
    let c = crumbs("/ambiances/good_night/report", Lang::En, names);
    assert_eq!(
        labels(&c),
        ["My home", "My moods", "Good night", "Last run"]
    );
    assert_eq!(c[2].href.as_deref(), Some("/ambiances/good_night"));
    assert_eq!(c[3].href, None);
    // An unknown id reads as the id itself.
    assert_eq!(
        labels(&crumbs("/ambiances/zz", Lang::En, names)),
        ["My home", "My moods", "zz"]
    );
}

#[test]
fn search_route_crumb_and_section() {
    assert_eq!(
        labels(&crumbs("/search", Lang::En, names)),
        ["My home", "Search"]
    );
    assert_eq!(
        labels(&crumbs("/search?q=lamp", Lang::Fr, names)),
        ["Ma maison", "Recherche"]
    );
    assert_eq!(section_for("/search", &[]), Section::Overview);
}

#[test]
fn assistant_route_crumb_and_section() {
    assert_eq!(
        labels(&crumbs("/assistant", Lang::En, names)),
        ["My home", "Assistant"]
    );
    assert_eq!(section_for("/assistant", &[]), Section::Assistant);
}

#[test]
fn weather_route_crumb_and_section() {
    assert_eq!(section_for("/weather", &[]), Section::Overview);
    let c = crumbs("/weather", Lang::En, names);
    assert_eq!(labels(&c), ["My home", "Weather"]);
    assert_eq!(c[0].href.as_deref(), Some("/"));
    assert_eq!(c[1].href, None);
}

#[test]
fn solar_route_crumb_and_section() {
    assert_eq!(section_for("/solar", &[]), Section::Overview);
    let c = crumbs("/solar", Lang::En, names);
    assert_eq!(labels(&c), ["My home", "Solar"]);
    assert_eq!(c[0].href.as_deref(), Some("/"));
    assert_eq!(c[1].href, None);
}

#[test]
fn extra_crumbs() {
    assert_eq!(labels(&crumbs("/", Lang::En, names)), ["My home"]);
    assert_eq!(crumbs("/", Lang::En, names)[0].href, None);
    assert_eq!(
        labels(&crumbs("/alerts/zz", Lang::Fr, names)),
        ["Ma maison", "Alertes", "Alerte"]
    );
    let c = crumbs("/groups/g1/edit", Lang::En, names);
    assert_eq!(labels(&c), ["My home", "Groups", "g1", "Edit group"]);
    assert_eq!(c[2].href.as_deref(), Some("/group/g1"));
}

#[test]
fn settings_crumbs() {
    assert_eq!(
        labels(&crumbs("/settings", Lang::En, names)),
        ["My home", "Settings"]
    );
    let c = crumbs("/settings/zigbee2mqtt", Lang::Fr, names);
    assert_eq!(labels(&c), ["Ma maison", "Réglages", "Zigbee2MQTT"]);
    assert_eq!(c[1].href.as_deref(), Some("/settings"));
    assert_eq!(
        labels(&crumbs("/settings/about", Lang::Fr, names)),
        ["Ma maison", "Réglages", "À propos"]
    );
    assert_eq!(
        labels(&crumbs("/settings/assistant-voice", Lang::Fr, names)),
        ["Ma maison", "Réglages", "Voix"]
    );
    assert_eq!(
        labels(&crumbs("/settings/assistant-listening", Lang::En, names)),
        ["My home", "Settings", "Listening"]
    );
    assert_eq!(
        labels(&crumbs("/settings/nope", Lang::En, names)),
        ["My home", "Settings"]
    );
    assert_eq!(section_for("/settings/mcp", &[]), Section::Settings);
}

/// Room ids are room names; the address bar keeps them percent-encoded.
#[test]
fn encoded_room_ids() {
    assert_eq!(
        section_for("/rooms/S%C3%A9jour", &[]),
        Section::Room("Séjour".into())
    );
    assert_eq!(
        section_for("/rooms/Salle%20de%20bain", &[]),
        Section::Room("Salle de bain".into())
    );
    let back = vec!["/rooms/S%C3%A9jour".to_string()];
    assert_eq!(
        section_for("/entity/light.a", &back),
        Section::Room("Séjour".into())
    );
    assert_eq!(
        labels(&crumbs("/rooms/S%C3%A9jour", Lang::Fr, names)),
        ["Ma maison", "Séjour"]
    );
    assert_eq!(
        labels(&crumbs("/rooms/Salle%20de%20bain", Lang::En, names)),
        ["My home", "Salle de bain"]
    );
    // A stray "%" that is no escape stays as typed.
    assert_eq!(section_for("/rooms/50%", &[]), Section::Room("50%".into()));
}

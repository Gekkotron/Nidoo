use super::*;

#[test]
fn eco_sentences() {
    assert_eq!(fan_choice(Lang::En, "low", 2, 4), "low (on 2 of 4 units)");
    assert_eq!(
        fan_choice(Lang::Fr, "low", 2, 4),
        "low (sur 2 des 4 appareils)"
    );
    assert_eq!(
        eco_until_line(Lang::En, "Sun 1 Nov 00:00"),
        "Eco until Sun 1 Nov 00:00"
    );
    assert_eq!(
        eco_until_line(Lang::Fr, "dim. 1 nov. 00:00"),
        "Éco jusqu'au dim. 1 nov. 00:00"
    );
}

/// Same word in both languages.
const SAME: &[ClimateText] = &[
    ClimateText::ZoneActive,
    ClimateText::UnitMin,
    ClimateText::Base,
    ClimateText::UnitHours,
    ClimateText::SrcBoost,
    ClimateText::DriveThermostat,
];

#[test]
fn every_word_has_both_languages() {
    assert_table(ClimateText::ALL, climate_text, SAME);
}

#[test]
fn learning_progress_agrees_with_the_count() {
    assert_eq!(
        learning_progress(Lang::En, 6, 10),
        "Learning: 6 of 10 days so far"
    );
    assert_eq!(
        learning_progress(Lang::Fr, 6, 10),
        "Apprentissage\u{A0}: 6 jours sur 10 pour l'instant"
    );
    assert_eq!(
        learning_progress(Lang::Fr, 1, 10),
        "Apprentissage\u{A0}: 1 jour sur 10 pour l'instant"
    );
    assert_eq!(
        learning_progress(Lang::Fr, 0, 10),
        "Apprentissage\u{A0}: 0 jour sur 10 pour l'instant"
    );
}

#[test]
fn sentences_read_in_both_languages() {
    assert_eq!(
        hold_until(Lang::En, "Heat 21°", "07:00", ClimateText::SetHere),
        "Heat 21° until 07:00 · set here"
    );
    assert_eq!(
        hold_until(Lang::Fr, "Chauffage 21°", "07:00", ClimateText::SetOnDevice),
        "Chauffage 21° jusqu'à 07:00 · réglé sur l'appareil"
    );
    assert_eq!(
        now_with_age(Lang::Fr, "à l'instant"),
        "Maintenant · à l'instant"
    );
    assert_eq!(automatic_with(Lang::Fr, "Salon"), "Automatique · Salon");
    assert_eq!(
        could_not_save(Lang::Fr, "refusé"),
        "Enregistrement impossible\u{A0}: refusé"
    );
    assert_eq!(heating_power(Lang::En, 43), "Heating · 43\u{A0}%");
    assert_eq!(heating_power(Lang::Fr, 43), "En chauffe · 43\u{A0}%");
}

/// Every reason shape the engine writes, exactly as decide.rs and feedback.rs format it.
#[test]
fn engine_reasons_in_french() {
    let n = "\u{a0}:";
    let cases: Vec<(&str, String)> = vec![
        ("Zone is off", "Pièce arrêtée".into()),
        (
            "Frost protection: 6.5 °C < 7.0 °C",
            format!("Protection antigel{n} 6,5 °C < 7,0 °C"),
        ),
        (
            "Frost protection: 8.0 °C, holding until 9.0 °C",
            format!("Protection antigel{n} 8,0 °C, maintenu jusqu'à 9,0 °C"),
        ),
        (
            "Overheat protection: 31.0 °C > 30.0 °C",
            format!("Protection surchauffe{n} 31,0 °C > 30,0 °C"),
        ),
        (
            "Overheat protection: 29.8 °C, holding until 29.5 °C",
            format!("Protection surchauffe{n} 29,8 °C, maintenu jusqu'à 29,5 °C"),
        ),
        (
            "Overcool protection: 14.0 °C < 15.0 °C",
            format!("Protection surrefroidissement{n} 14,0 °C < 15,0 °C"),
        ),
        (
            "Overcool protection: 15.2 °C, holding until 16.5 °C",
            format!("Protection surrefroidissement{n} 15,2 °C, maintenu jusqu'à 16,5 °C"),
        ),
        (
            "Overcool protection: 14.0 °C < 15.0 °C; cannot stop cooling: ac1, ac2",
            format!(
                "Protection surrefroidissement{n} 14,0 °C < 15,0 °C; impossible d'arrêter la climatisation{n} ac1, ac2"
            ),
        ),
        (
            "Reading is stale (last 18.0 °C): not acting until approved rules exist",
            format!(
                "Mesure périmée (dernière valeur 18,0 °C){n} aucune action tant qu'il n'y a pas de règles approuvées"
            ),
        ),
        (
            "Reading is stale: no rules to fall back on, not acting",
            format!("Mesure périmée{n} aucune règle de repli, aucune action"),
        ),
        (
            "No temperature reading: holding the safer of the current target and base",
            format!(
                "Aucune mesure de température{n} on garde la plus prudente de la consigne actuelle et de la base"
            ),
        ),
        (
            "Safety cleared: restoring previous settings",
            format!("Sécurité levée{n} réglages précédents rétablis"),
        ),
        (
            "Waiting for approved rules",
            "En attente de règles approuvées".into(),
        ),
        (
            "Waiting for approved rules; would set 19.0 °C",
            "En attente de règles approuvées; réglerait sur 19,0 °C".into(),
        ),
        (
            "Waiting for approved rules; would set off",
            "En attente de règles approuvées; arrêterait".into(),
        ),
        (
            "Boost to 22.0 °C until 11:00",
            "Boost à 22,0 °C jusqu'à 11:00".into(),
        ),
        (
            "Override to 20.5 °C until 11:00",
            "Dérogation à 20,5 °C jusqu'à 11:00".into(),
        ),
        (
            "Override to off until 11:00",
            "Dérogation à arrêt jusqu'à 11:00".into(),
        ),
        (
            "No heating or cooling season",
            "Hors saison de chauffage ou de climatisation".into(),
        ),
        (
            "No rules for heat mode",
            "Aucune règle en mode chauffage".into(),
        ),
        (
            "No rules for cool mode",
            "Aucune règle en mode climatisation".into(),
        ),
        ("Holiday: 7.0 °C", format!("Vacances{n} 7,0 °C")),
        ("Holiday: off", format!("Vacances{n} arrêt")),
        (
            "Eco until Wed 21 Oct 11:00: 16.0 °C",
            format!("Éco jusqu'au mer. 21 oct. 11:00{n} 16,0 °C"),
        ),
        (
            "Eco until Wed 21 Oct 11:00 (nobody home): 16.0 °C",
            format!("Éco jusqu'au mer. 21 oct. 11:00 (personne à la maison){n} 16,0 °C"),
        ),
        (
            "Eco until Wed 21 Oct 11:00: 14.0 °C; schedule already lower",
            format!(
                "Éco jusqu'au mer. 21 oct. 11:00{n} 14,0 °C; la programmation est déjà plus basse"
            ),
        ),
        (
            "Eco until Sun 1 Nov 00:00: 26.0 °C; schedule already higher",
            format!(
                "Éco jusqu'au dim. 1 nov. 00:00{n} 26,0 °C; la programmation est déjà plus haute"
            ),
        ),
        (
            "Nobody home: eco 16.0 °C",
            format!("Personne à la maison{n} éco 16,0 °C"),
        ),
        (
            "Nobody home: eco off",
            format!("Personne à la maison{n} éco arrêt"),
        ),
        (
            "Schedule block 2: 19.0 °C",
            format!("Plage 2 de la programmation{n} 19,0 °C"),
        ),
        (
            "Schedule base: 18.0 °C",
            format!("Base de la programmation{n} 18,0 °C"),
        ),
        (
            "No one in the room for 30 min: 18.0 °C (19.0 − 1.0)",
            format!("Personne dans la pièce depuis 30 min{n} 18,0 °C (19,0 − 1,0)"),
        ),
        (
            "No one in the room for 30 min: 26.0 °C (base)",
            format!("Personne dans la pièce depuis 30 min{n} 26,0 °C (base)"),
        ),
        (
            "No one in the room for 30 min: 7.0 °C (frost)",
            format!("Personne dans la pièce depuis 30 min{n} 7,0 °C (antigel)"),
        ),
        (
            "trv2 held until 11:00",
            "trv2 maintenu jusqu'à 11:00".into(),
        ),
        (
            "trv held as it is until 11:00",
            "trv laissé tel quel jusqu'à 11:00".into(),
        ),
        (
            "the room thermometer has been silent for over 6 h, using the devices' own readings",
            "le thermomètre de la pièce est muet depuis plus de 6 h, mesures des appareils utilisées"
                .into(),
        ),
        (
            "the room thermometer has been silent for over 6 h 10 min, using the devices' own readings",
            "le thermomètre de la pièce est muet depuis plus de 6 h 10 min, mesures des appareils utilisées"
                .into(),
        ),
        (
            "room above 19.5 °C: AC 18.5 °C (-0.5)",
            format!("pièce au-dessus de 19,5 °C{n} clim 18,5 °C (-0,5)"),
        ),
        (
            "room below 20.5 °C: AC 21.0 °C",
            format!("pièce en dessous de 20,5 °C{n} clim 21,0 °C"),
        ),
        (
            "room near 19.0 °C: AC 17.5 °C (-1.5)",
            format!("pièce proche de 19,0 °C{n} clim 17,5 °C (-1,5)"),
        ),
        (
            "room below 20.5 °C: AC 21.5 °C (+0.5)",
            format!("pièce en dessous de 20,5 °C{n} clim 21,5 °C (+0,5)"),
        ),
        (
            "no room thermometer",
            "pas de thermomètre dans la pièce".into(),
        ),
        (
            "room thermometer has not reported: no correction",
            format!("le thermomètre de la pièce n'a encore rien envoyé{n} aucune correction"),
        ),
        (
            "room thermometer silent: no correction",
            format!("thermomètre de la pièce muet{n} aucune correction"),
        ),
        (
            "all units held: correction paused",
            format!("tous les appareils sont maintenus{n} correction en pause"),
        ),
    ];
    for (raw, want) in cases {
        assert_eq!(reason_text(Lang::Fr, raw), want, "{raw}");
    }
}

#[test]
fn reason_english_passes_through() {
    let raw = "Schedule block 2: 19.0 °C; trv2 held until 11:00";
    assert_eq!(reason_text(Lang::En, raw), raw);
}

#[test]
fn reason_unknown_segments_stay_as_they_are() {
    assert_eq!(
        reason_text(Lang::Fr, "Something new: 1.5 x"),
        "Something new: 1.5 x"
    );
    assert_eq!(reason_text(Lang::Fr, ""), "");
    assert_eq!(
        reason_text(Lang::Fr, "Schedule block x: 19.0 °C"),
        "Schedule block x: 19.0 °C"
    );
    assert_eq!(
        reason_text(Lang::Fr, "Zone is off; whatever"),
        "Pièce arrêtée; whatever"
    );
}

#[test]
fn reason_segments_rejoin_with_semicolons() {
    assert_eq!(
        reason_text(
            Lang::Fr,
            "Schedule block 1: 19.0 °C; room above 19.5 °C: AC 18.5 °C (-0.5); trv held until 11:00"
        ),
        "Plage 1 de la programmation\u{a0}: 19,0 °C; pièce au-dessus de 19,5 °C\u{a0}: clim 18,5 °C (-0,5); trv maintenu jusqu'à 11:00"
    );
}

#[test]
fn reason_colons_take_a_non_breaking_space() {
    let out = reason_text(Lang::Fr, "Holiday: 7.0 °C");
    assert!(out.contains("\u{a0}:"));
    assert!(!out.contains(" :"));
    // The clock keeps its bare colon.
    assert_eq!(
        reason_text(Lang::Fr, "Boost to 22.0 °C until 11:00"),
        "Boost à 22,0 °C jusqu'à 11:00"
    );
}

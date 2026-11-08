//! The browser tests, built as one wasm binary so the run links once and starts Chrome once.
//! Each `tests/<name>.rs` stays its own module; a new test file needs a line here. A test that
//! clicks a link or presses a page-wide key needs a clean page instead: give its file a `[[test]]`
//! of its own in `Cargo.toml`, like `search_slash`.
#![cfg(target_arch = "wasm32")]

wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

mod ambiance_editor;
mod ambiances_page;
mod assist_redirect;
mod assist_speak;
mod assistant;
mod camera_widget;
mod card_frame;
mod climate_layout;
mod climate_rooms;
mod climate_timeline;
mod cover;
mod dashboard_screen;
mod energy;
mod energy_preview;
mod entity_picker;
mod flow_canvas;
mod flows_list;
mod generic_card;
mod group_card;
mod group_wizard;
mod groups_screens;
mod home_ambiances;
mod home_header;
mod home_settings_panel;
mod lang_toggle;
mod lock_card;
mod network_page;
mod network_panel;
mod popup;
mod rich_cards;
mod room_back;
mod rooms;
mod satellite_card;
mod search_back_scroll;
mod search_page;
mod security;
mod settings_updates;
mod setup_boot;
mod setup_connection;
mod setup_devices;
mod setup_done;
mod setup_energy;
mod setup_keys;
mod setup_services;
mod setup_welcome;
mod sheet;
mod shell;
mod smoke;
mod solar_card;
mod solar_page;
mod sound_page;
mod speech_input;
mod weather_page;
mod web_errors;
mod wheel;

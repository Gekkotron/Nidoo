//! Every hub integration test, built as one binary so the run links the hub once instead of
//! once per file. Each `tests/<name>.rs` stays its own module; a new test file needs a line here.

mod common;

mod alarm_hub;
mod ambiances;
mod assist_db;
mod assist_hub;
mod backup_archive;
mod backup_db;
mod backup_ftp;
mod backup_job;
mod backup_restore;
mod climate_hub;
mod config;
mod dashboards;
mod engine_sun;
mod flow_labels;
mod flows;
mod history_db;
mod home_settings;
mod http;
mod http_flows_config;
mod http_static_ui;
mod hub;
mod init_bundled;
mod mcp;
mod mcp_stdio;
mod network;
mod settings_db;
mod setup_http;
mod solar_hub;
mod solar_poller;
mod sound_http;
mod stt_http;
mod stt_models;
mod system;
mod tts_download;
mod tts_hf;
mod tts_http;
mod tts_real;
mod tts_speech;
mod tts_voices;
mod weather_client;
mod weather_hub;
mod weather_poller;

#![forbid(unsafe_code)]
//! Wire types shared by the hub and the WebAssembly UI: views over the registry, the
//! snapshot, WebSocket events and REST bodies. Pure data, no I/O, no clock.

mod alerts;
mod ambiances;
mod cameras;
mod dashboards;
mod energy;
mod events;
mod flows;
mod groups;
mod history;
mod home;
mod network;
pub mod search;
mod setup;
mod snapshot;
mod solar;
mod sound;
pub mod system;
mod tts;
mod views;
mod virtual_entities;
mod weather;

pub use alerts::{AlertAction, AlertChannel, AlertSeverity, AlertView, parse_history};
pub use ambiances::{
    AMBIANCE_ICONS, ActionReport, Ambiance, AmbianceAction, AmbianceView, ApplyReport, Category,
    CategoryAction, CategoryPlan, ClimateAction, DeviceAction, FlowAction, FlowDo, Outcome,
    Problem, Target, already_there, category_members, is_valid_ambiance_id, plan_category,
    validate_ambiance,
};
pub use cameras::{CameraInfo, CamerasView};
pub use dashboards::{
    Dashboard, DashboardReorder, DashboardView, HOUSE_ID, HOUSE_NAME, MAX_CAMERAS, Span, Widget,
    WidgetConfig, WidgetKind, allowed_spans, decode_dashboard,
};
pub use energy::{energy_entities, energy_entity};
pub use events::{ApiError, CommandResponse, ConnectionStatus, OverridePatch, WsEvent};
pub use flows::{
    FlowView, FlowWriteOutcome, FlowsConfigView, RunStatus, RunView, SchedulePreview, StepOutcome,
    StepView, TriggerInfo,
};
pub use groups::{GroupMemberView, GroupSpec, GroupView};
pub use history::{
    EnergyKind, GridDay, GridSlot, GridTotals, HistoryDaily, HistoryRange, HistorySeries,
    HistoryToday, LastChange, LastChangeItem,
};
pub use home::{EnergyView, HomeSettings, OffPeakDevice, OffPeakEntry, OffPeakWindow};
pub use network::{NetworkWatchSpec, NetworkWatchView, WatchState, WatchStatus};
pub use setup::{
    DatabaseInput, DatabaseView, FrigateInput, FrigateView, GeoPlace, Location, MqttInput,
    MqttView, PushInput, PushView, Secret, SecretView, SetupStatus, SetupStep, SolcastInput,
    SolcastView, TestResult,
};
pub use snapshot::{Snapshot, area_views, device_views, entity_views, snapshot};
pub use solar::{
    SolarDay, SolarForecastDay, SolarHistory, SolarNow, SolarPeak, SolarPeriod, SolarSite,
    SolarSpan, SolarToday, SolarView,
};
pub use sound::{SoundSpeakReply, SoundSpeakRequest};
pub use system::{
    CpuView, DiskView, MemView, NotifyInput, RestartView, ServiceState, ServiceView, StopKind,
    SystemService, SystemView,
};
pub use tts::{
    AvailableLanguage, DEFAULT_VOICE, DownloadRequest, DownloadView, EngineState, HouseRequest,
    InspectRequest, JobState, LanguageRequest, LanguageView, ModelSource, ModelView, RepoListing,
    SpeechRequest, SpeechTicket, TokenInput, TtsStatus, UploadSession, VoiceSource, VoiceView,
    VoicesQuery, VoicesView, WeightFile,
};
pub use views::{
    AreaView, BridgeView, DeviceView, EntityView, IgnoredView, UpdateView, mask_secret,
};
pub use virtual_entities::{VirtualKind, VirtualSpec};
pub use weather::{
    WEATHER_DEVICE_NAME, WeatherCondition, WeatherCurrent, WeatherDay, WeatherPlace, WeatherToday,
    WeatherView, weather_entity_name,
};

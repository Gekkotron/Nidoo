//! The menu-bar icon and its menu, on the main thread (AppKit's rule).

use std::sync::Arc;
use std::time::{Duration, Instant};

use nidoo_satellite::{Control, Handles, UiState};
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
use tokio::sync::mpsc;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder};

use crate::icon::icon;
use crate::login;
use crate::models::{Download, Models};

enum UserEvent {
    Ui(UiState),
    Menu(MenuEvent),
}

struct MenuBar {
    tray: Option<TrayIcon>,
    status: MenuItem,
    listening: CheckMenuItem,
    set_key: MenuItem,
    train: MenuItem,
    login: CheckMenuItem,
    quit: MenuItem,
    last: UiState,
    models: Arc<Models>,
}

impl MenuBar {
    fn new(ui: UiState, models: Arc<Models>) -> Self {
        Self {
            tray: None,
            status: MenuItem::new("", false, None),
            listening: CheckMenuItem::new("Listening", true, ui.listening, None),
            set_key: MenuItem::new("Set voice key from the clipboard", true, None),
            train: MenuItem::new("Train Hey Jarvis…", true, None),
            login: CheckMenuItem::new("Start at login", true, login::enabled(), None),
            quit: MenuItem::new("Quit Nidoo Satellite", true, None),
            last: ui,
            models,
        }
    }

    /// Builds the tray icon; AppKit wants the event loop running first.
    fn show(&mut self) {
        let menu = Menu::new();
        let built = menu
            .append_items(&[
                &self.status,
                &PredefinedMenuItem::separator(),
                &self.listening,
                &self.set_key,
                &self.train,
                &PredefinedMenuItem::separator(),
                &self.login,
                &PredefinedMenuItem::separator(),
                &self.quit,
            ])
            .map_err(anyhow::Error::from)
            .and_then(|()| {
                Ok(TrayIconBuilder::new()
                    .with_menu(Box::new(menu))
                    .with_icon_templated(icon(self.last.icon))
                    .with_tooltip("Nidoo Satellite")
                    .build()?)
            });
        match built {
            Ok(tray) => self.tray = Some(tray),
            Err(e) => tracing::error!(error = %e, "menu-bar icon"),
        }
        self.refresh();
    }

    fn apply(&mut self, ui: UiState) {
        if ui.icon != self.last.icon
            && let Some(tray) = &self.tray
        {
            let _ = tray.set_icon_templated(Some(icon(ui.icon)));
        }
        self.last = ui;
        self.refresh();
    }

    /// Status line and item texts (also once a second, for the model download).
    fn refresh(&mut self) {
        let status = match (self.models.state(), self.last.training) {
            (Download::Pending, false) => format!("{} · downloading Hey Jarvis…", self.last.status),
            (Download::Failed(_), false) => format!(
                "{} · Hey Jarvis download failed, retrying",
                self.last.status
            ),
            _ => self.last.status.clone(),
        };
        if self.status.text() != status {
            self.status.set_text(status);
        }
        self.listening.set_checked(self.last.listening);
        let train = if self.last.training {
            "Cancel training"
        } else {
            "Train Hey Jarvis…"
        };
        if self.train.text() != train {
            self.train.set_text(train);
        }
    }

    /// Acts on a click; true for Quit.
    fn clicked(&mut self, e: &MenuEvent, control: &mpsc::Sender<Control>) -> bool {
        let send = |c: Control| {
            let _ = control.blocking_send(c);
        };
        if e.id == *self.listening.id() {
            send(Control::SetListening(self.listening.is_checked()));
        } else if e.id == *self.set_key.id() {
            let text = arboard::Clipboard::new()
                .and_then(|mut c| c.get_text())
                .unwrap_or_default();
            send(Control::SetVoiceKey(text));
        } else if e.id == *self.train.id() {
            send(if self.last.training {
                Control::CancelTraining
            } else {
                Control::TrainWake
            });
        } else if e.id == *self.login.id() {
            if let Err(e) = login::set(self.login.is_checked()) {
                tracing::warn!(error = %format!("{e:#}"), "start at login");
            }
            self.login.set_checked(login::enabled());
        } else if e.id == *self.quit.id() {
            return true;
        }
        false
    }
}

/// Runs the menu bar until Quit.
pub fn run(rt: tokio::runtime::Runtime, handles: Handles, models: Arc<Models>) -> ! {
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    event_loop.set_activation_policy(ActivationPolicy::Accessory);
    let proxy = event_loop.create_proxy();
    let p = proxy.clone();
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        let _ = p.send_event(UserEvent::Menu(e));
    }));
    let mut ui = handles.ui.clone();
    rt.spawn(async move {
        while ui.changed().await.is_ok() {
            let state = ui.borrow_and_update().clone();
            if proxy.send_event(UserEvent::Ui(state)).is_err() {
                break;
            }
        }
    });

    let mut bar = MenuBar::new(handles.ui.borrow().clone(), models);
    let control = handles.control.clone();
    let mut done = Some(handles.done);
    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_secs(1));
        match event {
            Event::NewEvents(StartCause::Init) => bar.show(),
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) => bar.refresh(),
            Event::UserEvent(UserEvent::Ui(state)) => bar.apply(state),
            Event::UserEvent(UserEvent::Menu(e)) if bar.clicked(&e, &control) => {
                // Leave the house: the runtime sends End, closes the mic and the
                // Sendspin port before we go.
                let _ = control.blocking_send(Control::Quit);
                if let Some(done) = done.take() {
                    let _ = rt.block_on(tokio::time::timeout(Duration::from_secs(2), done));
                }
                bar.tray = None;
                *flow = ControlFlow::Exit;
            }
            _ => {}
        }
    })
}

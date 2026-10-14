//! 사용량을 뒤에서 주기적으로 가져와 보관하고, 메뉴 막대와 창에 알린다.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};

use crate::menu_bar;
use crate::providers::{self, Provider, ProviderReport};
use crate::settings::SettingsStore;

/// 새 값이 나올 때마다 창으로 보내는 이벤트 이름
pub const USAGE_UPDATED: &str = "usage-updated";

/// 창을 열었을 때 마지막 갱신이 이보다 오래됐으면 바로 다시 가져온다.
const STALE_ON_OPEN: Duration = Duration::from_secs(10);

#[derive(Default)]
struct Latest {
    reports: Vec<ProviderReport>,
    fetched_at: Option<Instant>,
}

pub struct Refresher {
    latest: Mutex<Latest>,
    wake: Sender<()>,
}

impl Refresher {
    /// 갱신 스레드를 깨우는 쪽과, 스레드가 기다릴 수신기를 만든다.
    pub fn new() -> (Self, Receiver<()>) {
        let (wake, wake_rx) = mpsc::channel();
        let refresher = Refresher {
            latest: Mutex::new(Latest::default()),
            wake,
        };
        (refresher, wake_rx)
    }

    /// 가장 최근에 가져온 값. 아직 한 번도 못 가져왔으면 비어 있다.
    pub fn reports(&self) -> Vec<ProviderReport> {
        self.latest.lock().unwrap().reports.clone()
    }

    pub fn refresh_if_stale(&self) {
        let fetched_at = self.latest.lock().unwrap().fetched_at;
        if fetched_at.is_none_or(|at| at.elapsed() >= STALE_ON_OPEN) {
            let _ = self.wake.send(());
        }
    }
}

/// 갱신 스레드를 시작한다. `Refresher`, `SettingsStore`, `MenuBarState` 가 등록된 뒤에 불러야 한다.
pub fn spawn(app: AppHandle, providers: Vec<Box<dyn Provider>>, wake_rx: Receiver<()>) {
    thread::spawn(move || loop {
        let reports = providers::fetch_all(&providers);
        {
            let refresher = app.state::<Refresher>();
            let mut latest = refresher.latest.lock().unwrap();
            latest.reports = reports.clone();
            latest.fetched_at = Some(Instant::now());
        }
        menu_bar::update(&app, &reports);
        let _ = app.emit(USAGE_UPDATED, &reports);

        let interval = app.state::<SettingsStore>().get().refresh_interval();
        if let Err(RecvTimeoutError::Disconnected) = wake_rx.recv_timeout(interval) {
            break;
        }
        // 기다리는 동안 쌓인 요청은 한 번으로 친다.
        while wake_rx.try_recv().is_ok() {}
    });
}

//! 사용자 설정. 앱 설정 폴더의 `settings.json` 에 저장한다.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};

const MIN_REFRESH_INTERVAL_SECS: u64 = 30;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// 메뉴 막대 아이콘 옆에 사용률 숫자를 표시할지
    pub show_percent_in_menu_bar: bool,
    pub refresh_interval_secs: u64,
    /// 창에 보여줄 서비스 id
    pub page: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            show_percent_in_menu_bar: true,
            refresh_interval_secs: 60,
            page: "claude".to_string(),
        }
    }
}

impl Settings {
    pub fn refresh_interval(&self) -> Duration {
        Duration::from_secs(self.refresh_interval_secs.max(MIN_REFRESH_INTERVAL_SECS))
    }
}

pub struct SettingsStore {
    /// 설정 폴더를 알 수 없으면 None. 그때는 저장하지 않고 메모리에서만 유지한다.
    path: Option<PathBuf>,
    current: Mutex<Settings>,
}

impl SettingsStore {
    /// 파일이 없거나 읽을 수 없으면 기본값으로 시작한다.
    pub fn load(path: Option<PathBuf>) -> Self {
        let current = path
            .as_ref()
            .and_then(|path| fs::read_to_string(path).ok())
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
        SettingsStore {
            path,
            current: Mutex::new(current),
        }
    }

    pub fn get(&self) -> Settings {
        self.current.lock().unwrap().clone()
    }

    /// 설정을 바꾸고 저장한 뒤 바뀐 값을 돌려준다.
    pub fn update(&self, change: impl FnOnce(&mut Settings)) -> Settings {
        let mut current = self.current.lock().unwrap();
        change(&mut current);

        if let Some(path) = &self.path {
            if let Some(dir) = path.parent() {
                let _ = fs::create_dir_all(dir);
            }
            if let Ok(json) = serde_json::to_string_pretty(&*current) {
                let _ = fs::write(path, json);
            }
        }
        current.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_partial_file_falls_back_to_defaults() {
        let dir = std::env::temp_dir().join(format!("fitzroy-settings-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("nested/settings.json");

        assert_eq!(SettingsStore::load(Some(path.clone())).get(), Settings::default());
        assert_eq!(SettingsStore::load(None).get(), Settings::default());

        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, r#"{"showPercentInMenuBar": false, "unknown": 1}"#).unwrap();
        let settings = SettingsStore::load(Some(path.clone())).get();
        assert!(!settings.show_percent_in_menu_bar);
        assert_eq!(settings.refresh_interval_secs, 60);

        fs::write(&path, "not json").unwrap();
        assert_eq!(SettingsStore::load(Some(path)).get(), Settings::default());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn update_is_saved_and_loaded_again() {
        let dir = std::env::temp_dir().join(format!("fitzroy-settings-save-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("settings.json");

        let store = SettingsStore::load(Some(path.clone()));
        let updated = store.update(|s| s.show_percent_in_menu_bar = false);
        assert!(!updated.show_percent_in_menu_bar);
        assert!(!SettingsStore::load(Some(path)).get().show_percent_in_menu_bar);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn refresh_interval_has_a_floor() {
        let settings = Settings {
            refresh_interval_secs: 1,
            ..Settings::default()
        };
        assert_eq!(settings.refresh_interval(), Duration::from_secs(30));
    }
}

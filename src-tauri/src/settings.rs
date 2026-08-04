use std::{fs, path::PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub provider: Provider,
    pub protocol: Protocol,
    pub base_url: String,
    pub model: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Chatgpt,
    Compatible,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    Responses,
    ChatCompletions,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            provider: Provider::Chatgpt,
            protocol: Protocol::Responses,
            base_url: "http://localhost:11434".to_owned(),
            model: "gpt-5.4-mini".to_owned(),
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        if self.model.trim().is_empty() {
            return Err("Model is required".to_owned());
        }

        if self.provider == Provider::Compatible {
            let url = url::Url::parse(self.base_url.trim())
                .map_err(|_| "Endpoint must be a valid URL".to_owned())?;
            if !matches!(url.scheme(), "http" | "https") {
                return Err("Endpoint must use HTTP or HTTPS".to_owned());
            }
            if url.host_str().is_none() {
                return Err("Endpoint must include a host".to_owned());
            }
        }

        Ok(())
    }
}

fn path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map(|directory| directory.join("settings.json"))
        .map_err(|error| format!("Could not locate the settings directory: {error}"))
}

pub fn load(app: &AppHandle) -> Result<Settings, String> {
    let path = path(app)?;
    if !path.exists() {
        return Ok(Settings::default());
    }

    let contents =
        fs::read_to_string(path).map_err(|error| format!("Could not read settings: {error}"))?;
    let settings: Settings = serde_json::from_str(&contents)
        .map_err(|error| format!("Settings are invalid: {error}"))?;
    settings.validate()?;
    Ok(settings)
}

pub fn save(app: &AppHandle, settings: &Settings) -> Result<(), String> {
    settings.validate()?;
    let path = path(app)?;
    let parent = path
        .parent()
        .ok_or_else(|| "Settings path has no parent directory".to_owned())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create the settings directory: {error}"))?;
    let contents = serde_json::to_string_pretty(settings)
        .map_err(|error| format!("Could not serialize settings: {error}"))?;
    fs::write(path, contents).map_err(|error| format!("Could not save settings: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compatible_provider_requires_http_endpoint() {
        let settings = Settings {
            provider: Provider::Compatible,
            base_url: "file:///tmp/model".to_owned(),
            ..Settings::default()
        };

        assert_eq!(
            settings.validate(),
            Err("Endpoint must use HTTP or HTTPS".to_owned())
        );
    }
}

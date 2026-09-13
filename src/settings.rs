use std::{fs, path::PathBuf};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub provider: Provider,
    pub protocol: Protocol,
    pub base_url: String,
    pub model: String,
    #[serde(default)]
    pub reasoning_effort: ReasoningEffort,
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

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    #[default]
    Default,
    None,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

impl ReasoningEffort {
    pub fn as_api_str(self) -> Option<&'static str> {
        match self {
            Self::Default => None,
            Self::None => Some("none"),
            Self::Minimal => Some("minimal"),
            Self::Low => Some("low"),
            Self::Medium => Some("medium"),
            Self::High => Some("high"),
            Self::Xhigh => Some("xhigh"),
            Self::Max => Some("max"),
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            provider: Provider::Chatgpt,
            protocol: Protocol::Responses,
            base_url: "http://localhost:11434".to_owned(),
            model: "gpt-5.4-mini".to_owned(),
            reasoning_effort: ReasoningEffort::Default,
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
            if url.scheme() == "http" && !is_loopback(&url) {
                return Err("Non-local endpoints must use HTTPS".to_owned());
            }
        }

        Ok(())
    }
}

fn is_loopback(url: &url::Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        None => false,
    }
}

fn path() -> Result<PathBuf, String> {
    ProjectDirs::from("com", "workedexamples", "desktop")
        .map(|dirs| dirs.config_dir().join("settings.json"))
        .ok_or_else(|| "Could not locate the settings directory".to_owned())
}

fn legacy_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(PathBuf::from).map(|path| {
        path.join("com.workedexamples.desktop")
            .join("settings.json")
    })
}

pub fn load() -> Result<Settings, String> {
    let Some(path) = existing_path(path()?, legacy_path()) else {
        return Ok(Settings::default());
    };

    let contents =
        fs::read_to_string(path).map_err(|error| format!("Could not read settings: {error}"))?;
    let settings: Settings = serde_json::from_str(&contents)
        .map_err(|error| format!("Settings are invalid: {error}"))?;
    settings.validate()?;
    Ok(settings)
}

fn existing_path(current: PathBuf, legacy: Option<PathBuf>) -> Option<PathBuf> {
    if current.exists() {
        Some(current)
    } else {
        legacy.filter(|path| path.exists())
    }
}

pub fn save(settings: &Settings) -> Result<(), String> {
    settings.validate()?;
    let path = path()?;
    save_to_path(settings, &path)
}

fn save_to_path(settings: &Settings, path: &std::path::Path) -> Result<(), String> {
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

    #[test]
    fn compatible_provider_requires_https_for_remote_hosts() {
        for base_url in ["http://example.com", "http://192.168.1.20:11434"] {
            let settings = Settings {
                provider: Provider::Compatible,
                base_url: base_url.to_owned(),
                ..Settings::default()
            };

            assert_eq!(
                settings.validate(),
                Err("Non-local endpoints must use HTTPS".to_owned())
            );
        }
    }

    #[test]
    fn compatible_provider_allows_http_only_on_loopback() {
        for base_url in [
            "http://localhost:11434",
            "http://127.0.0.1:11434",
            "http://127.8.9.10:11434",
            "http://[::1]:11434",
            "https://example.com",
        ] {
            let settings = Settings {
                provider: Provider::Compatible,
                base_url: base_url.to_owned(),
                ..Settings::default()
            };

            assert_eq!(settings.validate(), Ok(()), "{base_url}");
        }
    }

    #[test]
    fn existing_settings_default_reasoning_effort() {
        let settings: Settings = serde_json::from_str(
            r#"{"provider":"chatgpt","protocol":"responses","baseUrl":"http://localhost:11434","model":"gpt-5.4-mini"}"#,
        )
        .unwrap();

        assert_eq!(settings.reasoning_effort, ReasoningEffort::Default);
    }
}

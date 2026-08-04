use keyring::{Entry, Error};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// Keep the original service name so an in-place product rename does not orphan credentials.
const SERVICE: &str = "com.workedexamples.desktop";
const CHATGPT_MANIFEST: &str = "chatgpt-oauth-manifest";
const COMPATIBLE_ACCOUNT: &str = "compatible-api-key";
const TOKEN_CHUNK_CHARS: usize = 1000;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthCredentials {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at_ms: u64,
    pub account_id: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct OAuthManifest {
    generation: String,
    access_chunks: usize,
    refresh_chunks: usize,
    expires_at_ms: u64,
    account_id: Option<String>,
    email: Option<String>,
}

fn entry(account: &str) -> Result<Entry, String> {
    Entry::new(SERVICE, account)
        .map_err(|error| format!("Credential store is unavailable: {error}"))
}

pub fn load_oauth() -> Result<Option<OAuthCredentials>, String> {
    let manifest = match entry(CHATGPT_MANIFEST)?.get_password() {
        Ok(value) => serde_json::from_str::<OAuthManifest>(&value)
            .map_err(|_| "Stored ChatGPT credentials are invalid".to_owned())?,
        Err(Error::NoEntry) => return Ok(None),
        Err(error) => return Err(format!("Could not read ChatGPT credentials: {error}")),
    };
    Ok(Some(OAuthCredentials {
        access_token: read_chunks(&manifest.generation, "access", manifest.access_chunks)?,
        refresh_token: read_chunks(&manifest.generation, "refresh", manifest.refresh_chunks)?,
        expires_at_ms: manifest.expires_at_ms,
        account_id: manifest.account_id,
        email: manifest.email,
    }))
}

pub fn save_oauth(credentials: &OAuthCredentials) -> Result<(), String> {
    let old_manifest = load_manifest()?;
    let generation = Uuid::new_v4().simple().to_string();
    let access = chunks(&credentials.access_token);
    let refresh = chunks(&credentials.refresh_token);

    let result = (|| {
        write_chunks(&generation, "access", &access)?;
        write_chunks(&generation, "refresh", &refresh)?;
        let manifest = OAuthManifest {
            generation: generation.clone(),
            access_chunks: access.len(),
            refresh_chunks: refresh.len(),
            expires_at_ms: credentials.expires_at_ms,
            account_id: credentials.account_id.clone(),
            email: credentials.email.clone(),
        };
        let value = serde_json::to_string(&manifest)
            .map_err(|error| format!("Could not serialize ChatGPT credentials: {error}"))?;
        entry(CHATGPT_MANIFEST)?
            .set_password(&value)
            .map_err(|error| format!("Could not save ChatGPT credentials: {error}"))
    })();

    if let Err(error) = result {
        delete_generation(&generation, access.len(), refresh.len());
        return Err(error);
    }
    if let Some(old) = old_manifest {
        delete_generation(&old.generation, old.access_chunks, old.refresh_chunks);
    }
    Ok(())
}

pub fn delete_oauth() -> Result<(), String> {
    let manifest = load_manifest()?;
    match entry(CHATGPT_MANIFEST)?.delete_credential() {
        Ok(()) | Err(Error::NoEntry) => Ok(()),
        Err(error) => Err(format!("Could not remove ChatGPT credentials: {error}")),
    }?;
    if let Some(manifest) = manifest {
        delete_generation(
            &manifest.generation,
            manifest.access_chunks,
            manifest.refresh_chunks,
        );
    }
    Ok(())
}

fn load_manifest() -> Result<Option<OAuthManifest>, String> {
    match entry(CHATGPT_MANIFEST)?.get_password() {
        Ok(value) => serde_json::from_str(&value)
            .map(Some)
            .map_err(|_| "Stored ChatGPT credentials are invalid".to_owned()),
        Err(Error::NoEntry) => Ok(None),
        Err(error) => Err(format!("Could not read ChatGPT credentials: {error}")),
    }
}

fn chunks(value: &str) -> Vec<String> {
    if value.is_empty() {
        return vec![String::new()];
    }

    let mut chunks = Vec::new();
    let mut current = String::new();
    for character in value.chars() {
        if current.chars().count() == TOKEN_CHUNK_CHARS {
            chunks.push(current);
            current = String::new();
        }
        current.push(character);
    }
    chunks.push(current);
    chunks
}

fn chunk_account(generation: &str, token: &str, index: usize) -> String {
    format!("chatgpt-oauth-{generation}-{token}-{index}")
}

fn write_chunks(generation: &str, token: &str, chunks: &[String]) -> Result<(), String> {
    for (index, chunk) in chunks.iter().enumerate() {
        entry(&chunk_account(generation, token, index))?
            .set_password(chunk)
            .map_err(|error| format!("Could not save ChatGPT credentials: {error}"))?;
    }
    Ok(())
}

fn read_chunks(generation: &str, token: &str, count: usize) -> Result<String, String> {
    let mut value = String::new();
    for index in 0..count {
        let chunk = entry(&chunk_account(generation, token, index))?
            .get_password()
            .map_err(|error| format!("Stored ChatGPT credentials are incomplete: {error}"))?;
        value.push_str(&chunk);
    }
    Ok(value)
}

fn delete_generation(generation: &str, access_chunks: usize, refresh_chunks: usize) {
    for (token, count) in [("access", access_chunks), ("refresh", refresh_chunks)] {
        for index in 0..count {
            let Ok(entry) = entry(&chunk_account(generation, token, index)) else {
                continue;
            };
            let _ = entry.delete_credential();
        }
    }
}

pub fn has_api_key() -> Result<bool, String> {
    match entry(COMPATIBLE_ACCOUNT)?.get_password() {
        Ok(_) => Ok(true),
        Err(Error::NoEntry) => Ok(false),
        Err(error) => Err(format!("Could not read the endpoint credential: {error}")),
    }
}

pub fn load_api_key() -> Result<Option<String>, String> {
    match entry(COMPATIBLE_ACCOUNT)?.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(Error::NoEntry) => Ok(None),
        Err(error) => Err(format!("Could not read the endpoint credential: {error}")),
    }
}

pub fn save_api_key(value: Option<&str>) -> Result<(), String> {
    let entry = entry(COMPATIBLE_ACCOUNT)?;
    match value.map(str::trim) {
        Some(value) if !value.is_empty() => entry
            .set_password(value)
            .map_err(|error| format!("Could not save the endpoint credential: {error}")),
        _ => match entry.delete_credential() {
            Ok(()) | Err(Error::NoEntry) => Ok(()),
            Err(error) => Err(format!("Could not remove the endpoint credential: {error}")),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_and_reassembles_large_tokens() {
        let token = "a".repeat(4200);
        let chunks = chunks(&token);

        assert_eq!(chunks.len(), 5);
        assert!(chunks.iter().all(|chunk| chunk.chars().count() <= 1000));
        assert_eq!(chunks.concat(), token);
    }

    #[test]
    fn empty_token_still_has_one_chunk() {
        assert_eq!(chunks(""), vec![String::new()]);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn credential_backend_persists_across_entries() {
        let service = format!("com.workedexamples.test.{}", Uuid::new_v4().simple());
        let writer = Entry::new(&service, "persistence-probe").unwrap();
        writer.set_password("probe-value").unwrap();

        let reader = Entry::new(&service, "persistence-probe").unwrap();
        assert_eq!(reader.get_password().unwrap(), "probe-value");
        reader.delete_credential().unwrap();
    }
}

//! Per-profile viewing/behaviour settings commands (cleanup phase).
//!
//! `get_profile_settings` returns the saved settings (or defaults);
//! `set_profile_settings` upserts them. Stored in the profile DB.
//!
//! The TMDB credential is **write-only** from the webview: reads report only
//! whether one is configured plus a short hint (its last 4 characters), and a
//! general settings save always keeps the stored credential. Only
//! `set_tmdb_credential` / `clear_tmdb_credential` change it.

use nova_core::ProfileSettings;
use nova_storage::ProfileSettingsRepository;
use serde::Serialize;
use tauri::State;

use crate::ipc::ApiError;
use crate::state::AppState;

use super::require_profile_pool;

/// Longest credential accepted (TMDB read access tokens are ~250 bytes).
const MAX_CREDENTIAL_BYTES: usize = 1024;

/// What the webview may see: every setting except the credential itself.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSettingsView {
    #[serde(flatten)]
    pub settings: ProfileSettings,
    pub tmdb_credential_configured: bool,
    /// Last 4 characters, so the user can tell which token is saved.
    pub tmdb_credential_hint: Option<String>,
}

fn configured(key: &Option<String>) -> Option<&str> {
    key.as_deref().map(str::trim).filter(|k| !k.is_empty())
}

/// Strip the credential from settings bound for the webview.
fn view(mut settings: ProfileSettings) -> ProfileSettingsView {
    let key = settings.tmdb_api_key.take();
    let hint = configured(&key).map(|k| {
        let chars: Vec<char> = k.chars().collect();
        chars[chars.len().saturating_sub(4)..].iter().collect()
    });
    ProfileSettingsView {
        settings,
        tmdb_credential_configured: hint.is_some(),
        tmdb_credential_hint: hint,
    }
}

/// A general save never changes the credential, whatever the webview sent.
fn merge_for_save(stored: &ProfileSettings, mut incoming: ProfileSettings) -> ProfileSettings {
    incoming.tmdb_api_key = stored.tmdb_api_key.clone();
    incoming
}

fn validate_credential(credential: &str) -> Result<String, ApiError> {
    let credential = credential.trim();
    if credential.is_empty()
        || credential.len() > MAX_CREDENTIAL_BYTES
        || credential
            .chars()
            .any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(ApiError::InvalidInput(
            "the TMDB token looks malformed".into(),
        ));
    }
    Ok(credential.to_string())
}

#[tauri::command]
pub async fn get_profile_settings(
    profile_id: String,
    state: State<'_, AppState>,
) -> Result<ProfileSettingsView, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    Ok(view(ProfileSettingsRepository::new(&pool).get().await?))
}

#[tauri::command]
pub async fn set_profile_settings(
    profile_id: String,
    settings: ProfileSettings,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    let repo = ProfileSettingsRepository::new(&pool);
    let stored = repo.get().await?;
    repo.set(&merge_for_save(&stored, settings))
        .await
        .map_err(Into::into)
}

/// Save (or replace) the TMDB credential. It is never returned to the webview.
#[tauri::command]
pub async fn set_tmdb_credential(
    profile_id: String,
    credential: String,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    let credential = validate_credential(&credential)?;
    let pool = require_profile_pool(&profile_id, &state).await?;
    let repo = ProfileSettingsRepository::new(&pool);
    let mut settings = repo.get().await?;
    settings.tmdb_api_key = Some(credential);
    repo.set(&settings).await.map_err(Into::into)
}

/// Forget the TMDB credential; TMDB matching stops until a new one is saved.
#[tauri::command]
pub async fn clear_tmdb_credential(
    profile_id: String,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    let repo = ProfileSettingsRepository::new(&pool);
    let mut settings = repo.get().await?;
    settings.tmdb_api_key = None;
    repo.set(&settings).await.map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "eyJhbGciOiJIUzI1NiJ9.eyJhdWQiOiJzZWNyZXQifQ.c2lnbmF0dXJlWXYZ";

    fn with_token() -> ProfileSettings {
        ProfileSettings {
            tmdb_enabled: true,
            tmdb_api_key: Some(TOKEN.into()),
            ..ProfileSettings::default()
        }
    }

    #[test]
    fn the_view_never_contains_the_credential() {
        let v = view(with_token());
        let json = serde_json::to_string(&v).unwrap();
        assert!(
            !json.contains("eyJ") && !json.contains("tmdbApiKey"),
            "{json}"
        );
        assert!(json.contains("\"tmdbCredentialConfigured\":true"));
        assert!(json.contains("\"tmdbCredentialHint\":\"WXYZ\""));
        assert!(
            json.contains("\"tmdbEnabled\":true"),
            "other settings still flow"
        );

        let empty = view(ProfileSettings {
            tmdb_api_key: Some("   ".into()),
            ..ProfileSettings::default()
        });
        assert!(!empty.tmdb_credential_configured && empty.tmdb_credential_hint.is_none());
    }

    #[test]
    fn a_general_save_keeps_the_stored_credential() {
        let stored = with_token();
        // The webview sends settings without the key (or a stale/forged one).
        for sent in [None, Some("attacker-supplied".to_string())] {
            let incoming = ProfileSettings {
                autoplay_next: false,
                tmdb_api_key: sent,
                ..ProfileSettings::default()
            };
            let saved = merge_for_save(&stored, incoming);
            assert_eq!(saved.tmdb_api_key.as_deref(), Some(TOKEN));
            assert!(!saved.autoplay_next, "the rest of the save applies");
        }
    }

    #[test]
    fn credentials_are_validated() {
        assert_eq!(validate_credential(&format!("  {TOKEN}\n")).unwrap(), TOKEN);
        for bad in ["", "   ", "two words", "tab\there", &"x".repeat(1025)] {
            assert!(validate_credential(bad).is_err(), "{bad:?}");
        }
    }
}

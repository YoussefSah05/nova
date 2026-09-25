//! User-selected model folders. Bookmarks stay in app.db and are never sent to
//! the webview. The current Tauri bundle is not App Sandboxed; the scoped form
//! is preferred when macOS offers it, while a normal bookmark preserves the
//! folder identity in the current non-sandboxed distribution.

use std::path::Path;

use objc2::{runtime::Bool, MainThreadMarker};
use objc2_app_kit::{NSModalResponseOK, NSOpenPanel};
use objc2_foundation::{
    NSData, NSURLBookmarkCreationOptions, NSURLBookmarkResolutionOptions, NSURL,
};
use tauri::AppHandle;

use crate::ipc::ApiError;

const PLAIN_BOOKMARK: u8 = 0;
const SCOPED_BOOKMARK: u8 = 1;

pub async fn choose_folder(app: &AppHandle) -> Result<Option<Vec<u8>>, ApiError> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(choose_folder_on_main());
    })
    .map_err(|_| ApiError::Internal("could not open model folder picker".into()))?;
    rx.await
        .map_err(|_| ApiError::Internal("model folder picker closed unexpectedly".into()))?
        .map_err(ApiError::Internal)
}

fn choose_folder_on_main() -> Result<Option<Vec<u8>>, String> {
    let mtm = MainThreadMarker::new().ok_or("model folder picker is not on the main thread")?;
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseDirectories(true);
    panel.setCanChooseFiles(false);
    panel.setAllowsMultipleSelection(false);
    if panel.runModal() != NSModalResponseOK {
        return Ok(None);
    }
    let url = panel
        .URLs()
        .firstObject()
        .ok_or("model folder picker returned no folder")?;
    let scoped_options = NSURLBookmarkCreationOptions::WithSecurityScope
        | NSURLBookmarkCreationOptions::SecurityScopeAllowOnlyReadAccess;
    let (kind, data) = match url
        .bookmarkDataWithOptions_includingResourceValuesForKeys_relativeToURL_error(
            scoped_options,
            None,
            None,
        ) {
        Ok(data) => (SCOPED_BOOKMARK, data),
        Err(_) => (
            PLAIN_BOOKMARK,
            url.bookmarkDataWithOptions_includingResourceValuesForKeys_relativeToURL_error(
                NSURLBookmarkCreationOptions::empty(),
                None,
                None,
            )
            .map_err(|_| "could not remember model folder")?,
        ),
    };
    let mut bytes = Vec::with_capacity(data.len() + 1);
    bytes.push(kind);
    bytes.extend_from_slice(&data.to_vec());
    Ok(Some(bytes))
}

struct ScopeGuard<'a>(&'a NSURL, bool);

impl Drop for ScopeGuard<'_> {
    fn drop(&mut self) {
        if self.1 {
            // SAFETY: Every successful start has exactly one matching stop,
            // and the NSURL outlives this guard.
            unsafe { self.0.stopAccessingSecurityScopedResource() };
        }
    }
}

/// Run a bounded metadata probe while the resolved grant is active. Resolution
/// failure and removed drives are ordinary unavailable states, not app errors.
pub fn with_bookmarked_folder<T>(
    bookmark: &[u8],
    probe: impl FnOnce(&Path) -> T,
) -> Option<(T, Option<Vec<u8>>)> {
    let (&kind, bytes) = bookmark.split_first()?;
    if bytes.is_empty() || !matches!(kind, PLAIN_BOOKMARK | SCOPED_BOOKMARK) {
        return None;
    }
    let data = NSData::with_bytes(bytes);
    let mut stale = Bool::NO;
    let mut options =
        NSURLBookmarkResolutionOptions::WithoutUI | NSURLBookmarkResolutionOptions::WithoutMounting;
    if kind == SCOPED_BOOKMARK {
        options |= NSURLBookmarkResolutionOptions::WithSecurityScope;
    }
    // SAFETY: `stale` is a live Bool for the duration of the call; NSData
    // retains an owned copy of the stored bytes.
    let url = unsafe {
        NSURL::URLByResolvingBookmarkData_options_relativeToURL_bookmarkDataIsStale_error(
            &data, options, None, &mut stale,
        )
    }
    .ok()?;
    let access_started = if kind == SCOPED_BOOKMARK {
        // SAFETY: The resolved URL and matching ScopeGuard are held together.
        unsafe { url.startAccessingSecurityScopedResource() }
    } else {
        false
    };
    if kind == SCOPED_BOOKMARK && !access_started {
        return None;
    }
    let _scope = ScopeGuard(&url, access_started);
    let path = url.path()?.to_string();
    let path = Path::new(&path);
    if !path.is_dir() {
        return None;
    }
    let observation = probe(path);
    let renewed = if stale.as_bool() {
        let creation_options = if kind == SCOPED_BOOKMARK {
            NSURLBookmarkCreationOptions::WithSecurityScope
                | NSURLBookmarkCreationOptions::SecurityScopeAllowOnlyReadAccess
        } else {
            NSURLBookmarkCreationOptions::empty()
        };
        url.bookmarkDataWithOptions_includingResourceValuesForKeys_relativeToURL_error(
            creation_options,
            None,
            None,
        )
        .ok()
        .map(|data| {
            let mut bytes = Vec::with_capacity(data.len() + 1);
            bytes.push(kind);
            bytes.extend_from_slice(&data.to_vec());
            bytes
        })
    } else {
        None
    };
    Some((observation, renewed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2_foundation::NSString;

    #[test]
    fn ordinary_bookmark_recovers_same_folder_and_reports_removed_folder_offline() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("removable-fixture");
        std::fs::create_dir(&folder).unwrap();
        let path = NSString::from_str(folder.to_str().unwrap());
        let url = NSURL::fileURLWithPath_isDirectory(&path, true);
        let data = url
            .bookmarkDataWithOptions_includingResourceValuesForKeys_relativeToURL_error(
                NSURLBookmarkCreationOptions::empty(),
                None,
                None,
            )
            .unwrap();
        let mut bookmark = vec![PLAIN_BOOKMARK];
        bookmark.extend_from_slice(&data.to_vec());
        let resolved = with_bookmarked_folder(&bookmark, |path| path.is_dir());
        assert_eq!(resolved.map(|(exists, _)| exists), Some(true));

        std::fs::remove_dir(&folder).unwrap();
        assert!(with_bookmarked_folder(&bookmark, |_| ()).is_none());
        std::fs::create_dir(&folder).unwrap();
        assert!(with_bookmarked_folder(&bookmark, |_| ()).is_some());
    }
}

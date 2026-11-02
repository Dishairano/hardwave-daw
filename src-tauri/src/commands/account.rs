//! Signing in to Hardwave from inside the DAW, and showing the account.
//!
//! The window never holds the token. It sends the e-mail address and the
//! password here once; the request to the site is made here, and the token
//! the site answers with goes straight into the file every Hardwave program
//! on the computer shares, so the plug-ins are signed in too. What goes back
//! to the window is only what it shows: a name, a mostly hidden address and
//! the plan.
//!
//! The work itself (the requests, the file, the renewal) is in the
//! hardwave-account crate, where it is tested against a stand-in site.

use hardwave_account::client::{AccountStatus, Site, OUR_SITE};
use hardwave_account::shared_token;
use std::time::{SystemTime, UNIX_EPOCH};

/// One account change at a time in this DAW: a check that renews the token
/// must not write it back after a sign-out from the same window deleted it.
static BUSY: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// What a feature that needs an account says to someone who is not signed in.
pub const SIGN_IN_FIRST: &str = "Sign in first: the Sign in button is at the top right of the \
     DAW. Every Hardwave program on this computer shares that one sign-in.";

/// The shared token, for a feature that needs an account.
pub fn token_or_sign_in_first() -> Result<String, String> {
    shared_token::load().ok_or_else(|| SIGN_IN_FIRST.to_string())
}

fn site() -> Result<Site, String> {
    Site::at(&crate::endpoints::service_url("HARDWAVE_SITE", OUR_SITE))
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Where this computer's sign-in stands. Asked when the DAW starts and
/// whenever its window comes back to the front, so a sign-in or sign-out in
/// the Suite or a plug-in shows here too. Renews a token that is getting old.
#[tauri::command]
pub async fn account_status() -> Result<AccountStatus, String> {
    let _one_at_a_time = BUSY.lock().await;
    let Some(file) = shared_token::path() else {
        return Ok(AccountStatus::SignedOut);
    };
    Ok(site()?.check(&file, now()).await)
}

/// Sign in with an e-mail address and password.
#[tauri::command]
pub async fn account_sign_in(email: String, password: String) -> Result<AccountStatus, String> {
    let _one_at_a_time = BUSY.lock().await;
    let file = shared_token::path()
        .ok_or_else(|| "Could not find the folder this computer keeps app data in.".to_string())?;
    site()?.sign_in_here(&file, &email, &password, now()).await
}

/// Sign this computer out: the shared file goes, so the plug-ins are signed
/// out as well. The site is told afterwards, without waiting for it.
#[tauri::command]
pub async fn account_sign_out() -> Result<AccountStatus, String> {
    let _one_at_a_time = BUSY.lock().await;
    let Some(file) = shared_token::path() else {
        return Ok(AccountStatus::SignedOut);
    };
    let token = shared_token::read(&file);
    shared_token::remove(&file)?;
    if let (Some(token), Ok(site)) = (token, site()) {
        tauri::async_runtime::spawn(async move { site.tell_signed_out(&token).await });
    }
    Ok(AccountStatus::SignedOut)
}

/// The site's pages the account panel links to, by name. The window names
/// one; it cannot send an address of its own.
pub fn page_url(page: &str) -> Option<String> {
    let path = match page {
        "register" => "/register",
        "forgot-password" => "/forgot-password",
        "account" => "/account",
        _ => return None,
    };
    Some(format!("{OUR_SITE}{path}"))
}

/// Open one of the site's account pages in the browser.
#[tauri::command]
pub fn account_open_page(app: tauri::AppHandle, page: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let url = page_url(&page).ok_or_else(|| "not a page of the account".to_string())?;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|_| "could not open the browser".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_can_only_open_the_sites_own_account_pages() {
        assert_eq!(
            page_url("register").as_deref(),
            Some("https://hardwavestudios.com/register")
        );
        assert_eq!(
            page_url("forgot-password").as_deref(),
            Some("https://hardwavestudios.com/forgot-password")
        );
        assert_eq!(
            page_url("account").as_deref(),
            Some("https://hardwavestudios.com/account")
        );
        for other in [
            "",
            "https://evil.example",
            "../etc",
            "account?x=1",
            "pricing",
        ] {
            assert_eq!(page_url(other), None, "{other}");
        }
    }
}

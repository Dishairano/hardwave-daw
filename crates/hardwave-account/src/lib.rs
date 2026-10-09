//! Who is on the other end, and may they use this.
//!
//! The DAW holds the same account token the plug-in windows use. The
//! site is the only thing that knows whether it is valid and whether
//! that account has Pro, so a service asks it rather than deciding for
//! itself. Nothing the client says about itself is believed.
//!
//! Every service the DAW talks to checks an account through here: the
//! room two producers meet in, and the stem separation queue.

use serde::Deserialize;

pub struct Who {
    pub user_id: String,
    pub display_name: String,
    pub subscribed: bool,
}

/// What the site's /api/auth/me answers. It writes the id as a number
/// and the name as `displayName`; this read the id as text and the name
/// as `display_name`, so every room failed with "the site answered
/// something we could not read" and the name never came across.
#[derive(Deserialize)]
struct Me {
    #[serde(default)]
    id: Option<Id>,
    #[serde(default, alias = "userId")]
    user_id: Option<Id>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default, alias = "displayName")]
    display_name: Option<String>,
    /// Staff, who have Pro without a subscription.
    #[serde(default, rename = "isAdmin")]
    is_admin: bool,
}

/// An account id, as a number or as text.
#[derive(Deserialize)]
#[serde(untagged)]
enum Id {
    Number(u64),
    Text(String),
}

impl Id {
    fn into_string(self) -> String {
        match self {
            Id::Number(n) => n.to_string(),
            Id::Text(s) => s,
        }
    }
}

/// Read /api/auth/me: (account id, name to show, is staff).
fn read_me(body: &[u8]) -> Result<(String, String, bool), String> {
    let me: Me = serde_json::from_slice(body)
        .map_err(|_| "the site answered something we could not read".to_string())?;
    let user_id = me
        .id
        .or(me.user_id)
        .map(Id::into_string)
        .or_else(|| me.email.clone())
        .ok_or_else(|| "the site did not say who you are".to_string())?;
    // Never the email address: the name is shown to the other person in
    // a room, and an address is not theirs to see.
    let name = me
        .display_name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| "A producer".into());
    Ok((user_id, name, me.is_admin))
}

#[derive(Deserialize)]
struct Subscription {
    #[serde(default, rename = "hasSubscription")]
    has_subscription: bool,
}

/// Where to ask. The live service asks hardwavestudios.com; a test
/// passes its own stand-in, which is why this is an argument rather
/// than a global.
pub fn site_from_environment() -> String {
    std::env::var("HARDWAVE_SITE").unwrap_or_else(|_| "https://hardwavestudios.com".to_string())
}

/// Ask the site who this token belongs to and whether they have Pro.
pub async fn identify(site: &str, token: &str) -> Result<Who, String> {
    if token.trim().is_empty() {
        return Err("sign in first".into());
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("could not reach the site: {e}"))?;

    let body = client
        .get(format!("{site}/api/auth/me"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|_| "could not reach the site to check who you are".to_string())?
        .error_for_status()
        .map_err(|_| "that sign-in is no longer valid".to_string())?
        .bytes()
        .await
        .map_err(|_| "the site answered something we could not read".to_string())?;
    let (user_id, display_name, is_admin) = read_me(&body)?;

    let subscription: Subscription = client
        .get(format!("{site}/api/subscription"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|_| "could not reach the site to check your subscription".to_string())?
        .json()
        .await
        .unwrap_or(Subscription {
            has_subscription: false,
        });

    Ok(Who {
        display_name,
        user_id,
        subscribed: subscription.has_subscription || is_admin,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sites_own_answer_is_read() {
        // What hardwavestudios.com/api/auth/me sends, field for field.
        let body = br#"{"id":8,"email":"someone@example.com","displayName":"Dish","avatarUrl":null,
            "isAdmin":true,"role":"admin","discordUsername":null,"discordLinkedAt":null}"#;
        let (id, name, admin) = read_me(body).expect("it reads");
        assert_eq!(id, "8");
        assert_eq!(name, "Dish");
        assert!(admin);
    }

    #[test]
    fn an_id_as_text_and_no_name_still_read() {
        let (id, name, admin) = read_me(br#"{"id":"u-42"}"#).expect("it reads");
        assert_eq!(
            (id.as_str(), name.as_str(), admin),
            ("u-42", "A producer", false)
        );
    }

    #[test]
    fn something_that_is_not_an_account_says_so() {
        assert_eq!(
            read_me(b"<html>").unwrap_err(),
            "the site answered something we could not read"
        );
        assert_eq!(
            read_me(b"{}").unwrap_err(),
            "the site did not say who you are"
        );
    }
}

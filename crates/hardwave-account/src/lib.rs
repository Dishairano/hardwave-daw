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

#[derive(Deserialize)]
struct Me {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
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

    let me: Me = client
        .get(format!("{site}/api/auth/me"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|_| "could not reach the site to check who you are".to_string())?
        .error_for_status()
        .map_err(|_| "that sign-in is no longer valid".to_string())?
        .json()
        .await
        .map_err(|_| "the site answered something we could not read".to_string())?;

    let user_id = me
        .id
        .or(me.user_id)
        .or_else(|| me.email.clone())
        .ok_or_else(|| "the site did not say who you are".to_string())?;

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
        display_name: me
            .display_name
            .or(me.email)
            .unwrap_or_else(|| "A producer".into()),
        user_id,
        subscribed: subscription.has_subscription,
    })
}

//! The DAW's own side of the account: signing in and out, keeping the
//! sign-in fresh, and asking the site what the account has.
//!
//! It uses the site's endpoints the Suite and the plug-ins use, with the
//! same token in the same shared file (`shared_token`), so a sign-in here
//! is a sign-in for every Hardwave program on the computer.
//!
//! Every request goes to hardwavestudios.com over https with the token in
//! the Authorization header, never in an address, and no redirect is
//! followed, so the token is never carried anywhere else. The password goes
//! to the site once, in the body of the sign-in request, and is not kept.
//! No error text here holds a token, a password or a reply body.
//!
//! Nothing is believed from this side: whether the account has Pro is what
//! the site says when asked.

use crate::shared_token;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;

/// The site every request goes to.
pub const OUR_SITE: &str = "https://hardwavestudios.com";

/// A request that has not answered in this long has failed.
const TIMEOUT: Duration = Duration::from_secs(10);
/// Signing out only tells the site as a courtesy; it does not wait long.
const SIGN_OUT_TIMEOUT: Duration = Duration::from_secs(5);
/// Longest reply read. The site's answers are well under a kilobyte.
const MAX_REPLY: usize = 64 * 1024;
/// A token with less than this left is renewed at the next check. The
/// site's tokens last 7 days, so a DAW opened every day renews about once
/// a day, and one left closed for a week asks for a sign-in again.
pub const RENEW_UNDER_SECS: i64 = 6 * 24 * 60 * 60;
/// A token this close to running out counts as run out, so it is never
/// sent on a request it would expire during.
const EXPIRY_MARGIN_SECS: i64 = 60;

pub const UNREACHABLE: &str =
    "Could not reach hardwavestudios.com. Check the internet connection and try again.";
pub const MISSING_FIELDS: &str = "Fill in your e-mail address and password.";
pub const WRONG_PASSWORD: &str = "That e-mail address and password do not match an account.";
pub const DEACTIVATED: &str =
    "This account has been switched off. Get in touch through hardwavestudios.com.";
pub const SITE_TROUBLE: &str = "The site could not sign you in just now. Try again in a minute.";
pub const NO_TOKEN_IN_REPLY: &str = "The site's answer held no sign-in. Try again in a minute.";

/// Where this computer's sign-in stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum AccountStatus {
    /// Nobody is signed in on this computer.
    SignedOut,
    /// A sign-in is on this computer but the site no longer takes it: it
    /// ran out, the password changed, or "sign out everywhere" was used.
    /// The shared file stays where it is, because another program may be
    /// about to renew it; only signing out deletes it.
    Ended,
    /// Signed in as far as this computer knows, but the site could not be
    /// asked (offline, or the site is down).
    Unreachable,
    SignedIn(Account),
}

/// The account, as much of it as the DAW shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub name: String,
    /// The address with most of its first part hidden, `dis…@example.com`,
    /// so a screen share or a screenshot does not hand it out.
    pub email: String,
    pub plan: Plan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Plan {
    Pro {
        via: ProVia,
        /// The day a cancelled Pro stops (`YYYY-MM-DD`); none while it renews.
        until: Option<String>,
    },
    Free,
    /// The site could not say. Never shown as Free: a member must not be
    /// told they have no Pro because a request failed.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProVia {
    Subscription,
    Trial,
    /// Hardwave staff, who have Pro without a subscription.
    Staff,
}

/// The site, and the one HTTP client every account request goes through.
pub struct Site {
    base: String,
    http: reqwest::Client,
}

impl Site {
    /// A site at `base`: https, or plain http to this machine, which is
    /// where the tests' stand-in runs. Nothing else is accepted.
    pub fn at(base: &str) -> Result<Self, String> {
        let url = reqwest::Url::parse(base).map_err(|_| "Not a site address.".to_string())?;
        let local = matches!(
            url.host_str(),
            Some("127.0.0.1") | Some("localhost") | Some("[::1]")
        );
        match url.scheme() {
            "https" if url.host_str().is_some() => {}
            "http" if local => {}
            _ => return Err("The account is only ever sent over https.".into()),
        }
        let mut builder = reqwest::Client::builder()
            .timeout(TIMEOUT)
            // The token rides in a header; a redirect could take it to
            // another host. Our endpoints never redirect.
            .redirect(reqwest::redirect::Policy::none())
            .https_only(!local)
            .user_agent(concat!("HardwaveDAW/", env!("CARGO_PKG_VERSION")));
        if local {
            builder = builder.no_proxy();
        }
        let http = builder
            .build()
            .map_err(|_| "Could not set up a connection.".to_string())?;
        Ok(Site {
            base: base.trim_end_matches('/').to_string(),
            http,
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    /// Swap an e-mail address and password for a token (POST
    /// /api/auth/login, as the Suite does).
    pub async fn sign_in(&self, email: &str, password: &str) -> Result<String, String> {
        let email = email.trim();
        if email.is_empty() || password.is_empty() {
            return Err(MISSING_FIELDS.into());
        }
        // Far beyond any real address or password; anything longer is not
        // worth sending.
        if email.len() > 254 || !email.contains('@') {
            return Err("That does not look like an e-mail address.".into());
        }
        if password.len() > 1024 {
            return Err(WRONG_PASSWORD.into());
        }
        let reply = self
            .http
            .post(self.url("/api/auth/login"))
            .json(&serde_json::json!({ "email": email, "password": password }))
            .send()
            .await
            .map_err(|_| UNREACHABLE.to_string())?;
        let status = reply.status().as_u16();
        let retry_after = reply
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok());
        if status != 200 {
            return Err(sign_in_refusal(status, retry_after));
        }
        let body = read_capped(reply).await.ok_or(NO_TOKEN_IN_REPLY)?;
        token_from(&body).ok_or_else(|| NO_TOKEN_IN_REPLY.to_string())
    }

    /// Swap a still-valid token for a fresh one (POST /api/auth/refresh,
    /// as the Suite and the plug-in windows do).
    pub async fn refresh(&self, token: &str) -> Result<String, String> {
        let reply = self
            .http
            .post(self.url("/api/auth/refresh"))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|_| UNREACHABLE.to_string())?;
        if !reply.status().is_success() {
            return Err(format!(
                "the site did not renew the sign-in (HTTP {})",
                reply.status().as_u16()
            ));
        }
        let body = read_capped(reply).await.ok_or(NO_TOKEN_IN_REPLY)?;
        token_from(&body).ok_or_else(|| NO_TOKEN_IN_REPLY.to_string())
    }

    /// Tell the site this computer signed out (POST /api/auth/logout, as
    /// the Suite does). A courtesy: what signs the computer out is deleting
    /// the shared file, so a failure here changes nothing.
    pub async fn tell_signed_out(&self, token: &str) {
        let _ = self
            .http
            .post(self.url("/api/auth/logout"))
            .bearer_auth(token)
            .timeout(SIGN_OUT_TIMEOUT)
            .send()
            .await;
    }

    /// Who the token belongs to (GET /api/auth/me).
    async fn me(&self, token: &str) -> Result<crate::MeReply, Problem> {
        let reply = self
            .http
            .get(self.url("/api/auth/me"))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|_| Problem::Unreachable)?;
        match reply.status().as_u16() {
            200 => {}
            401 | 403 => return Err(Problem::Refused),
            _ => return Err(Problem::Unreachable),
        }
        let body = read_capped(reply).await.ok_or(Problem::Unreachable)?;
        crate::parse_me(&body).map_err(|_| Problem::Unreachable)
    }

    /// What plan the account is on (GET /api/subscription). Staff count as
    /// Pro, as they do for rooms and stems.
    async fn plan(&self, token: &str, staff: bool) -> Plan {
        let asked = async {
            let reply = self
                .http
                .get(self.url("/api/subscription"))
                .bearer_auth(token)
                .send()
                .await
                .ok()?;
            if !reply.status().is_success() {
                return None;
            }
            let body = read_capped(reply).await?;
            serde_json::from_slice::<SubscriptionReply>(&body).ok()
        };
        plan_from(asked.await, staff)
    }

    /// Where this computer's sign-in stands, renewing it on the way when it
    /// is getting old. `file` is the shared token file; `now` is the time
    /// in seconds since 1970.
    pub async fn check(&self, file: &Path, now: i64) -> AccountStatus {
        let Some(mut token) = shared_token::read(file) else {
            return AccountStatus::SignedOut;
        };
        if let Some(claims) = Claims::of(&token) {
            if !claims.valid_at(now) {
                return AccountStatus::Ended;
            }
            if claims.exp - now < RENEW_UNDER_SECS {
                if let Some(renewed) = self.renew(file, &token, now).await {
                    token = renewed;
                }
            }
        }
        match self.me(&token).await {
            Err(Problem::Refused) => AccountStatus::Ended,
            Err(Problem::Unreachable) => AccountStatus::Unreachable,
            Ok(me) => {
                let plan = self.plan(&token, me.is_admin).await;
                AccountStatus::SignedIn(Account {
                    name: shown_name(me.display_name.as_deref(), me.email.as_deref()),
                    email: me.email.as_deref().map(mask_email).unwrap_or_default(),
                    plan,
                })
            }
        }
    }

    /// Renew `token` and put the new one in the shared file, unless another
    /// program changed the file meanwhile (signed out, or in as someone
    /// else): that change stands. Returns the token now in the file.
    async fn renew(&self, file: &Path, token: &str, now: i64) -> Option<String> {
        let renewed = self.refresh(token).await.ok()?;
        if !is_renewal_of(&renewed, token, now) {
            return None;
        }
        if shared_token::read(file).as_deref() != Some(token) {
            return None;
        }
        shared_token::write(file, &renewed).ok()?;
        Some(renewed)
    }

    /// Sign in and keep the token in the shared file, so every Hardwave
    /// program on the computer is signed in; then say where things stand.
    pub async fn sign_in_here(
        &self,
        file: &Path,
        email: &str,
        password: &str,
        now: i64,
    ) -> Result<AccountStatus, String> {
        let token = self.sign_in(email, password).await?;
        shared_token::write(file, &token)?;
        Ok(self.check(file, now).await)
    }
}

enum Problem {
    /// The site says the token is no good.
    Refused,
    /// The site could not be asked, or did not answer sensibly.
    Unreachable,
}

/// What to tell someone whose sign-in the site refused with `status`.
fn sign_in_refusal(status: u16, retry_after: Option<u64>) -> String {
    match status {
        400 => MISSING_FIELDS.into(),
        401 => WRONG_PASSWORD.into(),
        403 => DEACTIVATED.into(),
        429 => match retry_after {
            Some(secs) if secs > 0 => {
                let minutes = secs.div_ceil(60);
                let unit = if minutes == 1 { "minute" } else { "minutes" };
                format!("Too many tries. Wait {minutes} {unit} and try again.")
            }
            _ => "Too many tries. Wait a few minutes and try again.".into(),
        },
        500..=599 => SITE_TROUBLE.into(),
        other => format!("The site gave an answer the DAW does not know (HTTP {other})."),
    }
}

/// A reply's body, or nothing when it is bigger than any answer we expect.
async fn read_capped(reply: reqwest::Response) -> Option<Vec<u8>> {
    if reply
        .content_length()
        .is_some_and(|len| len > MAX_REPLY as u64)
    {
        return None;
    }
    let body = reply.bytes().await.ok()?;
    (body.len() <= MAX_REPLY).then(|| body.to_vec())
}

#[derive(Deserialize)]
struct TokenReply {
    #[serde(default)]
    success: Option<bool>,
    #[serde(default)]
    token: Option<String>,
}

/// The token in a sign-in or renewal reply: `{"success":true,"token":..}`
/// from a sign-in, `{"token":..}` from a renewal.
fn token_from(body: &[u8]) -> Option<String> {
    let reply: TokenReply = serde_json::from_slice(body).ok()?;
    if reply.success == Some(false) {
        return None;
    }
    reply.token.filter(|t| shared_token::looks_like_token(t))
}

#[derive(Deserialize)]
struct SubscriptionReply {
    #[serde(default, rename = "hasSubscription")]
    has_subscription: bool,
    #[serde(default)]
    subscription: Option<SubscriptionDetail>,
}

#[derive(Deserialize)]
struct SubscriptionDetail {
    #[serde(default)]
    status: Option<String>,
    #[serde(default, rename = "currentPeriodEnd")]
    current_period_end: Option<serde_json::Value>,
    #[serde(default, rename = "cancelAtPeriodEnd")]
    cancel_at_period_end: bool,
}

fn plan_from(reply: Option<SubscriptionReply>, staff: bool) -> Plan {
    match reply {
        Some(reply) if reply.has_subscription => {
            let detail = reply.subscription;
            let trial = detail
                .as_ref()
                .and_then(|d| d.status.as_deref())
                .is_some_and(|s| s == "trialing");
            let until = detail
                .filter(|d| d.cancel_at_period_end)
                .and_then(|d| d.current_period_end)
                .and_then(|end| end.as_str().and_then(day_of).map(str::to_string));
            Plan::Pro {
                via: if trial {
                    ProVia::Trial
                } else {
                    ProVia::Subscription
                },
                until,
            }
        }
        _ if staff => Plan::Pro {
            via: ProVia::Staff,
            until: None,
        },
        Some(_) => Plan::Free,
        None => Plan::Unknown,
    }
}

/// `YYYY-MM-DD` from the start of a date the site wrote, if it is one.
fn day_of(text: &str) -> Option<&str> {
    let day = text.get(..10)?;
    let b = day.as_bytes();
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    (digits(0..4) && b[4] == b'-' && digits(5..7) && b[7] == b'-' && digits(8..10)).then_some(day)
}

/// The name to show: the display name, else the hidden address.
fn shown_name(display_name: Option<&str>, email: Option<&str>) -> String {
    match display_name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(name) => name.chars().take(60).collect(),
        None => email
            .map(mask_email)
            .unwrap_or_else(|| "Your account".into()),
    }
}

/// `dishairano@example.com` becomes `dis…@example.com`; a first part of
/// three characters or fewer keeps only its first.
pub fn mask_email(email: &str) -> String {
    let Some((local, domain)) = email.rsplit_once('@') else {
        return "…".into();
    };
    let keep = if local.chars().count() > 3 { 3 } else { 1 };
    let start: String = local.chars().take(keep).collect();
    format!("{start}…@{domain}")
}

/// What a token says about itself. Read only to decide when to renew and
/// whether a renewal belongs to the same account; the site checks the
/// signature on every request, so nothing here is trusted beyond that.
struct Claims {
    exp: i64,
    account: serde_json::Map<String, serde_json::Value>,
}

/// The claims that name the account. Two tokens that differ in any of
/// these belong to different accounts.
const ACCOUNT_CLAIMS: [&str; 4] = ["userId", "user_id", "sub", "email"];

impl Claims {
    fn of(token: &str) -> Option<Self> {
        let mut parts = token.split('.');
        let (_header, payload, _signature) = (parts.next()?, parts.next()?, parts.next()?);
        if parts.next().is_some() {
            return None;
        }
        let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
        let serde_json::Value::Object(claims) = serde_json::from_slice(&bytes).ok()? else {
            return None;
        };
        let exp = claims.get("exp")?.as_f64().filter(|f| f.is_finite())? as i64;
        let account = ACCOUNT_CLAIMS
            .iter()
            .filter_map(|key| claims.get(*key).map(|v| (key.to_string(), v.clone())))
            .collect();
        Some(Claims { exp, account })
    }

    fn valid_at(&self, now: i64) -> bool {
        self.exp.saturating_sub(EXPIRY_MARGIN_SECS) > now
    }

    fn same_account(&self, other: &Claims) -> bool {
        self.account
            .iter()
            .all(|(key, value)| other.account.get(key).is_none_or(|o| o == value))
    }
}

/// A renewal is used only when it is a valid token of the same account
/// that does not run out before the one it replaces.
fn is_renewal_of(new: &str, old: &str, now: i64) -> bool {
    match (Claims::of(new), Claims::of(old)) {
        (Some(n), Some(o)) => n.valid_at(now) && n.same_account(&o) && n.exp >= o.exp,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::{IntoResponse, Response};
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    const NOW: i64 = 1_793_000_000;
    const DAY: i64 = 24 * 60 * 60;

    /// A token the way the site signs them; the signature is not checked
    /// on this side.
    fn jwt(user: u64, exp: i64) -> String {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"HS256","typ":"JWT"}"#);
        let claims = serde_json::json!({
            "userId": user, "email": format!("user{user}@example.com"), "isAdmin": false,
            "tv": 0, "iat": exp - 7 * DAY, "exp": exp,
        });
        let payload = URL_SAFE_NO_PAD.encode(claims.to_string());
        format!("{header}.{payload}.c2lnbmF0dXJl")
    }

    /// What the stand-in site saw and how it answers.
    #[derive(Default)]
    struct Seen {
        bearer: Vec<(String, String)>,
        login_bodies: Vec<serde_json::Value>,
    }

    #[derive(Clone)]
    struct Answers {
        login: (u16, String, Option<&'static str>),
        refresh: (u16, String),
        me: (u16, String),
        subscription: (u16, String),
    }

    impl Answers {
        fn signed_in(token_for_login: &str, renewed: &str) -> Self {
            Answers {
                login: (
                    200,
                    serde_json::json!({ "success": true, "token": token_for_login }).to_string(),
                    None,
                ),
                refresh: (200, serde_json::json!({ "token": renewed }).to_string()),
                me: (
                    200,
                    r#"{"id":8,"email":"dishairano@hardwavestudios.com","displayName":"Dishairano",
                        "avatarUrl":null,"isAdmin":false,"role":"user"}"#
                        .into(),
                ),
                subscription: (
                    200,
                    r#"{"success":true,"hasSubscription":true,"subscription":{"status":"active",
                        "currentPeriodEnd":"2026-12-12T00:00:00.000Z","cancelAtPeriodEnd":false}}"#
                        .into(),
                ),
            }
        }
    }

    fn bearer(headers: &HeaderMap) -> String {
        headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string()
    }

    fn reply(status: u16, body: String) -> Response {
        (StatusCode::from_u16(status).unwrap(), body).into_response()
    }

    #[derive(Clone)]
    struct Fake {
        answers: Answers,
        seen: Arc<Mutex<Seen>>,
    }

    impl Fake {
        fn saw(&self, endpoint: &str, headers: &HeaderMap) {
            self.seen
                .lock()
                .unwrap()
                .bearer
                .push((endpoint.to_string(), bearer(headers)));
        }
    }

    async fn login(State(f): State<Fake>, Json(body): Json<serde_json::Value>) -> Response {
        f.seen.lock().unwrap().login_bodies.push(body);
        let (status, body, retry) = f.answers.login.clone();
        let mut r = reply(status, body);
        if let Some(secs) = retry {
            r.headers_mut().insert("retry-after", secs.parse().unwrap());
        }
        r
    }

    async fn refresh(State(f): State<Fake>, h: HeaderMap) -> Response {
        f.saw("refresh", &h);
        reply(f.answers.refresh.0, f.answers.refresh.1.clone())
    }

    async fn me(State(f): State<Fake>, h: HeaderMap) -> Response {
        f.saw("me", &h);
        reply(f.answers.me.0, f.answers.me.1.clone())
    }

    async fn subscription(State(f): State<Fake>, h: HeaderMap) -> Response {
        f.saw("subscription", &h);
        reply(f.answers.subscription.0, f.answers.subscription.1.clone())
    }

    async fn logout(State(f): State<Fake>, h: HeaderMap) -> Response {
        f.saw("logout", &h);
        reply(200, r#"{"success":true}"#.into())
    }

    /// A stand-in for hardwavestudios.com on this machine.
    async fn fake_site(answers: Answers) -> (Site, Arc<Mutex<Seen>>) {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let app = Router::new()
            .route("/api/auth/login", post(login))
            .route("/api/auth/refresh", post(refresh))
            .route("/api/auth/me", get(me))
            .route("/api/subscription", get(subscription))
            .route("/api/auth/logout", post(logout))
            .with_state(Fake {
                answers,
                seen: seen.clone(),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (Site::at(&format!("http://{addr}")).unwrap(), seen)
    }

    fn token_file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hw-account-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("hardwave").join("auth_token")
    }

    fn signed_in_as_dishairano(plan: Plan) -> AccountStatus {
        AccountStatus::SignedIn(Account {
            name: "Dishairano".into(),
            email: "dis…@hardwavestudios.com".into(),
            plan,
        })
    }

    #[tokio::test]
    async fn signing_in_keeps_the_token_in_the_shared_file_and_shows_the_account() {
        let fresh = jwt(8, NOW + 7 * DAY);
        let (site, seen) = fake_site(Answers::signed_in(&fresh, "unused")).await;
        let file = token_file("sign-in");

        let status = site
            .sign_in_here(&file, " dishairano@hardwavestudios.com ", "hunter2", NOW)
            .await
            .expect("signed in");

        assert_eq!(
            status,
            signed_in_as_dishairano(Plan::Pro {
                via: ProVia::Subscription,
                until: None
            })
        );
        // The very file the plug-ins and the Suite read.
        assert_eq!(std::fs::read_to_string(&file).unwrap(), fresh);
        let seen = seen.lock().unwrap();
        assert_eq!(
            seen.login_bodies,
            vec![
                serde_json::json!({ "email": "dishairano@hardwavestudios.com", "password": "hunter2" })
            ]
        );
        // A fresh token is not renewed; the token goes as a Bearer header.
        let expected = format!("Bearer {fresh}");
        assert_eq!(
            seen.bearer,
            vec![
                ("me".to_string(), expected.clone()),
                ("subscription".to_string(), expected)
            ]
        );
    }

    #[tokio::test]
    async fn a_refused_sign_in_says_why_and_writes_nothing() {
        let cases: [(u16, Option<&'static str>, &str); 6] = [
            (401, None, WRONG_PASSWORD),
            (400, None, MISSING_FIELDS),
            (403, None, DEACTIVATED),
            (
                429,
                Some("540"),
                "Too many tries. Wait 9 minutes and try again.",
            ),
            (
                429,
                None,
                "Too many tries. Wait a few minutes and try again.",
            ),
            (502, None, SITE_TROUBLE),
        ];
        for (status, retry, message) in cases {
            let mut answers = Answers::signed_in("unused", "unused");
            answers.login = (
                status,
                r#"{"success":false,"error":"secret.detail"}"#.into(),
                retry,
            );
            let (site, _) = fake_site(answers).await;
            let file = token_file(&format!("refused-{status}"));
            let err = site
                .sign_in_here(&file, "a@b.c", "hunter2", NOW)
                .await
                .unwrap_err();
            assert_eq!(err, message, "HTTP {status}");
            assert!(!err.contains("secret.detail") && !err.contains("hunter2"));
            assert!(!file.exists());
        }
    }

    #[tokio::test]
    async fn a_reply_without_a_usable_token_is_not_a_sign_in() {
        for body in [
            r#"{"success":true}"#,
            r#"{"success":true,"token":""}"#,
            r#"{"success":false,"token":"aaa.bbb.ccc"}"#,
            r#"{"success":true,"token":"aaa.bbb\r\nX-Evil: 1"}"#,
            "<html>secret.detail</html>",
        ] {
            let mut answers = Answers::signed_in("unused", "unused");
            answers.login = (200, body.into(), None);
            let (site, _) = fake_site(answers).await;
            let file = token_file("no-token");
            let err = site
                .sign_in_here(&file, "a@b.c", "pw", NOW)
                .await
                .unwrap_err();
            assert_eq!(err, NO_TOKEN_IN_REPLY, "{body}");
            assert!(!file.exists());
        }
    }

    #[tokio::test]
    async fn nothing_is_sent_without_an_address_and_a_password() {
        // Port 9 on this machine: nothing listens, and nothing is tried.
        let site = Site::at("http://127.0.0.1:9").unwrap();
        assert_eq!(site.sign_in(" ", "pw").await.unwrap_err(), MISSING_FIELDS);
        assert_eq!(site.sign_in("a@b.c", "").await.unwrap_err(), MISSING_FIELDS);
        assert_eq!(
            site.sign_in("no-at-sign", "pw").await.unwrap_err(),
            "That does not look like an e-mail address."
        );
    }

    #[tokio::test]
    async fn no_site_at_all_says_so_plainly() {
        let site = Site::at("http://127.0.0.1:9").unwrap();
        let file = token_file("offline");
        assert_eq!(
            site.sign_in_here(&file, "a@b.c", "pw", NOW)
                .await
                .unwrap_err(),
            UNREACHABLE
        );
        // Signed in already, but the site cannot be asked: not signed out.
        shared_token::write(&file, &jwt(8, NOW + 6 * DAY + 3600)).unwrap();
        assert_eq!(site.check(&file, NOW).await, AccountStatus::Unreachable);
        assert!(file.exists());
    }

    #[tokio::test]
    async fn an_old_token_is_renewed_and_the_new_one_shared() {
        let old = jwt(8, NOW + 2 * DAY);
        let renewed = jwt(8, NOW + 7 * DAY);
        let (site, seen) = fake_site(Answers::signed_in("unused", &renewed)).await;
        let file = token_file("renew");
        shared_token::write(&file, &old).unwrap();

        let status = site.check(&file, NOW).await;

        assert!(matches!(status, AccountStatus::SignedIn(_)));
        assert_eq!(shared_token::read(&file).as_deref(), Some(renewed.as_str()));
        let seen = seen.lock().unwrap();
        assert_eq!(seen.bearer[0], ("refresh".into(), format!("Bearer {old}")));
        // Asked with the renewed token from then on.
        assert_eq!(seen.bearer[1], ("me".into(), format!("Bearer {renewed}")));
    }

    #[tokio::test]
    async fn a_renewal_for_someone_else_or_shorter_is_thrown_away() {
        let old = jwt(8, NOW + 2 * DAY);
        for bad in [jwt(9, NOW + 7 * DAY), jwt(8, NOW + DAY), "not-a-jwt".into()] {
            let (site, _) = fake_site(Answers::signed_in("unused", &bad)).await;
            let file = token_file("bad-renewal");
            shared_token::write(&file, &old).unwrap();
            site.check(&file, NOW).await;
            assert_eq!(shared_token::read(&file).as_deref(), Some(old.as_str()));
        }
    }

    #[tokio::test]
    async fn a_failed_renewal_keeps_the_old_token_and_carries_on() {
        let old = jwt(8, NOW + 2 * DAY);
        let mut answers = Answers::signed_in("unused", "unused");
        answers.refresh = (500, "oops".into());
        let (site, _) = fake_site(answers).await;
        let file = token_file("renew-fails");
        shared_token::write(&file, &old).unwrap();
        assert!(matches!(
            site.check(&file, NOW).await,
            AccountStatus::SignedIn(_)
        ));
        assert_eq!(shared_token::read(&file).as_deref(), Some(old.as_str()));
    }

    #[tokio::test]
    async fn a_run_out_or_refused_sign_in_ends_without_deleting_the_file() {
        // Run out: not even sent.
        let (site, seen) = fake_site(Answers::signed_in("unused", "unused")).await;
        let file = token_file("ended");
        let expired = jwt(8, NOW - 1);
        shared_token::write(&file, &expired).unwrap();
        assert_eq!(site.check(&file, NOW).await, AccountStatus::Ended);
        assert!(seen.lock().unwrap().bearer.is_empty());
        assert_eq!(shared_token::read(&file).as_deref(), Some(expired.as_str()));

        // Refused by the site (a password change, "sign out everywhere").
        let mut answers = Answers::signed_in("unused", "unused");
        answers.me = (401, r#"{"error":"Unauthorized"}"#.into());
        let (site, _) = fake_site(answers).await;
        let valid = jwt(8, NOW + 7 * DAY);
        shared_token::write(&file, &valid).unwrap();
        assert_eq!(site.check(&file, NOW).await, AccountStatus::Ended);
        assert_eq!(shared_token::read(&file).as_deref(), Some(valid.as_str()));
    }

    #[tokio::test]
    async fn no_file_is_signed_out_and_asks_nothing() {
        let (site, seen) = fake_site(Answers::signed_in("unused", "unused")).await;
        let file = token_file("none");
        assert_eq!(site.check(&file, NOW).await, AccountStatus::SignedOut);
        assert!(seen.lock().unwrap().bearer.is_empty());
    }

    #[tokio::test]
    async fn a_subscription_that_cannot_be_read_is_unknown_never_free() {
        let mut answers = Answers::signed_in("unused", "unused");
        answers.subscription = (500, "oops".into());
        let (site, _) = fake_site(answers).await;
        let file = token_file("plan-unknown");
        shared_token::write(&file, &jwt(8, NOW + 7 * DAY)).unwrap();
        assert_eq!(
            site.check(&file, NOW).await,
            signed_in_as_dishairano(Plan::Unknown)
        );
    }

    #[tokio::test]
    async fn signing_out_tells_the_site_with_the_token_in_a_header() {
        let (site, seen) = fake_site(Answers::signed_in("unused", "unused")).await;
        site.tell_signed_out("aaa.bbb.ccc").await;
        assert_eq!(
            seen.lock().unwrap().bearer,
            vec![("logout".to_string(), "Bearer aaa.bbb.ccc".to_string())]
        );
    }

    #[test]
    fn the_plan_reads_the_sites_own_answers() {
        let read = |body: &str| serde_json::from_slice::<SubscriptionReply>(body.as_bytes()).ok();
        // No subscription: what /api/subscription sends, field for field.
        let free = read(
            r#"{"success":true,"hasSubscription":false,"hasActiveTrial":false,
                "betaEligible":false,"trials":[],"subscription":null}"#,
        );
        assert_eq!(plan_from(free, false), Plan::Free);
        let trial = read(
            r#"{"success":true,"hasSubscription":true,"subscription":{"status":"trialing",
                "currentPeriodEnd":"2026-11-20T10:00:00.000Z","cancelAtPeriodEnd":false}}"#,
        );
        assert_eq!(
            plan_from(trial, false),
            Plan::Pro {
                via: ProVia::Trial,
                until: None
            }
        );
        let cancelled = read(
            r#"{"success":true,"hasSubscription":true,"subscription":{"status":"active",
                "currentPeriodEnd":"2026-12-12T00:00:00.000Z","cancelAtPeriodEnd":true}}"#,
        );
        assert_eq!(
            plan_from(cancelled, false),
            Plan::Pro {
                via: ProVia::Subscription,
                until: Some("2026-12-12".into())
            }
        );
        // A cancelled subscription past its end: the site says no subscription.
        let ended = read(
            r#"{"success":true,"hasSubscription":false,"subscription":{"status":"canceled",
                "currentPeriodEnd":"2026-10-01T00:00:00.000Z","cancelAtPeriodEnd":true}}"#,
        );
        assert_eq!(plan_from(ended, false), Plan::Free);
        // Staff have Pro whatever the subscription says, or if it cannot be read.
        assert_eq!(
            plan_from(read(r#"{"hasSubscription":false}"#), true),
            Plan::Pro {
                via: ProVia::Staff,
                until: None
            }
        );
        assert_eq!(
            plan_from(None, true),
            Plan::Pro {
                via: ProVia::Staff,
                until: None
            }
        );
        assert_eq!(plan_from(None, false), Plan::Unknown);
    }

    #[test]
    fn the_status_reaches_the_window_in_the_shape_it_reads() {
        let json = serde_json::to_value(signed_in_as_dishairano(Plan::Pro {
            via: ProVia::Subscription,
            until: Some("2026-12-12".into()),
        }))
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "state": "signedIn", "name": "Dishairano", "email": "dis…@hardwavestudios.com",
                "plan": { "kind": "pro", "via": "subscription", "until": "2026-12-12" },
            })
        );
        assert_eq!(
            serde_json::to_value(AccountStatus::Ended).unwrap(),
            serde_json::json!({ "state": "ended" })
        );
        assert_eq!(
            serde_json::to_value(Plan::Unknown).unwrap(),
            serde_json::json!({ "kind": "unknown" })
        );
    }

    #[test]
    fn the_address_is_mostly_hidden() {
        assert_eq!(
            mask_email("dishairano@hardwavestudios.com"),
            "dis…@hardwavestudios.com"
        );
        assert_eq!(mask_email("bob@example.com"), "b…@example.com");
        assert_eq!(mask_email("éèàü@x.nl"), "éèà…@x.nl");
        assert_eq!(mask_email("nonsense"), "…");
        assert_eq!(
            shown_name(Some("  "), Some("bob@example.com")),
            "b…@example.com"
        );
        assert_eq!(shown_name(None, None), "Your account");
    }

    #[test]
    fn only_https_or_this_machine() {
        assert!(Site::at("https://hardwavestudios.com").is_ok());
        assert!(Site::at("http://127.0.0.1:4000").is_ok());
        assert!(Site::at("http://hardwavestudios.com").is_err());
        assert!(Site::at("ftp://hardwavestudios.com").is_err());
        assert!(Site::at("not a url").is_err());
    }

    #[test]
    fn a_token_says_when_it_runs_out() {
        let claims = Claims::of(&jwt(8, NOW + DAY)).expect("reads");
        assert_eq!(claims.exp, NOW + DAY);
        assert!(claims.valid_at(NOW));
        assert!(!claims.valid_at(NOW + DAY - 30));
        assert!(Claims::of("aaa.bbb").is_none());
        assert!(Claims::of("a.b.c.d").is_none());
        assert!(Claims::of("x.bm90IGpzb24.y").is_none());
    }
}

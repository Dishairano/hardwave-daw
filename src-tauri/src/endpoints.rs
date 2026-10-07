//! Where the DAW's services live.
//!
//! Each address can be pointed somewhere else for development, but only
//! in a development build. A release build always talks to our own
//! hosts, so an environment variable on someone's machine cannot send
//! their sign-in or their songs to another server.

/// The address in `var` when this is a development build and it is
/// set, otherwise `ours`.
pub fn service_url(var: &str, ours: &str) -> String {
    if cfg!(debug_assertions) {
        if let Ok(value) = std::env::var(var) {
            if !value.trim().is_empty() {
                return value;
            }
        }
    }
    ours.to_string()
}

/// Whether a transfer address handed to us by one of our services may
/// be used: https always; plain http only to this machine, which is
/// where the tests' stand-in services run.
pub fn transfer_allowed(url: &str) -> bool {
    match url::Url::parse(url) {
        Ok(u) if u.scheme() == "https" => u.host_str().is_some(),
        Ok(u) if u.scheme() == "http" => {
            matches!(
                u.host_str(),
                Some("127.0.0.1") | Some("localhost") | Some("[::1]")
            )
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfers_only_over_https_or_to_this_machine() {
        assert!(transfer_allowed(
            "https://hel1.your-objectstorage.com/bucket/key?sig=1"
        ));
        assert!(transfer_allowed("http://127.0.0.1:4000/storage/1"));
        assert!(!transfer_allowed("http://storage.example/key"));
        assert!(!transfer_allowed("file:///etc/passwd"));
        assert!(!transfer_allowed("ftp://x/y"));
        assert!(!transfer_allowed("not a url"));
    }
}

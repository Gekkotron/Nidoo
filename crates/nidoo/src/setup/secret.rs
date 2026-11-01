//! How a `PUT` treats a secret: absent (or `""`) keeps, `null` clears, a string sets.

use nidoo_api::Secret;

/// A new address (`host:port`) needs the secret again: otherwise anyone on the LAN could point a service at
/// their own machine and collect the stored password. With no stored address, any address is
/// new as long as `stored_secret` says a secret is stored (it would be sent there).
pub fn require_for_new_address(
    old: Option<&str>,
    new: &str,
    secret: &Secret,
    stored_secret: bool,
) -> Result<(), &'static str> {
    let moved = match old {
        Some(old) => !old.eq_ignore_ascii_case(new),
        None => stored_secret,
    };
    if moved && *secret == Secret::Keep {
        return Err("Enter the password for the new address");
    }
    Ok(())
}

/// The secret after the input is applied to what is stored.
pub fn apply(stored: Option<String>, input: Secret) -> Option<String> {
    match input {
        Secret::Keep => stored,
        Secret::Clear => None,
        Secret::Set(s) => Some(s),
    }
}

/// A URL's `user:password@` part, taken off: `(the URL without it, user, password)`, the two
/// percent-decoded and `None` when empty. `None` when the URL carries no such part. The rest of
/// the URL is kept as written.
pub fn split_userinfo(url: &str) -> Option<(String, Option<String>, Option<String>)> {
    let (scheme, rest) = url.split_once("://")?;
    // Where a URL parser ends the authority (a backslash counts as `/` for http and https).
    let end = rest.find(['/', '?', '#', '\\']).unwrap_or(rest.len());
    let authority = &rest[..end];
    let at = authority.rfind('@')?;
    let info = &authority[..at];
    let (user, password) = match info.split_once(':') {
        Some((user, password)) => (user, Some(password)),
        None => (info, None),
    };
    let decode = |s: &str| {
        Some(
            percent_encoding::percent_decode_str(s)
                .decode_utf8_lossy()
                .into_owned(),
        )
        .filter(|s| !s.is_empty())
    };
    let bare = format!("{scheme}://{}{}", &authority[at + 1..], &rest[end..]);
    Some((bare, decode(user), password.and_then(decode)))
}

/// `url` without its `user:password@` part, for a read.
pub fn without_userinfo(url: &str) -> String {
    split_userinfo(url).map_or_else(|| url.to_owned(), |(bare, _, _)| bare)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nidoo_api::MqttInput;

    fn input(body: &str) -> Secret {
        let json = format!(r#"{{"host":"h","port":1{body}}}"#);
        serde_json::from_str::<MqttInput>(&json).unwrap().password
    }

    #[test]
    fn absent_is_keep() {
        assert_eq!(input(""), Secret::Keep);
        assert_eq!(
            apply(Some("old".into()), Secret::Keep).as_deref(),
            Some("old")
        );
    }

    #[test]
    fn null_is_clear() {
        assert_eq!(input(r#","password":null"#), Secret::Clear);
        assert_eq!(apply(Some("old".into()), Secret::Clear), None);
    }

    #[test]
    fn string_is_set() {
        assert_eq!(input(r#","password":"new""#), Secret::Set("new".into()));
        assert_eq!(
            apply(Some("old".into()), Secret::Set("new".into())).as_deref(),
            Some("new")
        );
    }

    #[test]
    fn empty_string_is_keep() {
        assert_eq!(input(r#","password":"""#), Secret::Keep);
        // So an empty field sent to a new address still needs the password.
        let err = require_for_new_address(Some("a:1"), "b:1", &input(r#","password":"""#), true);
        assert_eq!(err, Err("Enter the password for the new address"));
    }

    #[test]
    fn userinfo_is_split_off() {
        assert_eq!(split_userinfo("http://192.0.2.5:5000/x"), None);
        assert_eq!(split_userinfo("not a url"), None);
        assert_eq!(
            split_userinfo("http://fu:f%40p@192.0.2.5:5000"),
            Some((
                "http://192.0.2.5:5000".to_owned(),
                Some("fu".to_owned()),
                Some("f@p".to_owned())
            ))
        );
        assert_eq!(
            split_userinfo("https://only@frigate.example.com/a/"),
            Some((
                "https://frigate.example.com/a/".to_owned(),
                Some("only".to_owned()),
                None
            ))
        );
        assert_eq!(
            split_userinfo("http://:pw@192.0.2.5/"),
            Some(("http://192.0.2.5/".to_owned(), None, Some("pw".to_owned())))
        );
    }

    #[test]
    fn new_host_without_password_is_refused() {
        let err =
            require_for_new_address(Some("a.example.com"), "b.example.com", &Secret::Keep, true);
        assert_eq!(err, Err("Enter the password for the new address"));
        assert!(require_for_new_address(Some("a"), "b", &Secret::Set("p".into()), true).is_ok());
        assert!(require_for_new_address(Some("a"), "b", &Secret::Clear, true).is_ok());
    }

    #[test]
    fn same_host_other_port_is_a_new_address() {
        let err = require_for_new_address(Some("h:1883"), "h:1884", &Secret::Keep, true);
        assert!(err.is_err());
    }

    #[test]
    fn same_host_keeps_password() {
        assert!(require_for_new_address(Some("a"), "a", &Secret::Keep, true).is_ok());
        assert!(require_for_new_address(None, "b", &Secret::Keep, false).is_ok());
    }

    #[test]
    fn a_stored_secret_with_no_stored_address_follows_no_new_address() {
        let err = require_for_new_address(None, "b", &Secret::Keep, true);
        assert_eq!(err, Err("Enter the password for the new address"));
        assert!(require_for_new_address(None, "b", &Secret::Set("p".into()), true).is_ok());
        assert!(require_for_new_address(None, "b", &Secret::Clear, true).is_ok());
    }
}

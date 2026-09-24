const SERVICE: &str = "chat.caper.desktop";

pub fn load(origin: &url::Url) -> Result<Option<String>, String> {
    let entry = entry(origin)?;
    match entry.get_password() {
        Ok(token) => Ok(Some(token)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(safe_error(error)),
    }
}

pub fn save(origin: &url::Url, token: &str) -> Result<(), String> {
    entry(origin)?.set_password(token).map_err(safe_error)
}

pub fn delete(origin: &url::Url) -> Result<(), String> {
    let entry = entry(origin)?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(safe_error(error)),
    }
}

fn entry(origin: &url::Url) -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, &credential_key(origin)).map_err(safe_error)
}

fn credential_key(origin: &url::Url) -> String {
    format!("account-session:{}", origin.origin().ascii_serialization())
}

fn safe_error(_: keyring::Error) -> String {
    "The operating system credential store is unavailable.".into()
}

#[cfg(test)]
mod tests {
    use super::credential_key;

    #[test]
    fn credentials_are_isolated_by_canonical_origin() {
        let production = url::Url::parse("https://caper.chat/ignored").unwrap();
        let staging = url::Url::parse("https://staging.caper.chat/").unwrap();
        let alternate_port = url::Url::parse("https://caper.chat:8443/").unwrap();
        assert_ne!(credential_key(&production), credential_key(&staging));
        assert_ne!(credential_key(&production), credential_key(&alternate_port));
        assert!(!credential_key(&production).contains("ignored"));
    }
}

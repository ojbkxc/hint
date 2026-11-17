use crate::error::Result;

const SERVICE: &str = "hint.desktop";

fn entry(provider: &str) -> Result<keyring::Entry> {
    Ok(keyring::Entry::new(SERVICE, provider)?)
}

pub fn save(provider: &str, key: &str) -> Result<()> {
    entry(provider)?.set_password(key)?;
    Ok(())
}

pub fn get(provider: &str) -> Result<String> {
    Ok(entry(provider)?.get_password()?)
}

pub fn has(provider: &str) -> bool {
    keyring::Entry::new(SERVICE, provider)
        .and_then(|entry| entry.get_password())
        .is_ok()
}

#[allow(dead_code)]
pub fn delete(provider: &str) -> Result<()> {
    entry(provider)?.delete_credential()?;
    Ok(())
}

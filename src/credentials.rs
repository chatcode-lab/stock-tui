use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use secrecy::{ExposeSecret, SecretString};

use crate::config::Credentials;

const KEY_NAME: &str = "ALPACA_API_KEY";
const SECRET_NAME: &str = "ALPACA_API_SECRET";
const MAX_CREDENTIAL_LENGTH: usize = 4_096;

#[derive(Debug, thiserror::Error)]
pub enum CredentialsFileError {
    #[error("could not read the stored credentials file")]
    Read,
    #[error("the stored credentials file must contain both Alpaca values")]
    Incomplete,
    #[error("an Alpaca credential is empty or contains unsupported characters")]
    InvalidValue,
    #[error("could not create the credentials directory")]
    CreateDirectory(#[source] std::io::Error),
    #[error("could not write the stored credentials file")]
    Write(#[source] std::io::Error),
}

pub fn load(path: &Path) -> Result<Option<Credentials>, CredentialsFileError> {
    let entries = match dotenvy::from_path_iter(path) {
        Ok(entries) => entries,
        Err(error) if error.not_found() => return Ok(None),
        Err(_) => return Err(CredentialsFileError::Read),
    };
    let mut key = None;
    let mut secret = None;
    for entry in entries {
        let (name, value) = entry.map_err(|_| CredentialsFileError::Read)?;
        match name.as_str() {
            KEY_NAME => key = (!value.is_empty()).then_some(value),
            SECRET_NAME => secret = (!value.is_empty()).then_some(value),
            _ => {}
        }
    }
    match (key, secret) {
        (Some(key), Some(secret)) => Ok(Some(Credentials {
            key: SecretString::from(key),
            secret: SecretString::from(secret),
        })),
        (None, None) => Ok(None),
        _ => Err(CredentialsFileError::Incomplete),
    }
}

pub fn save(path: &Path, credentials: &Credentials) -> Result<(), CredentialsFileError> {
    let key = credentials.key.expose_secret();
    let secret = credentials.secret.expose_secret();
    if !is_storable(key) || !is_storable(secret) {
        return Err(CredentialsFileError::InvalidValue);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(CredentialsFileError::CreateDirectory)?;
    }
    let contents = format!(
        "{KEY_NAME}={}\n{SECRET_NAME}={}\n",
        quote_dotenv(key),
        quote_dotenv(secret)
    );
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path).map_err(CredentialsFileError::Write)?;
    file.write_all(contents.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(CredentialsFileError::Write)?;
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(CredentialsFileError::Write)?;
    Ok(())
}

fn is_storable(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_CREDENTIAL_LENGTH
        && !value
            .chars()
            .any(|character| character == '\0' || character == '\r' || character == '\n')
}

fn quote_dotenv(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for character in value.chars() {
        match character {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '$' => quoted.push_str("\\$"),
            _ => quoted.push(character),
        }
    }
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use secrecy::ExposeSecret;
    use tempfile::TempDir;

    use super::*;

    fn credentials(key: &str, secret: &str) -> Credentials {
        Credentials {
            key: SecretString::from(key.to_owned()),
            secret: SecretString::from(secret.to_owned()),
        }
    }

    #[test]
    fn missing_and_empty_files_have_no_stored_credentials() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("credentials.env");
        assert!(load(&path).expect("missing credentials").is_none());
        fs::write(&path, "# no credentials\n").expect("empty credentials file");
        assert!(load(&path).expect("empty credentials").is_none());
    }

    #[test]
    fn credentials_round_trip_through_dotenv_escaping() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("credentials.env");
        let expected = credentials("key-$-value", r#"secret\"$#value"#);
        save(&path, &expected).expect("save credentials");
        let loaded = load(&path)
            .expect("load credentials")
            .expect("stored credentials");
        assert_eq!(loaded.key.expose_secret(), expected.key.expose_secret());
        assert_eq!(
            loaded.secret.expose_secret(),
            expected.secret.expose_secret()
        );
    }

    #[test]
    fn incomplete_or_unstorable_credentials_are_rejected_without_values() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("credentials.env");
        fs::write(&path, "ALPACA_API_KEY=only-one-value\n").expect("partial file");
        let error = load(&path).expect_err("partial credentials must fail");
        assert!(!format!("{error:?}").contains("only-one-value"));

        fs::write(
            &path,
            "ALPACA_API_KEY=\"secret-value\nALPACA_API_SECRET=other\n",
        )
        .expect("malformed file");
        let error = load(&path).expect_err("malformed credentials must fail");
        assert!(!format!("{error:?}").contains("secret-value"));

        let error = save(&path, &credentials("", "secret-value")).expect_err("empty key must fail");
        assert!(!format!("{error:?}").contains("secret-value"));
    }

    #[cfg(unix)]
    #[test]
    fn saved_file_is_owner_only_even_when_replacing_a_looser_file() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("credentials.env");
        fs::write(&path, "old").expect("seed file");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("loosen permissions");
        save(&path, &credentials("key", "secret")).expect("save credentials");
        let mode = fs::metadata(path)
            .expect("credential metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}

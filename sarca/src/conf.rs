//! Load `sarca.conf` (KEY=VALUE) into the process environment.
//!
//! Existing environment variables win (are not overwritten). Looks next to the
//! binary and in the current working directory. Migrates legacy `.env` once.

use std::{
    env,
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};

pub const CONF_NAME: &str = "sarca.conf";
pub const LEGACY_ENV_NAME: &str = ".env";

/// Apply the first readable conf file found. Returns the path that was loaded.
pub fn load_sarca_conf() -> Option<PathBuf> {
    for path in conf_candidates() {
        let path = migrate_legacy_env(&path);
        if !path.is_file() {
            continue;
        }
        match apply_conf_file(&path) {
            Ok(n) => {
                eprintln!("loaded config from {} ({n} keys)", path.display());
                return Some(path);
            },
            Err(e) => {
                eprintln!("warning: could not read {}: {e}", path.display());
            },
        }
    }
    None
}

/// First existing `sarca.conf` among the usual candidates (may be None).
pub fn resolve_conf_path() -> Option<PathBuf> {
    for path in conf_candidates() {
        let path = migrate_legacy_env(&path);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

/// Best-effort upsert of KEY=VALUE lines in `sarca.conf`. Returns Ok(false) if no file / not
/// writable.
pub fn upsert_conf_keys(keys: &[(&str, &str)]) -> Result<bool, String> {
    let Some(path) = resolve_conf_path() else {
        return Ok(false);
    };
    upsert_conf_keys_at(&path, keys)?;
    Ok(true)
}

fn upsert_conf_keys_at(path: &Path, keys: &[(&str, &str)]) -> Result<(), String> {
    let original = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut lines: Vec<String> = original.lines().map(std::string::ToString::to_string).collect();
    let mut found = vec![false; keys.len()];

    for line in &mut lines {
        let trimmed = line.trim().to_owned();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((raw_key, _)) = trimmed.split_once('=') else {
            continue;
        };
        let key = raw_key.trim();
        for (i, (k, v)) in keys.iter().enumerate() {
            if key == *k {
                *line = format!("{k}={v}");
                found[i] = true;
            }
        }
    }

    for (i, (k, v)) in keys.iter().enumerate() {
        if !found[i] {
            lines.push(format!("{k}={v}"));
        }
    }

    let mut out = lines.join("\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    fs::write(path, out).map_err(|e| e.to_string())
}

fn conf_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(exe) = env::current_exe() {
        if let Some(dir) = exe.parent() {
            out.push(dir.join(CONF_NAME));
        }
    }
    out.push(PathBuf::from(CONF_NAME));
    out
}

/// If `sarca.conf` is missing but legacy `.env` exists beside it, rename.
fn migrate_legacy_env(conf_path: &Path) -> PathBuf {
    if conf_path.is_file() {
        return conf_path.to_path_buf();
    }
    let legacy = conf_path.parent().unwrap_or_else(|| Path::new(".")).join(LEGACY_ENV_NAME);
    if legacy.is_file() {
        match fs::rename(&legacy, conf_path) {
            Ok(()) => {
                eprintln!("migrated {} → {}", legacy.display(), conf_path.display());
            },
            Err(e) => {
                eprintln!(
                    "warning: could not migrate {} → {}: {e}",
                    legacy.display(),
                    conf_path.display()
                );
                return legacy;
            },
        }
    }
    conf_path.to_path_buf()
}

fn apply_conf_file(path: &Path) -> Result<usize, String> {
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut applied = 0usize;
    for line in BufReader::new(file).lines() {
        let line = line.map_err(|e| e.to_string())?;
        if let Some((key, value)) = parse_conf_line(&line) {
            // Do not override variables already present in the environment.
            if env::var_os(&key).is_none() {
                env::set_var(&key, &value);
                applied += 1;
            }
        }
    }
    Ok(applied)
}

/// Parse a single conf line into (KEY, VALUE). Comments and blanks → None.
pub fn parse_conf_line(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let (key, value) = trimmed.split_once('=')?;
    let key = key.trim();
    if key.is_empty() {
        return None;
    }
    let mut value = value.trim().to_string();
    // Strip optional matching quotes.
    if (value.starts_with('"') && value.ends_with('"') && value.len() >= 2)
        || (value.starts_with('\'') && value.ends_with('\'') && value.len() >= 2)
    {
        value = value[1..value.len() - 1].to_string();
    }
    Some((key.to_string(), value))
}

#[cfg(test)]
mod tests {
    use std::{io::Write, sync::Mutex};

    use super::*;

    // serialize env-mutating tests
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn parse_skips_comments_and_blanks() {
        assert_eq!(parse_conf_line(""), None);
        assert_eq!(parse_conf_line("   "), None);
        assert_eq!(parse_conf_line("# comment"), None);
        assert_eq!(parse_conf_line("  # x"), None);
    }

    #[test]
    fn parse_key_value_and_quotes() {
        assert_eq!(parse_conf_line("PORT=8001"), Some(("PORT".into(), "8001".into())));
        assert_eq!(
            parse_conf_line("  NAME = \"hello world\" "),
            Some(("NAME".into(), "hello world".into()))
        );
        assert_eq!(parse_conf_line("TOKEN='abc=def'"), Some(("TOKEN".into(), "abc=def".into())));
        assert_eq!(parse_conf_line("EMPTY="), Some(("EMPTY".into(), String::new())));
    }

    #[test]
    fn apply_does_not_override_existing_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempfile_dir();
        let conf = dir.join(CONF_NAME);
        let mut f = fs::File::create(&conf).unwrap();
        writeln!(f, "SARCA_TEST_PORT=9999").unwrap();
        writeln!(f, "SARCA_TEST_ONLY_FROM_FILE=from-file").unwrap();

        env::set_var("SARCA_TEST_PORT", "1111");
        env::remove_var("SARCA_TEST_ONLY_FROM_FILE");

        let n = apply_conf_file(&conf).unwrap();
        assert_eq!(n, 1);
        assert_eq!(env::var("SARCA_TEST_PORT").unwrap(), "1111");
        assert_eq!(env::var("SARCA_TEST_ONLY_FROM_FILE").unwrap(), "from-file");

        env::remove_var("SARCA_TEST_PORT");
        env::remove_var("SARCA_TEST_ONLY_FROM_FILE");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn migrate_renames_legacy_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempfile_dir();
        let legacy = dir.join(LEGACY_ENV_NAME);
        let conf = dir.join(CONF_NAME);
        fs::write(&legacy, "PORT=8000\n").unwrap();
        assert!(!conf.exists());

        let resolved = migrate_legacy_env(&conf);
        assert_eq!(resolved, conf);
        assert!(conf.is_file());
        assert!(!legacy.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn upsert_replaces_and_appends_keys() {
        let dir = tempfile_dir();
        let conf = dir.join(CONF_NAME);
        fs::write(&conf, "PORT=8000\nTELEGRAM_API_ID=old\n").unwrap();
        upsert_conf_keys_at(
            &conf,
            &[("TELEGRAM_API_ID", "12345"), ("TELEGRAM_API_HASH", "abcdef")],
        )
        .unwrap();
        let text = fs::read_to_string(&conf).unwrap();
        assert!(text.contains("TELEGRAM_API_ID=12345"));
        assert!(text.contains("TELEGRAM_API_HASH=abcdef"));
        assert!(text.contains("PORT=8000"));
        assert!(!text.contains("TELEGRAM_API_ID=old"));
        let _ = fs::remove_dir_all(&dir);
    }

    fn tempfile_dir() -> PathBuf {
        let dir = env::temp_dir().join(format!("sarca-conf-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}

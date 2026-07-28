//! Compile-time / unit checks that remote Settings ACL stays correct.
//!
//! Root-cause regression: `remote.urls` of `http://*` does **not** match LAN
//! URLs with an explicit port (`http://192.168.x.x:8080/`), so Tauri denied
//! `default_gallery_path` / `pick_local_folder` with "not allowed by ACL".

use crate::remote_ipc::REMOTE_SETTINGS_COMMANDS;

/// Permission identifiers that must appear in `capabilities/default.json`.
pub const REQUIRED_ALLOW_PERMISSIONS: &[&str] = &[
    "allow-platform-label",
    "allow-get-session",
    "allow-update-session",
    "allow-connect",
    "allow-disconnect",
    "allow-open-app",
    "allow-open-sync-settings",
    "allow-pick-local-folder",
    "allow-default-gallery-path",
    "allow-list-storages",
    "allow-ensure-remote-folder",
    "allow-list-bindings",
    "allow-add-binding",
    "allow-remove-binding",
    "allow-sync-now",
    "allow-sync-statuses",
    "allow-sync-transfer-queue",
    "allow-get-client-prefs",
    "allow-set-client-prefs",
    "allow-export-logs",
    "allow-is-on-wifi",
    "allow-get-about",
    "allow-get-cache-size",
    "allow-clear-local-cache",
];

fn allow_perm_for_command(cmd: &str) -> String {
    format!("allow-{}", cmd.replace('_', "-"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use tauri_utils::acl::RemoteUrlPattern;

    fn capability_json() -> Value {
        serde_json::from_str(include_str!("../capabilities/default.json"))
            .expect("capabilities/default.json must parse")
    }

    #[test]
    fn capability_allow_list_covers_remote_settings_commands() {
        let cap = capability_json();
        let perms: Vec<&str> = cap["permissions"]
            .as_array()
            .expect("permissions array")
            .iter()
            .filter_map(|v| v.as_str())
            .collect();

        for cmd in REMOTE_SETTINGS_COMMANDS {
            // connect is registered for the shell but not always in dispatch list
            let allow = allow_perm_for_command(cmd);
            assert!(
                perms.contains(&allow.as_str()),
                "capabilities/default.json missing {allow} (for command {cmd})"
            );
        }

        for allow in REQUIRED_ALLOW_PERMISSIONS {
            assert!(
                perms.contains(allow),
                "capabilities/default.json missing required permission {allow}"
            );
        }
    }

    #[test]
    fn capability_remote_urls_match_lan_hosts_with_ports() {
        let cap = capability_json();
        let urls: Vec<&str> = cap["remote"]["urls"]
            .as_array()
            .expect("remote.urls")
            .iter()
            .filter_map(|v| v.as_str())
            .collect();

        assert!(
            urls.iter().any(|u| u.contains("*:*")),
            "remote.urls must include host:port wildcards (*:*); got {urls:?}. \
             Plain http://* does not match http://192.168.1.5:8080/"
        );

        let patterns: Vec<RemoteUrlPattern> = urls
            .iter()
            .map(|u| {
                u.parse()
                    .unwrap_or_else(|e| panic!("invalid remote URL pattern {u}: {e:?}"))
            })
            .collect();

        let samples = [
            "http://192.168.1.5:8080/",
            "http://192.168.1.5:8080/settings",
            "http://10.0.0.2:3000/storages",
            "http://localhost:8080/",
            "https://sarca.example.com/",
            "https://sarca.example.com:8443/app",
            "http://10.0.0.2/",
        ];

        for sample in samples {
            let url = url::Url::parse(sample).unwrap();
            let ok = patterns.iter().any(|p| p.test(&url));
            assert!(
                ok,
                "no remote.urls pattern matches {sample} (patterns={urls:?})"
            );
        }
    }

    #[test]
    fn plain_star_host_pattern_rejects_ports_regression() {
        // Documents the bug we fixed: http://* alone is insufficient.
        let broken: RemoteUrlPattern = "http://*".parse().unwrap();
        let lan = url::Url::parse("http://192.168.1.5:8080/").unwrap();
        assert!(
            !broken.test(&lan),
            "precondition failed: http://* unexpectedly matched a :port URL"
        );
        let fixed: RemoteUrlPattern = "http://*:*/*".parse().unwrap();
        assert!(fixed.test(&lan));
    }

    #[test]
    fn allow_permission_names_match_command_snake_case() {
        assert_eq!(
            allow_perm_for_command("default_gallery_path"),
            "allow-default-gallery-path"
        );
        assert_eq!(
            allow_perm_for_command("pick_local_folder"),
            "allow-pick-local-folder"
        );
        assert!(
            REQUIRED_ALLOW_PERMISSIONS.contains(&"allow-connect"),
            "connect must remain in the required allow list"
        );
    }

    #[test]
    fn build_rs_registers_same_commands_as_dispatch() {
        let build = include_str!("../build.rs");
        for cmd in REMOTE_SETTINGS_COMMANDS {
            assert!(
                build.contains(&format!("\"{cmd}\"")),
                "build.rs AppManifest missing command {cmd}"
            );
        }
        // Shell connect is ACL-registered even though remote dispatch omits it.
        assert!(build.contains("\"connect\""));
    }
}

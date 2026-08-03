//! Unit checks that the native command ACL stays deny-by-default.
//!
//! `capabilities/default.json` used to carry
//! `remote.urls: ["http://*:*/*", "https://*:*/*"]`, which injected
//! `__TAURI_INTERNALS__` — and with it the whole native command surface — into
//! every http(s) page the WebView ever loaded, including third-party pages
//! reached by redirect. That file now grants the bundled shell only; the one
//! origin the user connected to gets a narrower capability built at runtime by
//! [`crate::remote_ipc::grant_remote_capability`].
//!
//! These tests pin both halves: the static file must stay local-only and
//! wildcard-free, and the runtime grant must resolve real permission
//! identifiers for exactly one origin.

use crate::remote_ipc::{REMOTE_SETTINGS_COMMANDS, SHELL_ONLY_COMMANDS};

/// Permission identifiers that must appear in `capabilities/default.json`.
pub const REQUIRED_ALLOW_PERMISSIONS: &[&str] = &[
    "allow-platform-label",
    "allow-device-label",
    "allow-get-session",
    "allow-get-url-history",
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
    "allow-set-binding-enabled",
    "allow-update-binding-local-path",
    "allow-update-binding-remote-root",
    "allow-sync-now",
    "allow-sync-statuses",
    "allow-sync-transfer-queue",
    "allow-get-client-prefs",
    "allow-set-client-prefs",
    "allow-verify-app-lock-pin",
    "allow-export-logs",
    "allow-is-on-wifi",
    "allow-get-about",
    "allow-get-cache-size",
    "allow-clear-local-cache",
    "allow-cache-get-preview",
    "allow-cache-put-preview",
];

fn allow_perm_for_command(cmd: &str) -> String {
    format!("allow-{}", cmd.replace('_', "-"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote_ipc::{origin_url_pattern, permission_for};
    use serde_json::Value;
    use tauri_utils::acl::RemoteUrlPattern;

    fn capability_json() -> Value {
        serde_json::from_str(include_str!("../capabilities/default.json"))
            .expect("capabilities/default.json must parse")
    }

    fn capability_permissions(cap: &Value) -> Vec<&str> {
        cap["permissions"]
            .as_array()
            .expect("permissions array")
            .iter()
            .filter_map(|v| v.as_str())
            .collect()
    }

    #[test]
    fn capability_allow_list_covers_remote_settings_commands() {
        let cap = capability_json();
        let perms = capability_permissions(&cap);

        for cmd in REMOTE_SETTINGS_COMMANDS {
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
    fn capability_grants_nothing_to_remote_pages() {
        let cap = capability_json();

        assert!(
            cap.get("remote").is_none(),
            "capabilities/default.json must not carry a `remote` block: it would \
             hand __TAURI_INTERNALS__ to pages the WebView loads. Remote access \
             belongs in remote_ipc::grant_remote_capability, scoped to the \
             connected origin. Got {:?}",
            cap.get("remote")
        );
        assert_eq!(
            cap["local"].as_bool(),
            Some(true),
            "capabilities/default.json must be explicitly local-only"
        );
    }

    #[test]
    fn capability_has_no_wildcards_and_is_window_scoped() {
        let cap = capability_json();

        for perm in capability_permissions(&cap) {
            assert!(
                !perm.contains('*'),
                "wildcard permission {perm} in capabilities/default.json"
            );
        }

        let windows: Vec<&str> = cap["windows"]
            .as_array()
            .expect("windows array")
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert_eq!(
            windows,
            vec!["main"],
            "capability must stay bound to the main window; got {windows:?}"
        );
    }

    #[test]
    fn shell_only_commands_stay_out_of_the_remote_grant() {
        for cmd in SHELL_ONLY_COMMANDS {
            assert!(
                !REMOTE_SETTINGS_COMMANDS.contains(cmd),
                "{cmd} must not be reachable from a remote page"
            );
        }
    }

    #[test]
    fn runtime_grant_pattern_matches_one_origin_only() {
        let pattern: RemoteUrlPattern = origin_url_pattern("http://192.168.1.5:8080")
            .parse()
            .expect("runtime grant must produce a valid remote URL pattern");

        for allowed in [
            "http://192.168.1.5:8080/",
            "http://192.168.1.5:8080/settings",
            "http://192.168.1.5:8080/storages?tab=sync",
        ] {
            let url = url::Url::parse(allowed).unwrap();
            assert!(pattern.test(&url), "grant should match {allowed}");
        }

        for denied in [
            "http://192.168.1.5:9090/",  // other port
            "http://192.168.1.6:8080/",  // other host
            "https://192.168.1.5:8080/", // other scheme
            "http://evil.example.com/",
        ] {
            let url = url::Url::parse(denied).unwrap();
            assert!(
                !pattern.test(&url),
                "grant must not match {denied}; it is a different origin"
            );
        }
    }

    #[test]
    fn runtime_grant_trims_a_trailing_slash() {
        assert_eq!(
            origin_url_pattern("https://sarca.example.com/"),
            "https://sarca.example.com/*"
        );
        let pattern: RemoteUrlPattern = origin_url_pattern("https://sarca.example.com/")
            .parse()
            .unwrap();
        assert!(pattern.test(&url::Url::parse("https://sarca.example.com/app").unwrap()));
    }

    #[test]
    fn plain_star_host_pattern_rejects_ports_regression() {
        // Documents why the old wildcard was written `http://*:*/*`: plain
        // `http://*` never matched a LAN URL with an explicit port. Both are
        // gone now, and this keeps the reasoning from being reintroduced.
        let broken: RemoteUrlPattern = "http://*".parse().unwrap();
        let lan = url::Url::parse("http://192.168.1.5:8080/").unwrap();
        assert!(
            !broken.test(&lan),
            "precondition failed: http://* unexpectedly matched a :port URL"
        );
        let wildcard: RemoteUrlPattern = "http://*:*/*".parse().unwrap();
        assert!(
            wildcard.test(&url::Url::parse("http://evil.example.com:80/x").unwrap()),
            "precondition failed: the removed wildcard was indeed origin-blind"
        );
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
        assert_eq!(
            allow_perm_for_command("verify_app_lock_pin"),
            "allow-verify-app-lock-pin"
        );
        assert!(
            REQUIRED_ALLOW_PERMISSIONS.contains(&"allow-connect"),
            "connect must remain in the required allow list"
        );
    }

    #[test]
    fn runtime_grant_uses_the_same_identifiers_as_the_capability_file() {
        let cap = capability_json();
        let perms = capability_permissions(&cap);
        for cmd in REMOTE_SETTINGS_COMMANDS {
            let id = permission_for(cmd);
            assert_eq!(id, allow_perm_for_command(cmd));
            assert!(
                perms.contains(&id.as_str()),
                "runtime grant asks for {id}, which no permission defines"
            );
        }
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
        for cmd in SHELL_ONLY_COMMANDS {
            assert!(
                build.contains(&format!("\"{cmd}\"")),
                "build.rs AppManifest missing shell command {cmd}"
            );
        }
    }
}
